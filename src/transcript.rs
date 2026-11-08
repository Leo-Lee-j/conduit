//! Immutable transcript files. The manifest contains summaries and content hashes only.
//! A save receipt is valid only for the exact copy-on-write message objects it saved.
use crate::domain::{ChatItem, ChatItemData, ItemKind, SessionRecord};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
mod chunks;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Reference {
    pub hash: String,
    pub message_count: usize,
    pub last_activity_at: u64,
    pub has_user: bool,
    pub bytes: usize,
    pub attachments: Vec<PathBuf>,
    #[serde(skip)]
    pub directory: PathBuf,
    #[serde(skip)]
    lease: Option<Arc<Lease>>,
}

// Every reader, pending import, manifest, and durable receipt pins its blob.
// Collection is serialized with lease acquisition, including newly published blobs.
#[derive(Debug, PartialEq, Eq)]
struct Lease;
fn leases() -> &'static Mutex<HashMap<PathBuf, Weak<Lease>>> {
    static LEASES: OnceLock<Mutex<HashMap<PathBuf, Weak<Lease>>>> = OnceLock::new();
    LEASES.get_or_init(Mutex::default)
}
fn acquire(path: PathBuf) -> Arc<Lease> {
    let mut leases = leases().lock().unwrap();
    if let Some(lease) = leases.get(&path).and_then(Weak::upgrade) {
        return lease;
    }
    let lease = Arc::new(Lease);
    leases.insert(path, Arc::downgrade(&lease));
    lease
}

fn manifests() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static PATHS: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    PATHS.get_or_init(Mutex::default)
}

pub(crate) fn register_manifest(path: &Path) {
    manifests()
        .lock()
        .unwrap()
        .insert(directory(path), path.to_owned());
}

fn committed_roots(directory: &Path) -> Result<Vec<PathBuf>> {
    #[derive(Deserialize)]
    struct Manifest {
        workspaces: Vec<WorkspaceRoots>,
    }
    #[derive(Deserialize)]
    struct WorkspaceRoots {
        sessions: Vec<SessionRoot>,
    }
    #[derive(Deserialize)]
    struct SessionRoot {
        #[serde(default)]
        transcript: Root,
    }
    #[derive(Default, Deserialize)]
    struct Root {
        reference: Option<Reference>,
    }
    let path = manifests().lock().unwrap().get(directory).cloned();
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    manifest
        .workspaces
        .into_iter()
        .flat_map(|w| w.sessions)
        .filter_map(|s| s.transcript.reference)
        .map(|r| {
            validate_hash(&r.hash)?;
            Ok(directory.join(format!("{}.json", r.hash)))
        })
        .collect()
}

pub(crate) fn collect_periodically(directory: &Path) {
    static LAST: OnceLock<Mutex<HashMap<PathBuf, std::time::Instant>>> = OnceLock::new();
    let now = std::time::Instant::now();
    let mut last = LAST.get_or_init(Mutex::default).lock().unwrap();
    if last
        .get(directory)
        .is_some_and(|time| time.elapsed() < std::time::Duration::from_secs(30))
    {
        return;
    }
    last.insert(directory.to_owned(), now);
    drop(last);
    collect_unreferenced(directory);
}

/// Best-effort cleanup after committing the manifest. Live snapshots keep old
/// versions readable; failed saves and staged imports cannot lose their bodies.
pub(crate) fn collect_unreferenced(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut leases = leases().lock().unwrap();
    let Ok(committed) = committed_roots(directory) else {
        return;
    };
    let roots = leases
        .iter()
        .filter(|(path, lease)| path.parent() == Some(directory) && lease.strong_count() > 0)
        .map(|(path, _)| path.clone());
    let Ok(reachable) = chunks::reachable(directory, roots.chain(committed)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let owned = path
            .extension()
            .is_some_and(|extension| extension == "json" || extension == "chunk")
            && path
                .file_stem()
                .and_then(|name| name.to_str())
                .is_some_and(|name| validate_hash(name).is_ok());
        if owned && !reachable.contains(&path) && fs::remove_file(&path).is_ok() {
            leases.remove(&path);
        }
    }
}

