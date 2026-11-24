//! Read OpenCode's SQLite history without starting the CLI or changing its data.
use super::{ImportedSession, same_dir, truncate_title};
use crate::domain::{ChatItem, ChatItemData, ItemKind, Provider};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

struct Session {
    cwd: String,
    record: ImportedSession,
}

struct Pending {
    session: Session,
    cursor: (i64, String, String),
    message_id: String,
    message: Value,
}

pub(super) struct Scanner {
    connection: Connection,
    path: PathBuf,
    sessions: VecDeque<Session>,
    pending: Option<Pending>,
}

impl Scanner {
    pub(super) fn open(
        path: &Path,
        workspaces: &[PathBuf],
        cancel: &AtomicBool,
    ) -> anyhow::Result<Option<Self>> {
        if workspaces.is_empty() || !path.try_exists()? || cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(100))?;
        let mut sessions = VecDeque::new();
        {
            let mut statement = connection.prepare(
                "SELECT id, directory, title, time_created FROM session
                 WHERE parent_id IS NULL AND time_archived IS NULL
                 ORDER BY time_updated DESC, id DESC",
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                if cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
                let cwd: String = row.get(1)?;
                if !workspaces.iter().any(|workspace| same_dir(&cwd, workspace)) {
                    continue;
                }
                let created_at: i64 = row.get(3)?;
                sessions.push_back(Session {
                    cwd,
                    record: ImportedSession {
                        is_subagent: false,
                        provider: Provider::OpenCode,
                        native_id: row.get(0)?,
                        title: row.get(2)?,
                        created_at: created_at.max(0) as u64 / 1000,
                        model: None,
                        subagents: Vec::new(),
                        messages: Vec::new(),
                    },
                });
            }
        }
        Ok(Some(Self {
            connection,
            path: path.to_owned(),
            sessions,
            pending: None,
        }))
    }

    pub(super) fn remaining(&self) -> usize {
        self.sessions.len() + usize::from(self.pending.is_some())
    }

    pub(super) fn next(
        &mut self,
        workspaces: &[PathBuf],
        cancel: &AtomicBool,
        batch: &mut super::scan::Batch,
        started: Instant,
        limit: usize,
    ) {
        let initial_completed = batch.completed;
        while !cancel.load(Ordering::Acquire)
            && batch.completed - initial_completed < limit
            && !super::scan::batch_exhausted(batch, started)
        {
            let mut pending = match self.pending.take() {
                Some(pending) => pending,
                None => {
                    let Some(session) = self.sessions.pop_front() else {
                        break;
                    };
                    // Keep a consistent snapshot across batches of a large session.
                    if let Err(error) = self.connection.execute_batch("BEGIN") {
                        self.warn(batch, &session.record.native_id, error);
                        batch.completed += 1;
                        continue;
                    }
                    Pending {
                        session,
                        cursor: (-1, String::new(), String::new()),
                        message_id: String::new(),
                        message: Value::Null,
                    }
                }
            };
            match read_parts(&self.connection, &mut pending, cancel, batch, started) {
                Ok(false) => {
                    self.pending = Some(pending);
                    break;
                }
                Ok(true) => {
                    let record = pending.session.record;
                    if record
                        .messages
                        .iter()
                        .any(|item| item.kind == ItemKind::User)
                    {
                        for (workspace, sessions) in workspaces.iter().zip(&mut batch.sessions) {
                            if same_dir(&pending.session.cwd, workspace) {
                                sessions.push(record.clone());
                            }
                        }
                    }
                }
                Err(error) => self.warn(batch, &pending.session.record.native_id, error),
            }
            let _ = self.connection.execute_batch("ROLLBACK");
            batch.completed += 1;
        }
        if cancel.load(Ordering::Acquire) {
            self.pending = None;
            self.sessions.clear();
            let _ = self.connection.execute_batch("ROLLBACK");
        }
    }

    fn warn(&self, batch: &mut super::scan::Batch, id: &str, error: impl std::fmt::Display) {
        if batch.warnings.len() < 8 {
            batch
                .warnings
                .push(format!("{} [{id}]：{error}", self.path.display()));
        }
    }
}

