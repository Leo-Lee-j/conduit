use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::io::Write;

// Reparse cached Claude histories to include CLI-emitted thinking blocks.
const PARSER_VERSION: u32 = 2;
const MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;
const MAX_INDEX_BYTES: i64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Stamp {
    pub len: u64,
    pub modified: Option<u128>,
    pub created: Option<u128>,
}
impl Stamp {
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
        let nanos = |time: std::io::Result<std::time::SystemTime>| {
            time.ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|time| time.as_nanos())
        };
        Ok(Self {
            len: metadata.len(),
            modified: nanos(metadata.modified()),
            created: nanos(metadata.created()),
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub version: u32,
    pub provider: Provider,
    pub stamp: Stamp,
    pub offset: u64,
    pub cwds: Vec<String>,
    pub digest: Vec<u8>,
    pub terminated: bool,
}
impl Metadata {
    pub fn new(
        provider: Provider,
        stamp: Stamp,
        offset: u64,
        cwds: Vec<String>,
        digest: Vec<u8>,
        terminated: bool,
    ) -> Self {
        Self {
            version: PARSER_VERSION,
            provider,
            stamp,
            offset,
            cwds,
            digest,
            terminated,
        }
    }
    pub fn compatible(&self, provider: Provider) -> bool {
        self.version == PARSER_VERSION && self.provider == provider && self.offset <= self.stamp.len
    }
    pub fn may_append(&self, stamp: &Stamp) -> bool {
        stamp.len > self.stamp.len && stamp.created == self.stamp.created && self.terminated
    }
}

struct LimitedBuffer(Vec<u8>);
impl Write for LimitedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_ENTRY_BYTES {
            return Err(std::io::Error::other("history index entry budget exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) struct Index(Connection);
impl Index {
    pub fn open(path: &Path) -> Result<Self, String> {
        let open = || -> anyhow::Result<Self> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let connection = Connection::open(path)?;
            connection.busy_timeout(std::time::Duration::from_secs(2))?;
            connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA cache_size=-8192; PRAGMA mmap_size=0;
                CREATE TABLE IF NOT EXISTS history_files (path TEXT PRIMARY KEY, metadata TEXT NOT NULL, state BLOB NOT NULL, touched INTEGER NOT NULL);")?;
            Ok(Self(connection))
        };
        open().map_err(|error| error.to_string())
    }
    pub fn metadata(&self, path: &Path) -> Option<Metadata> {
        let value: Option<String> = self
            .0
            .query_row(
                "SELECT metadata FROM history_files WHERE path=?1",
                [path.to_string_lossy().as_ref()],
                |row| row.get(0),
            )
            .optional()
            .ok()?;
        serde_json::from_str(&value?).ok()
    }
    pub fn state(&self, path: &Path, metadata: &Metadata) -> Option<scan::Parser> {
        let value: Vec<u8> = self
            .0
            .query_row(
                "SELECT state FROM history_files WHERE path=?1 AND length(state)<=?2 AND metadata=?3",
                params![
                    path.to_string_lossy().as_ref(),
                    MAX_ENTRY_BYTES as i64,
                    serde_json::to_string(metadata).ok()?
                ],
                |row| row.get(0),
            )
            .ok()?;
        serde_json::from_slice(&value).ok()
    }
    pub fn save(
        &mut self,
        path: &Path,
        metadata: &Metadata,
        state: &scan::Parser,
    ) -> Result<(), String> {
        let mut save = || -> anyhow::Result<()> {
            let mut buffer = LimitedBuffer(Vec::new());
            if serde_json::to_writer(&mut buffer, state).is_err() {
                buffer.0.clear();
            }
            let bytes = buffer.0;
            let transaction = self.0.transaction()?;
            let touched = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as i64;
            transaction.execute(
                "INSERT OR REPLACE INTO history_files VALUES (?1,?2,?3,?4)",
                params![
                    path.to_string_lossy().as_ref(),
                    serde_json::to_string(metadata)?,
                    bytes,
                    touched
                ],
            )?;
            let mut total: i64 = transaction.query_row(
                "SELECT coalesce(sum(length(state)),0) FROM history_files",
                [],
                |row| row.get(0),
            )?;
            while total > MAX_INDEX_BYTES {
                let (oldest, size): (String, i64) = transaction.query_row(
                    "SELECT path,length(state) FROM history_files WHERE length(state)>0 ORDER BY touched,path LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                transaction
                    .execute("UPDATE history_files SET state=x'' WHERE path=?1", [oldest])?;
                total = total.saturating_sub(size);
            }
            transaction.commit()?;
            Ok(())
        };
        save().map_err(|error| error.to_string())
    }
}