#[derive(Clone, Debug)]
struct Receipt {
    reference: Reference,
    messages: Vec<Weak<ChatItemData>>,
    parts: Vec<Arc<chunks::Message>>,
    pages: Vec<String>,
}
impl Receipt {
    fn matches(&self, messages: &[ChatItem]) -> bool {
        self.messages.len() == messages.len()
            && self
                .messages
                .iter()
                .zip(messages)
                .all(|(saved, message)| saved.ptr_eq(&message.identity()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Transcript {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<Reference>,
    #[serde(skip, default = "loaded")]
    pub loaded: bool,
    #[serde(skip)]
    receipt: Arc<Mutex<Option<Receipt>>>,
}
fn loaded() -> bool {
    true
}
impl Default for Transcript {
    fn default() -> Self {
        Self {
            reference: None,
            loaded: true,
            receipt: Arc::default(),
        }
    }
}

impl Transcript {
    pub fn bind(&mut self, directory: &Path) -> Result<()> {
        if let Some(reference) = &mut self.reference {
            validate_hash(&reference.hash)?;
            if reference.message_count == 0 {
                bail!(conduit::i18n::text(
                    "transcript.session_summary_is_missing_message_count"
                ));
            }
            reference.directory = directory.to_owned();
            reference.lease = Some(acquire(path(reference)?));
            self.loaded = false;
        }
        Ok(())
    }

    /// Does not perform I/O. A concurrent writer can only acknowledge its own snapshot.
    pub fn saved(&self, messages: &[ChatItem]) -> Option<Reference> {
        if !self.loaded {
            return self.reference.clone();
        }
        self.receipt
            .lock()
            .unwrap()
            .as_ref()
            .filter(|receipt| receipt.matches(messages))
            .map(|receipt| receipt.reference.clone())
    }

    fn acknowledge(
        &self,
        reference: Reference,
        messages: &[ChatItem],
        parts: Vec<Arc<chunks::Message>>,
        pages: Vec<String>,
    ) {
        *self.receipt.lock().unwrap() = Some(Receipt {
            reference,
            messages: messages.iter().map(ChatItem::identity).collect(),
            parts,
            pages,
        });
    }
}

pub(crate) fn directory(state: &Path) -> PathBuf {
    state.with_extension("transcripts")
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!(conduit::i18n::text(
            "transcript.invalid_session_message_reference"
        ));
    }
    Ok(())
}

fn path(reference: &Reference) -> Result<PathBuf> {
    validate_hash(&reference.hash)?;
    if reference.directory.as_os_str().is_empty() {
        bail!(conduit::i18n::text(
            "transcript.session_message_storage_directory_missing"
        ));
    }
    Ok(reference.directory.join(format!("{}.json", reference.hash)))
}

pub(crate) fn attachment_paths(messages: &[ChatItem]) -> Vec<PathBuf> {
    let mut paths: Vec<_> = messages
        .iter()
        .filter(|message| message.kind == ItemKind::User)
        .flat_map(|message| crate::file_ref::split_prompt(&message.text).1)
        .map(PathBuf::from)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn resident_bytes(messages: &[ChatItem]) -> usize {
    messages
        .iter()
        .map(|message| {
            std::mem::size_of::<ChatItemData>()
                + message.key.len()
                + message.text.len()
                + message.input.len()
                + message.detail.len()
        })
        .sum()
}

/// Called only on the disk writer/background executor. Never rewrites a referenced blob.
pub(crate) fn save(session: &SessionRecord, directory: &Path) -> Result<Option<Reference>> {
    if !session.transcript.loaded && !session.messages.is_empty() {
        bail!(conduit::i18n::format(
            "transcript.session_messages_are_not_loaded_refusing_to",
            &[("arg0", format!("{}", session.id))]
        ));
    }
    if let Some(reference) = session.transcript.saved(&session.messages) {
        if reference.directory == directory {
            // Reuse immutable references without stat/read calls for every cold
            // session on every streaming save. Opening verifies the actual bytes.
            return Ok(Some(reference));
        }
        // Save-as and test stores must not leave a manifest pointing into another data directory.
        let mut restored = session.clone();
        restored.messages = read(session)?;
        restored.transcript = Transcript::default();
        return save(&restored, directory);
    }
    if !session.transcript.loaded {
        bail!(conduit::i18n::format(
            "transcript.session_is_missing_its_message_reference",
            &[("arg0", format!("{}", session.id))]
        ));
    }
    if session.messages.is_empty() {
        return Ok(None);
    }
    let previous = session.transcript.receipt.lock().unwrap().clone();
    let prepared = chunks::prepare(session, directory, previous.as_ref())?;
    session.transcript.acknowledge(
        prepared.reference.clone(),
        &session.messages,
        prepared.parts.clone(),
        prepared.pages.clone(),
    );
    Ok(Some(prepared.reference.clone()))
}

/// Durable immutable publication shared by roots, pages, and raw text blocks.
fn publish(bytes: &[u8], hash: &str, destination: &Path) -> Result<()> {
    let directory = destination.parent().context(conduit::i18n::text_static(
        "transcript.session_message_storage_directory_missing",
    ))?;
    if destination.exists() {
        // An interrupted/externally damaged file is never mistaken for a durable receipt.
        if fs::read(destination)?.as_slice() != bytes {
            bail!(conduit::i18n::format(
                "transcript.chunks.session_message_validation_failed",
                &[("arg0", format!("{}", destination.display()))]
            ));
        }
    } else {
        // Import and persistence can save identical bodies concurrently. Each stages a unique file.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = directory.join(format!("{}.{}.{sequence}.tmp", hash, std::process::id()));
        let result = (|| -> Result<()> {
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            output.write_all(bytes)?;
            output.sync_all()?;
            drop(output);
            // Concurrent writers publish the same hash/bytes. Atomic replacement
            // also works on filesystems without hard-link support.
            crate::store::replace_file(&temporary, destination)?;
            Ok(())
        })();
        let _ = fs::remove_file(&temporary);
        result?;
        #[cfg(not(windows))]
        fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

/// Returns one session's body without altering the UI's residency or error state.
pub(crate) fn read(session: &SessionRecord) -> Result<Vec<ChatItem>> {
    if session.transcript.loaded {
        return Ok(session.messages.clone());
    }
    let reference = session
        .transcript
        .reference
        .as_ref()
        .context(conduit::i18n::text_static(
            "transcript.session_message_reference_missing",
        ))?;
    Ok(chunks::read(reference)?.messages)
}

/// Load and recover interrupted turns on a worker. The caller still checks identity before applying.
pub(crate) fn load(mut session: SessionRecord) -> Result<SessionRecord> {
    if !session.transcript.loaded {
        let reference =
            session
                .transcript
                .reference
                .clone()
                .context(conduit::i18n::text_static(
                    "transcript.session_message_reference_missing",
                ))?;
        let loaded = chunks::read(&reference)?;
        session.messages = loaded.messages;
        session
            .transcript
            .acknowledge(reference, &session.messages, loaded.parts, loaded.pages);
        session.transcript.loaded = true;
        if session.interrupted {
            crate::store::restore_session(&mut session);
        }
    }
    Ok(session)
}

/// Only acknowledged, unchanged bodies can leave memory. Metadata is retained.
pub(crate) fn unload(session: &mut SessionRecord) -> bool {
    if !session.transcript.loaded || session.messages.is_empty() {
        return false;
    }
    let Some(reference) = session.transcript.saved(&session.messages) else {
        return false;
    };
    session.transcript = Transcript {
        reference: Some(reference),
        loaded: false,
        ..Default::default()
    };
    session.messages.clear();
    session.messages.shrink_to_fit();
    true
}

#[cfg(test)]
pub(crate) mod tests;