fn read_parts(
    connection: &Connection,
    pending: &mut Pending,
    cancel: &AtomicBool,
    batch: &mut super::scan::Batch,
    started: Instant,
) -> anyhow::Result<bool> {
    let mut statement = connection.prepare_cached(
        "SELECT m.time_created, m.id, m.data, COALESCE(p.id, ''), p.data, p.time_created
         FROM message m LEFT JOIN part p ON p.message_id = m.id AND p.session_id = m.session_id
         WHERE m.session_id = ?1 AND (m.time_created, m.id, COALESCE(p.id, '')) > (?2, ?3, ?4)
         ORDER BY m.time_created, m.id, p.id LIMIT 128",
    )?;
    let mut rows = statement.query(params![
        pending.session.record.native_id,
        pending.cursor.0,
        pending.cursor.1,
        pending.cursor.2,
    ])?;
    let mut read = 0;
    while !cancel.load(Ordering::Acquire) && !super::scan::batch_exhausted(batch, started) {
        let Some(row) = rows.next()? else {
            return Ok(read < 128);
        };
        let created: i64 = row.get(0)?;
        let message_id: String = row.get(1)?;
        let part_id: String = row.get(3)?;
        if pending.message_id != message_id {
            let data: String = row.get(2)?;
            batch.add_opencode_bytes(data.len() as u64);
            pending.message = serde_json::from_str(&data)?;
            pending.message_id = message_id.clone();
            let model = pending.message.get("model").unwrap_or(&pending.message);
            if let (Some(provider), Some(model)) =
                (model["providerID"].as_str(), model["modelID"].as_str())
            {
                pending.session.record.model = Some(format!("{provider}/{model}"));
            }
        }
        if let Some(data) = row.get::<_, Option<String>>(4)? {
            batch.add_opencode_bytes(data.len() as u64);
            let part = serde_json::from_str(&data)?;
            let timestamp: i64 = row.get::<_, Option<i64>>(5)?.unwrap_or(created);
            if let Some(item) = part_item(&part_id, &pending.message, part, timestamp) {
                if pending.session.record.title.is_empty() && item.kind == ItemKind::User {
                    pending.session.record.title = truncate_title(&item.text);
                }
                pending.session.record.messages.push(item);
            }
        }
        pending.cursor = (created, message_id, part_id);
        read += 1;
    }
    Ok(false)
}

fn part_item(id: &str, message: &Value, part: Value, timestamp: i64) -> Option<ChatItem> {
    let role = message["role"].as_str()?;
    if !matches!(role, "user" | "assistant") || part["ignored"] == true || part["synthetic"] == true
    {
        return None;
    }
    let kind = match (role, part["type"].as_str()?) {
        ("user", "text" | "file") => ItemKind::User,
        ("assistant", "text") => ItemKind::Assistant,
        ("assistant", "reasoning") => ItemKind::Reasoning,
        ("assistant", "tool") => ItemKind::Tool,
        _ => return None,
    };
    let mut item = ChatItemData {
        reasoning_summary: None,
        key: format!("import-opencode-{id}"),
        created_at: timestamp.max(0) as u64 / 1000,
        kind: kind.clone(),
        text: part["text"].as_str().unwrap_or_default().to_owned(),
        input: String::new(),
        detail: String::new(),
        status: Default::default(),
        approval: None,
        model: None,
        elapsed_ms: None,
    };
    if part["type"] == "file" {
        // Do not materialize embedded data URLs from historical attachments.
        item.text = format!("@{}", part["filename"].as_str().unwrap_or("attachment"));
    } else if kind == ItemKind::Tool {
        let state = &part["state"];
        item.text = part["tool"].as_str().unwrap_or("tool").to_owned();
        item.input = serde_json::to_string_pretty(&state["input"]).unwrap_or_default();
        item.detail = state["output"]
            .as_str()
            .or_else(|| state["error"].as_str())
            .unwrap_or_default()
            .to_owned();
        item.status = match state["status"].as_str() {
            Some("completed") => crate::message_status::ToolStatus::Completed,
            Some("error") => crate::message_status::ToolStatus::Failed,
            _ => crate::message_status::ToolStatus::Interrupted,
        };
    }
    (!item.text.trim().is_empty()).then(|| ChatItem::from(item))
}

