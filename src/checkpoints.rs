//! Explicit single-file text checkpoints, with durable recovery copies before restoration.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_RECORDS: usize = 200;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub workspace: PathBuf,
    pub relative: PathBuf,
    pub title: String,
    pub at: u64,
    pub text: String,
}

#[derive(Clone)]
pub struct RestorePreview {
    pub checkpoint: Checkpoint,
    pub current: Option<String>,
}

fn error(key: &'static str) -> String {
    conduit::i18n::text(key)
}

pub fn directory() -> PathBuf {
    crate::store::Store::path()
        .parent()
        .unwrap()
        .join("file-checkpoints")
}

/// Reject absolute paths, traversal, metadata and all reparse points (including junctions).
pub fn checked_path(workspace: &Path, relative: &Path) -> Result<PathBuf> {
    ensure!(!relative.as_os_str().is_empty(), error("checkpoint.path"));
    let root = fs::canonicalize(workspace)?;
    let mut full = root.clone();
    for part in relative.components() {
        let Component::Normal(name) = part else {
            bail!(error("checkpoint.path"));
        };
        let name_text = name.to_string_lossy();
        ensure!(
            !name_text.eq_ignore_ascii_case(".git")
                && !name_text.contains(':')
                && !name_text.ends_with([' ', '.'])
                && !name_text.chars().any(char::is_control),
            error("checkpoint.path")
        );
        full.push(name);
        match fs::symlink_metadata(&full) {
            Ok(metadata) => ensure!(
                metadata.file_attributes() & 0x400 == 0,
                error("checkpoint.path")
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    ensure!(full.starts_with(root), error("checkpoint.path"));
    Ok(full)
}

fn read_file(file: &mut File) -> Result<String> {
    ensure!(
        file.metadata()?.file_attributes() & 0x400 == 0,
        error("checkpoint.path")
    );
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= MAX_BYTES,
        error("checkpoint.text_only")
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES && !bytes.contains(&0),
        error("checkpoint.text_only")
    );
    String::from_utf8(bytes).context(error("checkpoint.text_only"))
}

fn current(path: &Path) -> Result<Option<String>> {
    match OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)
    {
        Ok(mut file) => read_file(&mut file).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn save(directory: &Path, mut checkpoint: Checkpoint) -> Result<Checkpoint> {
    fs::create_dir_all(directory)?;
    ensure!(
        fs::read_dir(directory)?.take(MAX_RECORDS).count() < MAX_RECORDS,
        error("checkpoint.limit")
    );
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    checkpoint.id = format!(
        "{}-{}-{}",
        now.as_nanos(),
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    checkpoint.at = now.as_secs();
    let file_path = directory.join(format!("{}.json", checkpoint.id));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file_path)?;
    let result = (|| {
        serde_json::to_writer(&mut file, &checkpoint)?;
        file.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(file_path);
        return Err(error);
    }
    Ok(checkpoint)
}

pub fn capture(
    directory: &Path,
    workspace: &Path,
    relative: &Path,
    title: &str,
) -> Result<Checkpoint> {
    ensure!(
        !title.trim().is_empty() && title.len() <= 256,
        error("checkpoint.name")
    );
    let path = checked_path(workspace, relative)?;
    let text = current(&path)?.context(error("checkpoint.missing"))?;
    save(
        directory,
        Checkpoint {
            id: String::new(),
            workspace: fs::canonicalize(workspace)?,
            relative: relative.into(),
            title: title.trim().into(),
            at: 0,
            text,
        },
    )
}

pub fn list(directory: &Path, workspace: &Path) -> Result<Vec<Checkpoint>> {
    let workspace = fs::canonicalize(workspace)?;
    if !directory.exists() {
        return Ok(vec![]);
    }
    let mut records = vec![];
    for entry in fs::read_dir(directory)?.take(MAX_RECORDS) {
        let entry = entry?;
        if !entry.file_type()?.is_file() || entry.metadata()?.len() > MAX_BYTES * 6 + 8192 {
            continue;
        }
        let record: Checkpoint = serde_json::from_reader(File::open(entry.path())?)?;
        if record.workspace == workspace {
            ensure!(
                record.text.len() as u64 <= MAX_BYTES,
                error("checkpoint.text_only")
            );
            checked_path(&workspace, &record.relative)?;
            records.push(record);
        }
    }
    records.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| b.id.cmp(&a.id)));
    Ok(records)
}

pub fn preview(checkpoint: Checkpoint) -> Result<RestorePreview> {
    let path = checked_path(&checkpoint.workspace, &checkpoint.relative)?;
    let current = current(&path)?;
    Ok(RestorePreview {
        checkpoint,
        current,
    })
}

/// The exclusive writer handle rejects concurrent writers/deletion. Recheck the exact
/// reviewed bytes under this lock, then save a durable recovery copy before any write.
pub fn restore(directory: &Path, preview: &RestorePreview) -> Result<Option<Checkpoint>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let checkpoint = &preview.checkpoint;
    let path = checked_path(&checkpoint.workspace, &checkpoint.relative)?;
    ensure!(
        checkpoint.text.len() as u64 <= MAX_BYTES,
        error("checkpoint.text_only")
    );
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .create_new(preview.current.is_none())
        .open(&path)?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // Inspect the opened file, not a second path lookup. Writing a hard-linked file would
    // also change an unselected path, potentially outside the selected workspace.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    ensure!(
        information.nNumberOfLinks == 1,
        error("checkpoint.hardlink")
    );
    let actual = read_file(&mut file)?;
    if let Some(expected) = &preview.current {
        ensure!(&actual == expected, error("checkpoint.changed"));
    }
    let recovery = if preview.current.is_some() {
        let mut backup = checkpoint.clone();
        backup.title = error("checkpoint.recovery");
        backup.text = actual.clone();
        Some(save(directory, backup)?)
    } else {
        None
    };
    let result = (|| -> Result<()> {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(checkpoint.text.as_bytes())?;
        file.set_len(checkpoint.text.len() as u64)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let rollback = (|| -> Result<()> {
            file.seek(SeekFrom::Start(0))?;
            file.write_all(actual.as_bytes())?;
            file.set_len(actual.len() as u64)?;
            file.sync_all()?;
            Ok(())
        })();
        bail!(
            "{error:#}; recovery: {}; rollback: {:?}",
            recovery.as_ref().map_or("none", |r| r.id.as_str()),
            rollback.err()
        );
    }
    Ok(recovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_rejects_drift_and_retains_recovery_without_touching_neighbors() {
        let base = std::env::temp_dir().join(format!(
            "conduit-checkpoint-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("repo");
        let data = base.join("records");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), "old\r\n中文").unwrap();
        fs::write(root.join("neighbor"), "leave").unwrap();
        let checkpoint = capture(&data, &root, Path::new("a.txt"), "baseline").unwrap();
        fs::write(root.join("a.txt"), "new").unwrap();
        let plan = preview(checkpoint.clone()).unwrap();
        fs::write(root.join("a.txt"), "external edit").unwrap();
        assert!(restore(&data, &plan).is_err());
        let plan = preview(checkpoint).unwrap();
        let recovery = restore(&data, &plan).unwrap().unwrap();
        assert_eq!(recovery.text, "external edit");
        assert_eq!(
            fs::read_to_string(root.join("a.txt")).unwrap(),
            "old\r\n中文"
        );
        assert_eq!(fs::read_to_string(root.join("neighbor")).unwrap(), "leave");
        assert!(checked_path(&root, Path::new("../outside")).is_err());
        assert!(checked_path(&root, Path::new(".git/config")).is_err());
        assert_eq!(list(&data, &root).unwrap().len(), 2);
        let plan = preview(recovery).unwrap();
        fs::hard_link(root.join("a.txt"), base.join("outside-alias.txt")).unwrap();
        assert!(restore(&data, &plan).is_err());
        assert_eq!(
            fs::read_to_string(base.join("outside-alias.txt")).unwrap(),
            "old\r\n中文"
        );
        assert_eq!(list(&data, &root).unwrap().len(), 2);
        fs::remove_dir_all(base).unwrap();
    }
}
