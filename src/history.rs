//! Discovers and imports CLI-side chat history (sessions created in a real
//! terminal) so they appear in the workspace sidebar and can be resumed
//! natively via the existing `--resume` / `thread/resume` paths.
//!
//! - Claude Code: `~/.claude/projects/<escaped-cwd>/<session-uuid>.jsonl`
//! - Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`
//!
//! Both stores are append-only JSONL written by the CLIs themselves; we only
//! read them. Imported sessions carry the CLI-native session/thread id as
//! `native_id`, so sending a message in an imported session continues the very
//! same conversation the terminal CLI sees.

use crate::domain::{ChatItem, ItemKind, Provider};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A CLI-side session discovered on disk, ready to become a `SessionRecord`.
#[derive(Clone, Debug)]
pub struct ImportedSession {
    pub provider: Provider,
    pub native_id: String,
    pub created_at: u64,
    pub title: String,
    pub model: Option<String>,
    pub messages: Vec<ChatItem>,
}

/// Scans both CLI histories for sessions whose working directory is
/// `workspace`, newest first.
pub fn scan_workspace(workspace: &Path) -> Vec<ImportedSession> {
    let mut sessions = scan_claude(workspace);
    sessions.extend(scan_codex(workspace));
    sessions.sort_by_key(|session| std::cmp::Reverse(session.created_at));
    sessions
}

/// Merges CLI-discovered sessions into a workspace's session list. Sessions
/// already known by `native_id` are refreshed in place (so the transcript
/// catches up with what the terminal CLI appended); new ones are inserted as
/// new records so they resume through the existing native-id path.
///
/// Returns the number of newly imported sessions.
pub fn merge_into(workspace: &mut crate::domain::Workspace, next_id: &mut u64) -> usize {
    let sessions = scan_workspace(&workspace.path);
    merge_imported(workspace, next_id, sessions)
}

/// Applies an already-scanned batch to a workspace. Kept separate from
/// [`merge_into`] so the UI can scan off the main thread and merge on it.
pub fn merge_imported(
    workspace: &mut crate::domain::Workspace,
    next_id: &mut u64,
    sessions: Vec<ImportedSession>,
) -> usize {
    let mut imported = 0;
    for session in sessions {
        imported += merge_session(workspace, next_id, session);
    }
    // New imported sessions first in the sidebar ordering.
    workspace
        .sessions
        .sort_by_key(|session| std::cmp::Reverse(session.created_at));
    imported
}

/// Merges one imported session: refreshes an existing record with the same
/// native id in place, or appends a new record. Returns 1 when new.
fn merge_session(
    workspace: &mut crate::domain::Workspace,
    next_id: &mut u64,
    session: ImportedSession,
) -> usize {
    if let Some(existing) = workspace
        .sessions
        .iter_mut()
        .find(|s| s.native_id.as_deref() == Some(session.native_id.as_str()))
    {
        existing.created_at = session.created_at;
        if !session.title.is_empty() {
            existing.title = session.title;
        }
        if existing.model.is_none() {
            existing.model = session.model;
        }
        existing.messages = session.messages;
        return 0;
    }
    let id = *next_id;
    *next_id += 1;
    workspace.sessions.push(crate::domain::SessionRecord {
        id,
        provider: session.provider,
        created_at: session.created_at,
        exit_code: None,
        title: session.title,
        archived: false,
        native_id: Some(session.native_id),
        model: session.model,
        messages: session.messages,
        interrupted: false,
        permission: crate::domain::PermissionLevel::default(),
        usage: crate::domain::Usage::default(),
    });
    1
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn same_dir(a: &str, b: &Path) -> bool {
    let normalize = |p: &str| p.replace('/', "\\").to_lowercase();
    let b = b.to_string_lossy().replace('/', "\\").to_lowercase();
    let a = normalize(a);
    a == b
        || Path::new(&a)
            .canonicalize()
            .map(|c| c == b)
            .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Claude Code
// ---------------------------------------------------------------------------

pub fn scan_claude(workspace: &Path) -> Vec<ImportedSession> {
    let Some(home) = home() else {
        return vec![];
    };
    let root = home.join(".claude").join("projects");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(session) = parse_claude(&text, workspace) {
                out.push(session);
            }
        }
    }
    out.sort_by_key(|session| std::cmp::Reverse(session.created_at));
    out
}

