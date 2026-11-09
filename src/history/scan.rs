use super::index::{Index, Metadata, Stamp};
use super::*;
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    sync::atomic::{AtomicBool, Ordering},
};
const MAX_LINE: usize = 8 * 1024 * 1024;
const BATCH_BYTES: u64 = 16 * 1024 * 1024;
const BATCH_FILES: usize = 16;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum Parser {
    Claude(Box<ClaudeState>),
    Codex(Box<CodexState>),
}
impl Parser {
    fn new(provider: Provider) -> Self {
        match provider {
            Provider::Claude => Self::Claude(Box::default()),
            _ => Self::Codex(Box::default()),
        }
    }
    fn extend(self, lines: impl Iterator<Item = String>) -> Self {
        match self {
            Self::Claude(state) => Self::Claude(Box::new(extend_claude(lines, *state))),
            Self::Codex(state) => Self::Codex(Box::new(extend_codex(lines, None, *state))),
        }
    }
    fn cwds(&self) -> Vec<String> {
        match self {
            Self::Claude(state) => state.cwds.clone(),
            Self::Codex(state) => vec![state.cwd.clone()],
        }
    }
    fn finish(self) -> Option<(Vec<String>, ImportedSession)> {
        match self {
            Self::Claude(state) => state.finish(),
            Self::Codex(state) => state.finish().map(|(cwd, session)| (vec![cwd], session)),
        }
    }
}

pub(crate) struct Batch {
    pub sessions: Vec<Vec<ImportedSession>>,
    pub warnings: Vec<String>,
    pub completed: usize,
    pub total: usize,
    pub done: bool,
    pub bytes_read: u64,
    pub verified_bytes: u64,
    pub cache_hits: usize,
}

struct Lines {
    reader: BufReader<std::io::Take<File>>,
    offset: u64,
    read: u64,
    error: Option<String>,
    digest: Sha256,
    terminated: bool,
    finished: bool,
}
impl Iterator for Lines {
    type Item = String;
    fn next(&mut self) -> Option<String> {
        if self.error.is_some() {
            return None;
        }
        let mut bytes = Vec::new();
        match self
            .reader
            .by_ref()
            .take((MAX_LINE + 1) as u64)
            .read_until(b'\n', &mut bytes)
        {
            Ok(0) => {
                self.finished = true;
                None
            }
            Ok(length) => {
                self.read += length as u64;
                if length > MAX_LINE {
                    self.error = Some(
                        conduit::i18n::text_static(
                            "history.scan.a_history_line_exceeds_8_mib_existing",
                        )
                        .into(),
                    );
                    return None;
                }
                self.terminated = bytes.last() == Some(&b'\n');
                if !self.terminated && serde_json::from_slice::<Value>(&bytes).is_err() {
                    self.terminated = true;
                    self.finished = true;
                    return None;
                }
                self.digest.update(&bytes);
                self.offset += length as u64;
                match String::from_utf8(bytes) {
                    Ok(line) => Some(line),
                    Err(_) => {
                        self.error = Some(
                            conduit::i18n::text_static("history.scan.history_file_is_not_utf_8")
                                .into(),
                        );
                        None
                    }
                }
            }
            Err(error) => {
                self.error = Some(conduit::i18n::format(
                    "history.scan.unable_to_read_history",
                    &[("error", format!("{}", error))],
                ));
                None
            }
        }
    }
}

struct Candidate {
    path: PathBuf,
    provider: Provider,
    stamp: Stamp,
    indexed: Option<Metadata>,
    retries: usize,
}

enum Pending {
    Verify {
        stamp: Stamp,
        metadata: Metadata,
        parser: Parser,
        reader: std::io::Take<File>,
        digest: Sha256,
    },
    Parse {
        stamp: Stamp,
        parser: Parser,
        lines: Lines,
    },
}

enum Loaded {
    Pending,
    Changed,
    Complete(Option<(Vec<String>, ImportedSession)>),
}

pub(super) fn batch_exhausted(batch: &Batch, started: std::time::Instant) -> bool {
    batch.bytes_read.saturating_add(batch.verified_bytes) >= BATCH_BYTES
        || started.elapsed() >= std::time::Duration::from_millis(100)
}