/// Read one verified descendant in a single SQLite snapshot. No migrations or writes.
pub(super) fn read_child(
    cwd: &Path,
    binding: &crate::subagents::ChildBinding,
    budget: &super::children::Budget<'_>,
) -> anyhow::Result<crate::cli_adapter::child_history::Snapshot> {
    use anyhow::{Context, ensure};
    let connection = Connection::open_with_flags(
        &binding.source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(100))?;
    connection.execute_batch("BEGIN")?;
    let metadata = |id: &str| -> anyhow::Result<(String, Option<String>, i64)> {
        Ok(connection.query_row(
            "SELECT directory, parent_id, time_created FROM session WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?)
    };
    let (directory, parent, created) = metadata(&binding.native)?;
    ensure!(
        same_dir(&directory, cwd) && parent.as_deref() == Some(binding.parent.as_str()),
        "Child history identity or workspace does not match"
    );
    let mut ancestor = binding.parent.clone();
    let mut seen = std::collections::HashSet::from([binding.native.clone()]);
    loop {
        budget.check(0, seen.len())?;
        ensure!(
            seen.insert(ancestor.clone()) && seen.len() <= 256,
            "Invalid child ancestry"
        );
        let (directory, parent, _) = metadata(&ancestor)?;
        ensure!(
            same_dir(&directory, cwd),
            "Child history ancestor workspace does not match"
        );
        if ancestor == binding.root_native {
            break;
        }
        ancestor = parent.context("Child history is not descended from the root session")?;
    }
    let mut statement = connection.prepare(
        "SELECT m.id, m.data, COALESCE(p.id, ''), p.data, COALESCE(p.time_created, m.time_created)
         FROM message m LEFT JOIN part p ON p.message_id = m.id AND p.session_id = m.session_id
         WHERE m.session_id = ?1 ORDER BY m.time_created, m.id, p.id",
    )?;
    let mut rows = statement.query([&binding.native])?;
    let mut bytes = 0;
    let mut count = 0;
    let mut messages = Vec::new();
    let mut last_id = String::new();
    let mut message = Value::Null;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        if id != last_id {
            let data = row.get_ref(1)?.as_str()?;
            bytes += data.len() as u64;
            budget.check(bytes, count)?;
            message = serde_json::from_str(data)?;
            last_id = id;
        }
        count += 1;
        budget.check(bytes, count)?;
        let data = row.get_ref(3)?;
        if data != rusqlite::types::ValueRef::Null {
            let data = data.as_str()?;
            bytes += data.len() as u64;
            budget.check(bytes, count)?;
            let part: Value = serde_json::from_str(data)?;
            let part_id: String = row.get(2)?;
            ensure!(!part_id.is_empty(), "Missing child message part identity");
            if let Some(mut item) = part_item(&part_id, &message, part, row.get(4)?) {
                item.key = format!("native-{}-{part_id}", last_id);
                messages.push(item);
            }
        }
    }
    // Database completion alone does not confer ownership; the UI also requires
    // the parent connection to be gone and an observed terminal child task.
    let idle = message["role"] == "assistant"
        && (message.get("error").is_some_and(|v| !v.is_null())
            || (message["time"]["completed"].as_u64().is_some()
                && message["finish"]
                    .as_str()
                    .is_some_and(|v| !matches!(v, "tool-calls" | "unknown"))));
    budget.check(bytes, count)?;
    Ok(crate::cli_adapter::child_history::Snapshot::Native {
        created_at: created.max(0) as u64 / 1000,
        messages,
        idle,
    })
}