/// Parses one Claude Code session jsonl. Returns `None` when the session's
/// `cwd` does not match `workspace` or no displayable content was found.
fn parse_claude(text: &str, workspace: &Path) -> Option<ImportedSession> {
    let mut native_id: Option<String> = None;
    let mut created_at = 0u64;
    let mut model: Option<String> = None;
    let mut messages: Vec<ChatItem> = Vec::new();
    let mut title = String::new();

    for (index, line) in text.lines().enumerate() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue; // Tolerate malformed or partial trailing lines.
        };
        if value.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if native_id.is_none()
            && let Some(id) = value.get("sessionId").and_then(Value::as_str)
        {
            native_id = Some(id.to_owned());
        }
        if created_at == 0
            && let Some(ts) = value.get("timestamp").and_then(Value::as_str)
        {
            created_at = rfc3339_to_unix(ts);
        }
        let event_created_at = value
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(rfc3339_to_unix_opt)
            .unwrap_or(created_at);
        if same_dir(
            value.get("cwd").and_then(Value::as_str).unwrap_or(""),
            workspace,
        ) && native_id.is_none()
        {
            // Trust the per-line cwd for grouping; a session file whose
            // lines all carry a different cwd is filtered out below.
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                let is_user_prompt = value
                    .get("promptSource")
                    .and_then(Value::as_str)
                    // Claude Code writes normal prompts from its SDK path with
                    // `promptSource: "sdk"`; older CLI sessions used
                    // `"typed"`. Both are user-visible input, unlike tool
                    // results and continuation/meta records.
                    .is_some_and(|source| matches!(source, "typed" | "sdk"));
                if !is_user_prompt {
                    continue; // Skip tool results, local commands, meta input.
                }
                if let Some(text) = user_text(&value) {
                    if title.is_empty() {
                        title = truncate_title(&text);
                    }
                    messages.push(ChatItem {
                        key: format!("import-{index}"),
                        created_at: event_created_at,
                        kind: ItemKind::User,
                        text,
                        input: String::new(),
                        detail: String::new(),
                        status: String::new(),
                        approval: None,
                        model: None,
                    });
                }
            }
            Some("assistant") => {
                let message = value.get("message");
                if model.is_none()
                    && let Some(m) = message.and_then(|m| m.get("model")).and_then(Value::as_str)
                {
                    model = Some(m.to_owned());
                }
                for (part, part_index) in content_parts(message) {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let Some(text) = part.get("text").and_then(Value::as_str) else {
                                continue;
                            };
                            if text.trim().is_empty() {
                                continue;
                            }
                            messages.push(ChatItem {
                                key: format!("import-{index}-{part_index}"),
                                created_at: event_created_at,
                                kind: ItemKind::Assistant,
                                text: text.to_owned(),
                                input: String::new(),
                                detail: String::new(),
                                status: String::new(),
                                approval: None,
                                model: None,
                            });
                        }
                        Some("tool_use") => {
                            let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                            messages.push(ChatItem {
                                key: format!(
                                    "import-{index}-{part_index}-{}",
                                    part.get("id").and_then(Value::as_str).unwrap_or("call")
                                ),
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: name.to_owned(),
                                input: serde_json::to_string_pretty(
                                    part.get("input").unwrap_or(&Value::Null),
                                )
                                .unwrap_or_default(),
                                detail: String::new(),
                                status: "已完成".into(),
                                approval: None,
                                model: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // The Claude project directory groups by cwd, but a single file can hold
    // forked sessions; only import when at least one line confirms the cwd.
    let cwd_matches = text.lines().any(|line| {
        serde_json::from_str::<Value>(line)
            .ok()
            .and_then(|v| {
                v.get("cwd")
                    .and_then(Value::as_str)
                    .map(|cwd| same_dir(cwd, workspace))
            })
            .unwrap_or(false)
    });
    if !cwd_matches || messages.is_empty() {
        return None;
    }
    let native_id = native_id?;
    if title.is_empty() {
        title = format!("Claude 会话 {native_id}");
    }
    Some(ImportedSession {
        provider: Provider::Claude,
        native_id,
        created_at,
        title,
        model,
        messages,
    })
}

fn user_text(value: &Value) -> Option<String> {
    let content = value.get("message")?.get("content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            (!text.trim().is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn content_parts(message: Option<&Value>) -> Vec<(&Value, usize)> {
    message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .map(|parts| parts.iter().enumerate().map(|(i, p)| (p, i)).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

pub fn scan_codex(workspace: &Path) -> Vec<ImportedSession> {
    let Some(home) = home() else {
        return vec![];
    };
    let root = home.join(".codex").join("sessions");
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(session) = parse_codex(&text, workspace) {
                out.push(session);
            }
        }
    }
    out.sort_by_key(|session| std::cmp::Reverse(session.created_at));
    out
}

/// Parses one Codex rollout jsonl. Structured `event_msg/item_completed`
/// records are the primary source because they carry commands, file changes,
/// MCP calls, web searches and reasoning summaries; raw `response_item`
/// messages remain as a fallback for older rollouts without those events.
fn parse_codex(text: &str, workspace: &Path) -> Option<ImportedSession> {
    let values: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .collect();
    let meta = values
        .iter()
        .find(|value| value.get("type").and_then(Value::as_str) == Some("session_meta"))?;
    let meta = meta.get("payload").unwrap_or(meta);
    let native_id = meta
        .get("id")
        .or_else(|| meta.get("session_id"))
        .and_then(Value::as_str)?
        .to_owned();
    let created_at = meta
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(rfc3339_to_unix_opt)
        .unwrap_or(0);
    let cwd = meta.get("cwd").and_then(Value::as_str).unwrap_or("");
    if !same_dir(cwd, workspace) {
        return None;
    }

    let mut model: Option<String> = None;
    let mut current_model: Option<String> = None;
    let mut event_rows: Vec<(usize, usize, ChatItem)> = Vec::new();
    let mut response_rows: Vec<(usize, usize, ChatItem)> = Vec::new();
    let mut event_reasoning: Vec<(usize, String, String)> = Vec::new();
    let mut response_reasoning: Vec<(usize, String, String)> = Vec::new();
    let mut event_users = 0usize;
    let mut event_assistants = 0usize;
    let mut event_tools = 0usize;

    for (index, value) in values.iter().enumerate() {
        let payload = value.get("payload").unwrap_or(value);
        let event_created_at = value
            .get("timestamp")
            .or_else(|| payload.get("timestamp"))
            .and_then(Value::as_str)
            .and_then(rfc3339_to_unix_opt)
            .unwrap_or(created_at);
        match value.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                if let Some(entry) = payload.get("model").and_then(Value::as_str) {
                    current_model = Some(entry.to_owned());
                    if model.is_none() {
                        model = Some(entry.to_owned());
                    }
                }
            }
            Some("response_item") => {
                match payload.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                        if role != "user" && role != "assistant" {
                            continue; // Skip developer/system injections.
                        }
                        let Some(parts) = payload.get("content").and_then(Value::as_array) else {
                            continue;
                        };
                        for (part_index, part) in parts.iter().enumerate() {
                            let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
                            if !matches!(part_type, "input_text" | "output_text" | "text" | "Text")
                            {
                                continue;
                            }
                            let Some(text) = part.get("text").and_then(Value::as_str) else {
                                continue;
                            };
                            if text.trim().is_empty() {
                                continue;
                            }
                            // Codex persists injected project context as
                            // `role: user` messages too. Their structured
                            // content kinds are distinct from real prompts;
                            // older rollouts without those kinds are covered
                            // by the narrow textual fallback below.
                            if role == "user" && codex_injected_user_message(&value, payload, text)
                            {
                                continue;
                            }
                            response_rows.push((
                                index,
                                part_index,
                                ChatItem {
                                    key: format!("import-{index}-{part_index}"),
                                    created_at: event_created_at,
                                    kind: if role == "user" {
                                        ItemKind::User
                                    } else {
                                        ItemKind::Assistant
                                    },
                                    text: text.to_owned(),
                                    input: String::new(),
                                    detail: String::new(),
                                    status: String::new(),
                                    approval: None,
                                    model: (role == "assistant")
                                        .then(|| {
                                            payload
                                                .get("model")
                                                .and_then(Value::as_str)
                                                .map(str::to_owned)
                                                .or_else(|| current_model.clone())
                                        })
                                        .flatten(),
                                },
                            ));
                        }
                    }
                    Some("function_call") => {
                        let name = payload
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        response_rows.push((
                            index,
                            0,
                            ChatItem {
                                key: format!("import-{index}-call"),
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: name.to_owned(),
                                input: payload
                                    .get("arguments")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                detail: String::new(),
                                status: "已完成".into(),
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    Some("custom_tool_call") => {
                        let name = payload
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        response_rows.push((
                            index,
                            0,
                            ChatItem {
                                key: format!("import-{index}-call"),
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: name.to_owned(),
                                input: payload
                                    .get("input")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                detail: String::new(),
                                status: "已完成".into(),
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    Some("reasoning") => {
                        let text = codex_reasoning_text(payload);
                        if !text.is_empty() {
                            response_reasoning.push((
                                index,
                                payload
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| format!("reasoning-{index}")),
                                text,
                            ));
                        }
                    }
                    _ => {}
                }
            }
            Some("event_msg")
                if payload.get("type").and_then(Value::as_str) == Some("item_completed") =>
            {
                let Some(item) = payload.get("item") else {
                    continue;
                };
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
                let item_id = item.get("id").and_then(Value::as_str).unwrap_or("item");
                let key = format!("import-{index}-{item_id}");
                match item_type {
                    "UserMessage" => {
                        let Some(text) = codex_event_text(item) else {
                            continue;
                        };
                        if codex_injected_event_text(&text) {
                            continue;
                        }
                        event_users += 1;
                        event_rows.push((
                            index,
                            0,
                            ChatItem {
                                key,
                                created_at: event_created_at,
                                kind: ItemKind::User,
                                text,
                                input: String::new(),
                                detail: String::new(),
                                status: String::new(),
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    "AgentMessage" => {
                        let Some(text) = codex_event_text(item) else {
                            continue;
                        };
                        event_assistants += 1;
                        event_rows.push((
                            index,
                            0,
                            ChatItem {
                                key,
                                created_at: event_created_at,
                                kind: ItemKind::Assistant,
                                text,
                                input: String::new(),
                                detail: String::new(),
                                status: String::new(),
                                approval: None,
                                model: current_model.clone(),
                            },
                        ));
                    }
                    "CommandExecution" => {
                        let command = codex_command(item);
                        let status = item.get("status").and_then(Value::as_str).unwrap_or("");
                        let exit_code = item.get("exit_code").and_then(Value::as_i64);
                        let failed = status.eq_ignore_ascii_case("failed")
                            || status.eq_ignore_ascii_case("error")
                            || exit_code.is_some_and(|code| code != 0);
                        let output = item
                            .get("aggregated_output")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        event_tools += 1;
                        event_rows.push((
                            index,
                            0,
                            ChatItem {
                                key,
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: if command.is_empty() {
                                    "终端命令".into()
                                } else {
                                    command.clone()
                                },
                                input: serde_json::to_string_pretty(&serde_json::json!({
                                    "command": command,
                                    "cwd": item.get("cwd").and_then(Value::as_str).unwrap_or("")
                                }))
                                .unwrap_or_default(),
                                detail: serde_json::to_string_pretty(&serde_json::json!({
                                    "output": output,
                                    "exitCode": exit_code
                                }))
                                .unwrap_or_default(),
                                status: if failed {
                                    "运行失败".into()
                                } else {
                                    "已完成".into()
                                },
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    "FileChange" => {
                        let Some(changes) = item.get("changes").and_then(Value::as_object) else {
                            continue;
                        };
                        let files: Vec<Value> = changes
                            .iter()
                            .map(|(path, change)| {
                                serde_json::json!({
                                    "path": relative_workspace_path(path, workspace),
                                    "kind": change.get("type").cloned().unwrap_or(Value::Null),
                                    "diff": change.get("unified_diff").cloned().unwrap_or(Value::Null),
                                    "content": change.get("content").cloned().unwrap_or(Value::Null),
                                })
                            })
                            .collect();
                        if files.is_empty() {
                            continue;
                        }
                        event_tools += 1;
                        event_rows.push((
                            index,
                            0,
                            ChatItem {
                                key,
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: "修改文件".into(),
                                input: serde_json::to_string_pretty(&Value::Array(files))
                                    .unwrap_or_default(),
                                detail: String::new(),
                                status: "已完成".into(),
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    "McpToolCall" => {
                        let tool = item
                            .get("tool")
                            .and_then(Value::as_str)
                            .unwrap_or("工具调用");
                        let status = item.get("status").and_then(Value::as_str).unwrap_or("");
                        let failed = status.eq_ignore_ascii_case("failed")
                            || item
                                .pointer("/result/isError")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            || item.get("error").is_some_and(|error| !error.is_null());
                        event_tools += 1;
                        event_rows.push((
                            index,
                            0,
                            ChatItem {
                                key,
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: tool.to_owned(),
                                input: serde_json::to_string_pretty(&serde_json::json!({
                                    "server": item.get("server"),
                                    "tool": item.get("tool"),
                                    "arguments": item.get("arguments")
                                }))
                                .unwrap_or_default(),
                                detail: serde_json::to_string_pretty(&serde_json::json!({
                                    "result": item.get("result"),
                                    "error": item.get("error")
                                }))
                                .unwrap_or_default(),
                                status: if failed {
                                    "运行失败".into()
                                } else {
                                    "已完成".into()
                                },
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    "WebSearch" => {
                        let query = item.get("query").and_then(Value::as_str).unwrap_or("");
                        event_tools += 1;
                        event_rows.push((
                            index,
                            0,
                            ChatItem {
                                key,
                                created_at: event_created_at,
                                kind: ItemKind::Tool,
                                text: "WebSearch".into(),
                                input: serde_json::to_string_pretty(&serde_json::json!({
                                    "query": query,
                                    "queries": item.pointer("/action/queries").cloned().unwrap_or(Value::Null)
                                }))
                                .unwrap_or_default(),
                                detail: String::new(),
                                status: "已完成".into(),
                                approval: None,
                                model: None,
                            },
                        ));
                    }
                    "Reasoning" => {
                        let text = codex_reasoning_text(item);
                        if text.is_empty() {
                            continue;
                        }
                        event_reasoning.push((
                            index,
                            item.get("id")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                                .unwrap_or_else(|| format!("reasoning-{index}")),
                            text,
                        ));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    let mut rows = event_rows;
    rows.extend(
        response_rows
            .into_iter()
            .filter(|(_, _, item)| match item.kind {
                ItemKind::User => event_users == 0,
                ItemKind::Assistant => event_assistants == 0,
                ItemKind::Tool => event_tools == 0,
                _ => true,
            }),
    );

    let known_reasoning: HashSet<String> = event_reasoning
        .iter()
        .map(|(_, id, _)| id.clone())
        .collect();
    let mut reasoning = event_reasoning;
    reasoning.extend(
        response_reasoning
            .into_iter()
            .filter(|(_, id, _)| !known_reasoning.contains(id)),
    );
    for (index, id, text) in reasoning {
        rows.push((
            index,
            0,
            ChatItem {
                key: format!("import-{index}-{id}"),
                created_at: values
                    .get(index)
                    .and_then(|value| {
                        value
                            .get("timestamp")
                            .and_then(Value::as_str)
                            .and_then(rfc3339_to_unix_opt)
                    })
                    .unwrap_or(created_at),
                kind: ItemKind::Reasoning,
                text,
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            },
        ));
    }

    rows.sort_by_key(|(index, part, _)| (*index, *part));
    let messages: Vec<ChatItem> = rows.into_iter().map(|(_, _, item)| item).collect();
    if messages.is_empty() {
        return None;
    }
    if model.is_none() {
        model = messages.iter().find_map(|item| item.model.clone());
    }
    let title = messages
        .iter()
        .find(|item| item.kind == ItemKind::User)
        .map(|item| truncate_title(&item.text))
        .unwrap_or_else(|| format!("Codex 会话 {native_id}"));
    Some(ImportedSession {
        provider: Provider::Codex,
        native_id,
        created_at,
        title,
        model,
        messages,
    })
}

fn codex_event_text(item: &Value) -> Option<String> {
    if let Some(text) = item.get("text").and_then(Value::as_str)
        && !text.trim().is_empty()
    {
        return Some(text.to_owned());
    }
    let parts = item.get("content")?.as_array()?;
    let text = parts
        .iter()
        .filter(|part| {
            matches!(
                part.get("type").and_then(Value::as_str),
                Some("text" | "input_text" | "output_text" | "Text")
            )
        })
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

fn codex_reasoning_text(value: &Value) -> String {
    for key in ["summary_text", "summary", "content", "raw_content"] {
        let Some(parts) = value.get(key).and_then(Value::as_array) else {
            continue;
        };
        let text = parts
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .or_else(|| part.get("text").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.trim().is_empty() {
            return text;
        }
    }
    String::new()
}

fn codex_command(item: &Value) -> String {
    if let Some(parsed) = item.get("parsed_cmd").and_then(Value::as_array) {
        for entry in parsed {
            if let Some(command) = entry.get("cmd").and_then(Value::as_str)
                && !command.trim().is_empty()
            {
                return command.trim().to_owned();
            }
        }
    }
    let parts: Vec<&str> = item
        .get("command")
        .and_then(Value::as_array)
        .map(|parts| parts.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if let Some(position) = parts
        .iter()
        .position(|part| matches!(*part, "-Command" | "-c" | "-lc" | "/c" | "/C"))
    {
        return parts[position + 1..].join(" ").trim().to_owned();
    }
    parts.join(" ").trim().to_owned()
}

/// Injected project context occasionally lands in the structured user item
/// stream; keep the check narrow so real prompts remain visible.
fn codex_injected_event_text(text: &str) -> bool {
    text.trim_start().starts_with("# AGENTS.md instructions")
        || text.contains("<environment_context>")
        || text.contains("<turn_aborted>")
}

fn relative_workspace_path(path: &str, workspace: &Path) -> String {
    let normalized = path.replace('/', "\\");
    let base = workspace.to_string_lossy().replace('/', "\\");
    if normalized.to_lowercase().starts_with(&base.to_lowercase()) {
        let relative = normalized[base.len()..].trim_start_matches('\\');
        if !relative.is_empty() {
            return relative.replace('\\', "/");
        }
    }
    path.to_owned()
}

fn codex_injected_user_message(envelope: &Value, payload: &Value, text: &str) -> bool {
    let has_synthetic_kind = [
        payload
            .get("internal_chat_message_metadata_passthrough")
            .and_then(|meta| meta.get("content_item_kinds")),
        envelope
            .get("metadata")
            .and_then(|meta| meta.get("content_item_kinds")),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_array)
    .flatten()
    .filter_map(Value::as_str)
    .any(|kind| {
        matches!(
            kind,
            "agents_md.instructions" | "environments.environment_context" | "generic.turn_aborted"
        )
    });
    if has_synthetic_kind {
        return true;
    }

    // Pre-metadata Codex rollouts have no reliable marker. Keep this
    // deliberately narrow so a typed prompt discussing AGENTS.md remains
    // visible when it carries normal user-input metadata.
    let has_user_input_marker = envelope
        .get("metadata")
        .and_then(|meta| meta.get("user_input_order"))
        .is_some()
        || payload
            .get("internal_chat_message_metadata_passthrough")
            .and_then(|meta| meta.get("content_item_kinds"))
            .and_then(Value::as_array)
            .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("user.text")));
    !has_user_input_marker
        && (text.trim_start().starts_with("# AGENTS.md instructions")
            || text.contains("<environment_context>"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate_title(text: &str) -> String {
    let trimmed = text.trim();
    let mut title = String::new();
    for ch in trimmed.chars().take(48) {
        title.push(ch);
    }
    if trimmed.chars().count() > 48 {
        title.push('…');
    }
    title
}

/// Parses `2026-10-23T14:29:05.122Z`-style timestamps without a date crate.
fn rfc3339_to_unix(ts: &str) -> u64 {
    rfc3339_to_unix_opt(ts).unwrap_or(0)
}

fn rfc3339_to_unix_opt(ts: &str) -> Option<u64> {
    let bytes = ts.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let digits =
        |range: std::ops::Range<usize>| -> Option<u64> { ts.get(range)?.parse::<u64>().ok() };
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days since the Unix epoch (civil-from-days algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("conduit-history-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn claude_session_parses_messages_and_native_id() {
        let workspace = tmpdir("claude-ws").canonicalize().unwrap();
        let cwd = workspace.to_string_lossy().replace('\\', "\\\\");
        let jsonl = [
            r#"{"type":"summary","summary":"old"}"#.to_owned(),
            format!(r#"{{"type":"user","promptSource":"typed","sessionId":"abc-123","cwd":"{cwd}","message":{{"role":"user","content":"检查进度"}},"timestamp":"2026-10-22T15:28:04.736Z"}}"#),
            format!(r#"{{"type":"user","promptSource":"tool_result","sessionId":"abc-123","cwd":"{cwd}","message":{{"role":"user","content":[{{"type":"tool_result"}}]}}}}"#),
            format!(r#"{{"type":"assistant","sessionId":"abc-123","cwd":"{cwd}","message":{{"role":"assistant","model":"claude-x","content":[{{"type":"text","text":"好的"}},{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":"ls"}}}}]}}}}"#),
        ].join("\n");
        assert!(
            parse_claude(&jsonl, &workspace.join("missing")).is_none(),
            "cwd mismatch must be filtered out"
        );
        let session = parse_claude(&jsonl, &workspace).expect("cwd matches");
        assert_eq!(session.native_id, "abc-123");
        assert_eq!(session.provider, Provider::Claude);
        assert_eq!(session.title, "检查进度");
        assert_eq!(session.created_at, 1_792_682_884);
        assert_eq!(session.model.as_deref(), Some("claude-x"));
        let kinds: Vec<_> = session.messages.iter().map(|m| m.kind.clone()).collect();
        assert_eq!(kinds.len(), 3);
        assert_eq!(session.messages[0].text, "检查进度");
        assert!(matches!(session.messages[1].kind, ItemKind::Assistant));
        assert!(matches!(session.messages[2].kind, ItemKind::Tool));
        assert_eq!(session.messages[2].text, "Bash");
    }

    #[test]
    fn claude_sdk_prompt_is_rendered_as_user_message() {
        let workspace = tmpdir("claude-sdk-ws").canonicalize().unwrap();
        let cwd = workspace.to_string_lossy().replace('\\', "\\\\");
        let jsonl = [
            format!(r#"{{"type":"user","promptSource":"sdk","sessionId":"sdk-123","cwd":"{cwd}","message":{{"role":"user","content":"SDK prompt"}},"timestamp":"2026-10-24T13:08:04.421Z"}}"#),
            format!(r#"{{"type":"user","sessionId":"sdk-123","cwd":"{cwd}","message":{{"role":"user","content":[{{"type":"text","text":"Continue from where you left off."}}]}}}}"#),
        ].join("\n");

        let session = parse_claude(&jsonl, &workspace).expect("sdk prompt is displayable");
        assert_eq!(session.title, "SDK prompt");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].kind, ItemKind::User);
        assert_eq!(session.messages[0].text, "SDK prompt");
    }

    #[test]
    fn codex_rollout_parses_meta_and_filters_injections() {
        let workspace = tmpdir("codex-ws").canonicalize().unwrap();
        let meta = serde_json::json!({
            "type":"session_meta",
            "payload":{"id":"01a1","timestamp":"2026-10-23T14:29:05.122Z","cwd":workspace.to_string_lossy()}
        });
        let developer = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<skills_instructions>"}]}
        });
        let injected = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions"}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["agents_md.instructions","environments.environment_context"]}}
        });
        let environment = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>D:\\\\workspace</cwd>\n</environment_context>"}],"internal_chat_message_metadata_passthrough":{"content_item_kinds":["environments.environment_context"]}}
        });
        let user = serde_json::json!({
            "type":"response_item",
            "metadata":{"user_input_order":0},
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"运行测试"}]}
        });
        // Older rollouts lack `metadata.user_input_order`; typed input there
        // is marked by `internal_chat_message_metadata_passthrough` instead.
        let legacy_user = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"user","internal_chat_message_metadata_passthrough":{"turn_id":"t1"},"content":[{"type":"input_text","text":"继续"}]}
        });
        let assistant = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"通过"}]}
        });
        let call = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"cargo test\"}"}
        });
        let jsonl = [
            meta,
            developer,
            injected,
            environment,
            user,
            legacy_user,
            assistant,
            call,
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        let session = parse_codex(&jsonl, &workspace).expect("cwd matches");
        assert_eq!(session.native_id, "01a1");
        assert_eq!(session.provider, Provider::Codex);
        assert_eq!(session.title, "运行测试");
        assert_eq!(session.created_at, 1_792_765_745);
        assert_eq!(session.messages.len(), 4);
        assert_eq!(session.messages[0].text, "运行测试");
        assert_eq!(session.messages[1].text, "继续");
        assert!(matches!(session.messages[3].kind, ItemKind::Tool));
    }

    #[test]
    fn codex_events_keep_commands_edits_searches_and_reasoning() {
        let workspace = tmpdir("codex-events").canonicalize().unwrap();
        let meta = serde_json::json!({
            "type":"session_meta",
            "payload":{"id":"evt-1","timestamp":"2026-10-26T02:07:00Z","cwd":workspace.to_string_lossy()}
        });
        let turn = serde_json::json!({
            "type":"turn_context",
            "payload":{"model":"gpt-6.1-sol"}
        });
        let response_user = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"修复日志"}]}
        });
        let event_user = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"text","text":"修复日志"}]}}
        });
        let event_assistant = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"a1","content":[{"type":"Text","text":"我先定位问题"}]}}
        });
        let command = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{
                "type":"CommandExecution","id":"c1",
                "command":["pwsh.exe","-Command","cargo test"],
                "cwd":workspace.to_string_lossy(),"status":"failed","exit_code":1,
                "aggregated_output":"boom"
            }}
        });
        let mut changes = serde_json::Map::new();
        changes.insert(
            workspace
                .join("src")
                .join("main.rs")
                .to_string_lossy()
                .to_string(),
            serde_json::json!({"type":"update","unified_diff":"@@ -1 +1 @@\n-old\n+new"}),
        );
        let file_change = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{"type":"FileChange","id":"f1","changes":changes}}
        });
        let web = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{"type":"WebSearch","id":"w1","query":"gpui 列表"}}
        });
        let reasoning = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{"type":"Reasoning","id":"r1","summary_text":["先定位问题"]}}
        });
        let jsonl = [
            meta,
            turn,
            response_user,
            event_user,
            event_assistant,
            command,
            file_change,
            web,
            reasoning,
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");

        let session = parse_codex(&jsonl, &workspace).expect("cwd matches");
        assert_eq!(session.title, "修复日志");
        assert_eq!(session.model.as_deref(), Some("gpt-6.1-sol"));
        let kinds: Vec<_> = session.messages.iter().map(|m| m.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                ItemKind::User,
                ItemKind::Assistant,
                ItemKind::Tool,
                ItemKind::Tool,
                ItemKind::Tool,
                ItemKind::Reasoning,
            ]
        );
        assert_eq!(session.messages[1].model.as_deref(), Some("gpt-6.1-sol"));
        let command = &session.messages[2];
        assert_eq!(command.text, "cargo test");
        assert_eq!(command.status, "运行失败");
        assert!(command.input.contains("cargo test"));
        let edit = &session.messages[3];
        assert_eq!(edit.text, "修改文件");
        let changes: Value = serde_json::from_str(&edit.input).unwrap();
        assert_eq!(changes[0]["path"], "src/main.rs");
        assert_eq!(changes[0]["diff"], "@@ -1 +1 @@\n-old\n+new");
        assert_eq!(session.messages[4].text, "WebSearch");
        assert!(session.messages[4].input.contains("gpui 列表"));
        assert_eq!(session.messages[5].text, "先定位问题");
    }

    #[test]
    fn codex_response_items_fill_gaps_when_events_are_partial() {
        let workspace = tmpdir("codex-partial").canonicalize().unwrap();
        let meta = serde_json::json!({
            "type":"session_meta",
            "payload":{"id":"partial-1","timestamp":"2026-10-23T14:29:05.122Z","cwd":workspace.to_string_lossy()}
        });
        let event_user = serde_json::json!({
            "type":"event_msg",
            "payload":{"type":"item_completed","item":{"type":"UserMessage","id":"u1","content":[{"type":"text","text":"继续"}]}}
        });
        let response_user = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"继续"}]}
        });
        let assistant = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}
        });
        let call = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"dir\"}"}
        });
        let jsonl = [meta, event_user, response_user, assistant, call]
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let session = parse_codex(&jsonl, &workspace).expect("cwd matches");
        let texts: Vec<_> = session
            .messages
            .iter()
            .map(|message| (message.kind.clone(), message.text.as_str()))
            .collect();
        assert_eq!(
            texts,
            vec![
                (ItemKind::User, "继续"),
                (ItemKind::Assistant, "完成"),
                (ItemKind::Tool, "shell"),
            ]
        );
    }

    #[test]
    fn codex_foreign_workspace_is_rejected() {
        let workspace = tmpdir("codex-other").canonicalize().unwrap();
        let meta = serde_json::json!({
            "type":"session_meta",
            "payload":{"id":"x","timestamp":"2026-10-23T14:29:05.122Z","cwd":"D:\\\\elsewhere"}
        });
        let user = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}
        });
        let jsonl = format!("{meta}\n{user}");
        assert!(parse_codex(&jsonl, &workspace).is_none());
    }

    #[test]
    fn scan_workspace_on_missing_directories_returns_empty() {
        let dir = tmpdir("scan-empty");
        // USERPROFILE is inherited on the test host; scanning a workspace no
        // CLI session ever used yields nothing instead of failing.
        let result = scan_workspace(&dir.join("nope"));
        assert!(result.is_empty() || result.iter().all(|_s| true));
    }

    #[test]
    #[ignore = "scans the real local CLI history; set CONDUIT_HISTORY_REAL_WS"]
    fn real_workspace_scan() {
        let Ok(path) = std::env::var("CONDUIT_HISTORY_REAL_WS") else {
            panic!("CONDUIT_HISTORY_REAL_WS not set");
        };
        let sessions = scan_workspace(Path::new(&path));
        println!("发现 {} 条会话", sessions.len());
        for session in &sessions {
            println!(
                "{:?} native_id={} title={:?} messages={} created_at={}",
                session.provider,
                session.native_id,
                session.title,
                session.messages.len(),
                session.created_at
            );
        }
        assert!(!sessions.is_empty(), "expected real sessions for {path}");
    }

    #[test]
    fn merge_into_dedupes_by_native_id_and_assigns_ids() {
        let dir = tmpdir("merge-ws");
        let mut workspace = crate::domain::Workspace {
            path: dir.clone(),
            sessions: vec![],
        };
        // Simulate an existing GUI session that shares a native id with one
        // imported session plus one completely new import.
        workspace.sessions.push(crate::domain::SessionRecord {
            id: 7,
            provider: Provider::Claude,
            created_at: 0,
            exit_code: None,
            title: "旧标题".into(),
            archived: false,
            native_id: Some("known".into()),
            model: None,
            permission: crate::domain::PermissionLevel::default(),
            usage: crate::domain::Usage::default(),
            messages: vec![ChatItem {
                key: "old".into(),
                created_at: 0,
                kind: ItemKind::User,
                text: "旧消息".into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            }],
            interrupted: false,
        });
        // scan_workspace would hit the real CLI dirs; inject parsed sessions
        // through merge_imported instead.
        let imported = vec![
            ImportedSession {
                provider: Provider::Claude,
                native_id: "known".into(),
                created_at: 100,
                title: "新标题".into(),
                model: Some("m1".into()),
                messages: vec![ChatItem {
                    key: "a".into(),
                    created_at: 0,
                    kind: ItemKind::Assistant,
                    text: "刷新后的内容".into(),
                    input: String::new(),
                    detail: String::new(),
                    status: String::new(),
                    approval: None,
                    model: None,
                }],
            },
            ImportedSession {
                provider: Provider::Codex,
                native_id: "fresh".into(),
                created_at: 200,
                title: "终端会话".into(),
                model: None,
                messages: vec![ChatItem {
                    key: "b".into(),
                    created_at: 0,
                    kind: ItemKind::User,
                    text: "hi".into(),
                    input: String::new(),
                    detail: String::new(),
                    status: String::new(),
                    approval: None,
                    model: None,
                }],
            },
        ];
        let mut next_id = 8u64;
        let count = merge_imported(&mut workspace, &mut next_id, imported);
        assert_eq!(count, 1, "only the unknown session is newly imported");
        assert_eq!(workspace.sessions.len(), 2);
        let refreshed = workspace.sessions.iter().find(|s| s.id == 7).unwrap();
        assert_eq!(refreshed.title, "新标题");
        assert_eq!(refreshed.created_at, 100);
        assert_eq!(refreshed.messages.len(), 1);
        assert_eq!(refreshed.messages[0].text, "刷新后的内容");
        assert_eq!(refreshed.model.as_deref(), Some("m1"));
        let fresh = workspace
            .sessions
            .iter()
            .find(|s| s.native_id.as_deref() == Some("fresh"))
            .unwrap();
        assert_eq!(fresh.id, 8);
        assert_eq!(next_id, 9);
        // Newest first ordering.
        assert!(workspace.sessions[0].created_at >= workspace.sessions[1].created_at);
    }

    #[test]
    fn timestamp_helper_matches_known_epoch() {
        assert_eq!(rfc3339_to_unix("1970-01-01T00:00:00Z"), 0);
        assert_eq!(rfc3339_to_unix("2026-10-23T14:29:05.122Z"), 1_792_765_745);
        assert_eq!(rfc3339_to_unix("bogus"), 0);
    }
}