pub(crate) struct Scanner {
    index: Option<Index>,
    workspaces: Vec<PathBuf>,
    queue: VecDeque<Candidate>,
    warnings: Vec<String>,
    total: usize,
    completed: usize,
    pending: Option<Pending>,
    opencode: Option<super::opencode::Scanner>,
}
impl Scanner {
    pub(crate) fn new(workspaces: Vec<PathBuf>, cancel: &AtomicBool) -> Self {
        let home = home();
        let opencode_path = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".local/share")))
            .map(|root| root.join("opencode/opencode.db"));
        Self::open(
            home.as_deref(),
            workspaces,
            &crate::store::Store::path().with_file_name("history-index-v1.sqlite3"),
            opencode_path.as_deref(),
            cancel,
        )
    }
    pub(super) fn open(
        home: Option<&Path>,
        workspaces: Vec<PathBuf>,
        index_path: &Path,
        opencode_path: Option<&Path>,
        cancel: &AtomicBool,
    ) -> Self {
        let mut warnings = Vec::new();
        let index = match Index::open(index_path) {
            Ok(index) => Some(index),
            Err(error) => {
                warnings.push(conduit::i18n::format(
                    "history.scan.history_index_unavailable_reading_logs_directly",
                    &[("error", error.to_string())],
                ));
                None
            }
        };
        let mut candidates = Vec::new();
        if let Some(home) = home.filter(|_| !workspaces.is_empty()) {
            for provider in [Provider::Claude, Provider::Codex] {
                let root = home.join(match provider {
                    Provider::Claude => ".claude/projects",
                    _ => ".codex/sessions",
                });
                let mut directories = vec![root];
                while let Some(directory) = directories.pop() {
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let entries = match std::fs::read_dir(&directory) {
                        Ok(entries) => entries,
                        Err(error) => {
                            if error.kind() != std::io::ErrorKind::NotFound && warnings.len() < 8 {
                                warnings.push(format!("{}：{error}", directory.display()));
                            }
                            continue;
                        }
                    };
                    for entry in entries.flatten() {
                        if cancel.load(Ordering::Acquire) {
                            break;
                        }
                        let path = entry.path();
                        let Ok(kind) = entry.file_type() else {
                            continue;
                        };
                        if kind.is_dir() {
                            directories.push(path);
                            continue;
                        }
                        if !kind.is_file()
                            || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
                        {
                            continue;
                        }
                        let Ok(stamp) = Stamp::read(&path) else {
                            continue;
                        };
                        let indexed = index
                            .as_ref()
                            .and_then(|index| index.metadata(&path))
                            .filter(|meta| meta.compatible(provider));
                        if let Some(meta) = &indexed
                            && meta.stamp == stamp
                            && !meta.cwds.is_empty()
                            && !matches_workspace(&meta.cwds, &workspaces)
                        {
                            continue;
                        }
                        candidates.push(Candidate {
                            path,
                            provider,
                            stamp,
                            indexed,
                            retries: 0,
                        });
                    }
                }
            }
        }
        candidates.sort_by(|left, right| {
            let priority = |candidate: &Candidate| {
                candidate
                    .indexed
                    .as_ref()
                    .is_some_and(|meta| matches_workspace(&meta.cwds, &workspaces))
            };
            priority(right)
                .cmp(&priority(left))
                .then_with(|| right.stamp.modified.cmp(&left.stamp.modified))
                .then_with(|| left.path.cmp(&right.path))
        });
        let opencode = opencode_path.and_then(|path| {
            match super::opencode::Scanner::open(path, &workspaces, cancel) {
                Ok(scanner) => scanner,
                Err(error) => {
                    warnings.push(format!("{}：{error}", path.display()));
                    None
                }
            }
        });
        let total = candidates.len() + opencode.as_ref().map_or(0, |scanner| scanner.remaining());
        Self {
            index,
            workspaces,
            queue: candidates.into(),
            warnings,
            total,
            completed: 0,
            pending: None,
            opencode,
        }
    }
    pub(crate) fn next(&mut self, cancel: &AtomicBool) -> Batch {
        let mut batch = Batch {
            sessions: vec![vec![]; self.workspaces.len()],
            warnings: std::mem::take(&mut self.warnings),
            completed: self.completed,
            total: self.total,
            done: false,
            bytes_read: 0,
            verified_bytes: 0,
            cache_hits: 0,
        };
        let started = std::time::Instant::now();
        let mut files = 0;
        while !cancel.load(Ordering::Acquire) {
            let Some(mut candidate) = self.queue.pop_front() else {
                break;
            };
            match self.load(&candidate, cancel, &mut batch, started) {
                Ok(Loaded::Pending) => {
                    self.queue.push_front(candidate);
                    break;
                }
                Ok(Loaded::Changed) if candidate.retries < 2 => {
                    candidate.retries += 1;
                    self.queue.push_back(candidate);
                    break;
                }
                Ok(Loaded::Changed) => {
                    if batch.warnings.len() < 8 {
                        batch.warnings.push(conduit::i18n::format(
                            "history.scan.log_keeps_changing_existing_conversation_preserved_the",
                            &[("arg0", format!("{}", candidate.path.display()))],
                        ));
                    }
                }
                Ok(Loaded::Complete(Some((cwds, session)))) => {
                    for (workspace, sessions) in self.workspaces.iter().zip(&mut batch.sessions) {
                        if cwds.iter().any(|cwd| same_dir(cwd, workspace)) {
                            sessions.push(session.clone());
                        }
                    }
                }
                Ok(Loaded::Complete(None)) => {}
                Err(error) => {
                    if batch.warnings.len() < 8 && !cancel.load(Ordering::Acquire) {
                        batch
                            .warnings
                            .push(format!("{}：{error}", candidate.path.display()));
                    }
                }
            }
            self.completed += 1;
            files += 1;
            if files >= BATCH_FILES || batch_exhausted(&batch, started) {
                break;
            }
        }
        batch.completed = self.completed;
        if self.queue.is_empty()
            && let Some(scanner) = &mut self.opencode
        {
            scanner.next(
                &self.workspaces,
                cancel,
                &mut batch,
                started,
                BATCH_FILES - files,
            );
            self.completed = batch.completed;
        }
        batch.done = (self.queue.is_empty()
            && self
                .opencode
                .as_ref()
                .is_none_or(|scanner| scanner.remaining() == 0))
            || cancel.load(Ordering::Acquire);
        for sessions in &mut batch.sessions {
            sessions.sort_by_key(|session| std::cmp::Reverse(session.created_at));
        }
        batch
    }
    fn load(
        &mut self,
        candidate: &Candidate,
        cancel: &AtomicBool,
        batch: &mut Batch,
        started: std::time::Instant,
    ) -> Result<Loaded, String> {
        let path = &candidate.path;
        let mut pending = match self.pending.take() {
            Some(pending) => pending,
            None => {
                let stamp = Stamp::read(path)?;
                let cached = candidate.indexed.as_ref().and_then(|metadata| {
                    if metadata.offset == 0 && stamp.len > 0 {
                        return None;
                    }
                    self.index
                        .as_ref()?
                        .state(path, metadata)
                        .map(|parser| (metadata, parser))
                });
                if let Some((metadata, parser)) = cached {
                    if metadata.stamp == stamp {
                        if Stamp::read(path)? != stamp {
                            return Ok(Loaded::Changed);
                        }
                        batch.cache_hits += 1;
                        return Ok(Loaded::Complete(parser.finish()));
                    }
                    if metadata.may_append(&stamp) {
                        Pending::Verify {
                            stamp,
                            metadata: metadata.clone(),
                            parser,
                            reader: File::open(path)
                                .map_err(|error| error.to_string())?
                                .take(metadata.offset),
                            digest: Sha256::new(),
                        }
                    } else {
                        parsing(
                            path,
                            stamp,
                            Parser::new(candidate.provider),
                            0,
                            Sha256::new(),
                        )?
                    }
                } else {
                    if candidate.provider == Provider::Codex
                        && self.foreign_codex(candidate, &stamp, batch)?
                    {
                        return Ok(Loaded::Complete(None));
                    }
                    parsing(
                        path,
                        stamp,
                        Parser::new(candidate.provider),
                        0,
                        Sha256::new(),
                    )?
                }
            }
        };
        if let Pending::Verify {
            stamp,
            metadata,
            parser,
            mut reader,
            mut digest,
        } = pending
        {
            let mut buffer = [0; 64 * 1024];
            while reader.limit() > 0 {
                if cancel.load(Ordering::Acquire) {
                    return Err(
                        conduit::i18n::text_static("history.scan.history_read_cancelled").into(),
                    );
                }
                if batch_exhausted(batch, started) {
                    self.pending = Some(Pending::Verify {
                        stamp,
                        metadata,
                        parser,
                        reader,
                        digest,
                    });
                    return Ok(Loaded::Pending);
                }
                let read = reader
                    .read(&mut buffer)
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    return Ok(Loaded::Changed);
                }
                digest.update(&buffer[..read]);
                batch.verified_bytes += read as u64;
            }
            if Stamp::read(path)? != stamp {
                return Ok(Loaded::Changed);
            }
            let actual: &[u8] = &digest.clone().finalize();
            pending = if actual == metadata.digest.as_slice() {
                parsing(path, stamp, parser, metadata.offset, digest)?
            } else {
                parsing(
                    path,
                    stamp,
                    Parser::new(candidate.provider),
                    0,
                    Sha256::new(),
                )?
            };
        }
        let Pending::Parse {
            stamp,
            parser,
            mut lines,
        } = pending
        else {
            unreachable!()
        };
        let previous_read = lines.read;
        let parser = parser.extend(std::iter::from_fn(|| {
            let read = batch.bytes_read + batch.verified_bytes + lines.read - previous_read;
            if cancel.load(Ordering::Acquire)
                || read >= BATCH_BYTES
                || started.elapsed() >= std::time::Duration::from_millis(100)
            {
                None
            } else {
                lines.next()
            }
        }));
        batch.bytes_read += lines.read - previous_read;
        if let Some(error) = lines.error {
            return Err(error);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(conduit::i18n::text_static("history.scan.history_read_cancelled").into());
        }
        if Stamp::read(path)? != stamp {
            return Ok(Loaded::Changed);
        }
        if !lines.finished {
            self.pending = Some(Pending::Parse {
                stamp,
                parser,
                lines,
            });
            return Ok(Loaded::Pending);
        }
        let metadata = Metadata::new(
            candidate.provider,
            stamp,
            lines.offset,
            parser.cwds(),
            lines.digest.finalize().to_vec(),
            lines.terminated,
        );
        if let Some(index) = &mut self.index
            && let Err(error) = index.save(path, &metadata, &parser)
            && batch.warnings.len() < 8
        {
            batch.warnings.push(conduit::i18n::format(
                "history.scan.failed_to_write_history_index",
                &[("error", error.to_string())],
            ));
        }
        Ok(Loaded::Complete(parser.finish()))
    }

    fn foreign_codex(
        &mut self,
        candidate: &Candidate,
        stamp: &Stamp,
        batch: &mut Batch,
    ) -> Result<bool, String> {
        let path = &candidate.path;
        let mut prefix = Vec::new();
        File::open(path)
            .map_err(|error| error.to_string())?
            .take(64 * 1024)
            .read_to_end(&mut prefix)
            .map_err(|error| error.to_string())?;
        batch.bytes_read += prefix.len() as u64;
        let cwd = String::from_utf8_lossy(&prefix)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|value| value.get("type").and_then(Value::as_str) == Some("session_meta"))
            .and_then(|value| {
                value
                    .get("payload")
                    .unwrap_or(&value)
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        if let Some(cwd) = cwd
            && !cwd.is_empty()
            && !matches_workspace(std::slice::from_ref(&cwd), &self.workspaces)
        {
            let metadata = Metadata::new(
                candidate.provider,
                stamp.clone(),
                0,
                vec![cwd],
                Sha256::digest([]).to_vec(),
                true,
            );
            if Stamp::read(path)? == *stamp
                && let Some(index) = &mut self.index
            {
                let _ = index.save(path, &metadata, &Parser::new(candidate.provider));
            }
            return Ok(true);
        }
        Ok(false)
    }
}

fn parsing(
    path: &Path,
    stamp: Stamp,
    parser: Parser,
    offset: u64,
    digest: Sha256,
) -> Result<Pending, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| error.to_string())?;
    let lines = Lines {
        reader: BufReader::new(file.take(stamp.len - offset)),
        offset,
        read: 0,
        error: None,
        digest,
        terminated: true,
        finished: false,
    };
    Ok(Pending::Parse {
        stamp,
        parser,
        lines,
    })
}

fn matches_workspace(cwds: &[String], workspaces: &[PathBuf]) -> bool {
    cwds.iter()
        .any(|cwd| workspaces.iter().any(|workspace| same_dir(cwd, workspace)))
}

pub(crate) fn run(workspaces: &[PathBuf], cancel: &AtomicBool) -> Batch {
    let mut scanner = Scanner::new(workspaces.to_vec(), cancel);
    let mut result = scanner.next(cancel);
    while !result.done {
        let next = scanner.next(cancel);
        for (sessions, incoming) in result.sessions.iter_mut().zip(next.sessions) {
            sessions.extend(incoming);
        }
        result.warnings.extend(next.warnings);
        result.completed = next.completed;
        result.done = next.done;
        result.bytes_read += next.bytes_read;
        result.verified_bytes += next.verified_bytes;
        result.cache_hits += next.cache_hits;
    }
    for sessions in &mut result.sessions {
        sessions.sort_by_key(|session| std::cmp::Reverse(session.created_at));
    }
    result
}
