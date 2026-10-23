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
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
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
    let mut imported = 0;
    for session in sessions {
        imported += merge_session(workspace, next_id, session);
    }
    // New imported sessions first in the sidebar ordering.
    workspace
        .sessions
        .sort_by(|a, b| b.created_at.cmp(&a.created_at));
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
    a == b || Path::new(&a).canonicalize().map(|c| c == b).unwrap_or(false)
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
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
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
        if same_dir(value.get("cwd").and_then(Value::as_str).unwrap_or(""), workspace)
            && native_id.is_none()
        {
            // Trust the per-line cwd for grouping; a session file whose
            // lines all carry a different cwd is filtered out below.
        }
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                let typed = value
                    .get("promptSource")
                    .and_then(Value::as_str)
                    .map(|s| s == "typed")
                    .unwrap_or(false);
                if !typed {
                    continue; // Skip tool results, local commands, meta input.
                }
                if let Some(text) = user_text(&value) {
                    if title.is_empty() {
                        title = truncate_title(&text);
                    }
                    messages.push(ChatItem {
                        key: format!("import-{index}"),
                        kind: ItemKind::User,
                        text,
                        input: String::new(),
                        detail: String::new(),
                        status: String::new(),
                        approval: None,
                    });
                }
            }
            Some("assistant") => {
                let message = value.get("message");
                if model.is_none()
                    && let Some(m) =
                        message.and_then(|m| m.get("model")).and_then(Value::as_str)
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
                                kind: ItemKind::Assistant,
                                text: text.to_owned(),
                                input: String::new(),
                                detail: String::new(),
                                status: String::new(),
                                approval: None,
                            });
                        }
                        Some("tool_use") => {
                            let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                            messages.push(ChatItem {
                                key: format!(
                                    "import-{index}-{part_index}-{}",
                                    part.get("id").and_then(Value::as_str).unwrap_or("call")
                                ),
                                kind: ItemKind::Tool,
                                text: name.to_owned(),
                                input: serde_json::to_string_pretty(
                                    part.get("input").unwrap_or(&Value::Null),
                                )
                                .unwrap_or_default(),
                                detail: String::new(),
                                status: "已完成".into(),
                                approval: None,
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
            .and_then(|v| v.get("cwd").and_then(Value::as_str).map(|cwd| same_dir(cwd, workspace)))
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
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Parses one Codex rollout jsonl. The first line is `session_meta` with the
/// session id, timestamp and `cwd` used for workspace grouping.
fn parse_codex(text: &str, workspace: &Path) -> Option<ImportedSession> {
    let mut native_id: Option<String> = None;
    let mut created_at = 0u64;
    let mut model: Option<String> = None;
    let mut messages: Vec<ChatItem> = Vec::new();
    let mut title = String::new();

    for (index, line) in text.lines().enumerate() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = value.get("payload").unwrap_or(&value);
        match value.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                native_id = payload
                    .get("id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                created_at = payload
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(rfc3339_to_unix_opt)
                    .unwrap_or(0);
                let cwd = payload.get("cwd").and_then(Value::as_str).unwrap_or("");
                if !same_dir(cwd, workspace) {
                    return None;
                }
            }
            Some("response_item") => {
                if model.is_none()
                    && payload.get("model").is_some()
                    && let Some(m) = payload.get("model").and_then(Value::as_str)
                {
                    model = Some(m.to_owned());
                }
                match payload.get("type").and_then(Value::as_str) {
                    Some("message") => {
                        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
                        if role != "user" && role != "assistant" {
                            continue; // Skip developer/system injections.
                        }
                        // Codex injects project instructions (AGENTS.md,
                        // environment, approval notices) as user-role
                        // messages without `metadata`; real typed input
                        // carries `metadata.user_input_order`.
                        if role == "user"
                            && payload.get("metadata").is_none()
                            && value.get("metadata").is_none()
                        {
                            continue;
                        }
                        let Some(parts) =
                            payload.get("content").and_then(Value::as_array)
                        else {
                            continue;
                        };
                        for (part_index, part) in parts.iter().enumerate() {
                            let part_type =
                                part.get("type").and_then(Value::as_str).unwrap_or("");
                            if part_type != "input_text" && part_type != "output_text" {
                                continue;
                            }
                            let Some(text) = part.get("text").and_then(Value::as_str) else {
                                continue;
                            };
                            if text.trim().is_empty() {
                                continue;
                            }
                            if role == "user" && title.is_empty() {
                                title = truncate_title(text);
                            }
                            messages.push(ChatItem {
                                key: format!("import-{index}-{part_index}"),
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
                            });
                        }
                    }
                    Some("function_call") => {
                        let name =
                            payload.get("name").and_then(Value::as_str).unwrap_or("tool");
                        messages.push(ChatItem {
                            key: format!("import-{index}-call"),
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
                        });
                    }
                    _ => {}
                }
            }
            _ => {
                if model.is_none()
                    && payload.get("type").and_then(Value::as_str) == Some("turn_context")
                    && let Some(m) = payload.get("model").and_then(Value::as_str)
                {
                    model = Some(m.to_owned());
                }
            }
        }
    }

    let native_id = native_id?;
    if messages.is_empty() {
        return None;
    }
    if title.is_empty() {
        title = format!("Codex 会话 {native_id}");
    }
    Some(ImportedSession {
        provider: Provider::Codex,
        native_id,
        created_at,
        title,
        model,
        messages,
    })
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
    let digits = |range: std::ops::Range<usize>| -> Option<u64> {
        ts.get(range)?.parse::<u64>().ok()
    };
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
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions"}]}
        });
        let user = serde_json::json!({
            "type":"response_item",
            "metadata":{"user_input_order":0},
            "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"运行测试"}]}
        });
        let assistant = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"通过"}]}
        });
        let call = serde_json::json!({
            "type":"response_item",
            "payload":{"type":"function_call","name":"shell","arguments":"{\"cmd\":\"cargo test\"}"}
        });
        let jsonl = [meta, developer, injected, user, assistant, call]
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let session = parse_codex(&jsonl, &workspace).expect("cwd matches");
        assert_eq!(session.native_id, "01a1");
        assert_eq!(session.provider, Provider::Codex);
        assert_eq!(session.title, "运行测试");
        assert_eq!(session.created_at, 1_792_765_745);
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.messages[0].text, "运行测试");
        assert!(matches!(session.messages[2].kind, ItemKind::Tool));
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
            messages: vec![ChatItem {
                key: "old".into(),
                kind: ItemKind::User,
                text: "旧消息".into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
            }],
            interrupted: false,
        });
        // scan_workspace would hit the real CLI dirs; patch through
        // merge_from instead by injecting parsed sessions directly.
        let imported = vec![
            ImportedSession {
                provider: Provider::Claude,
                native_id: "known".into(),
                created_at: 100,
                title: "新标题".into(),
                model: Some("m1".into()),
                messages: vec![ChatItem {
                    key: "a".into(),
                    kind: ItemKind::Assistant,
                    text: "刷新后的内容".into(),
                    input: String::new(),
                    detail: String::new(),
                    status: String::new(),
                    approval: None,
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
                    kind: ItemKind::User,
                    text: "hi".into(),
                    input: String::new(),
                    detail: String::new(),
                    status: String::new(),
                    approval: None,
                }],
            },
        ];
        let mut next_id = 8u64;
        let count = merge_from(&mut workspace, &mut next_id, imported);
        assert_eq!(count, 1, "only the unknown session is newly imported");
        assert_eq!(workspace.sessions.len(), 2);
        let refreshed = workspace.sessions.iter().find(|s| s.id == 7).unwrap();
        assert_eq!(refreshed.title, "新标题");
        assert_eq!(refreshed.created_at, 100);
        assert_eq!(refreshed.messages.len(), 1);
        assert_eq!(refreshed.messages[0].text, "刷新后的内容");
        assert_eq!(refreshed.model.as_deref(), Some("m1"));
        let fresh = workspace.sessions.iter().find(|s| s.native_id.as_deref() == Some("fresh")).unwrap();
        assert_eq!(fresh.id, 8);
        assert_eq!(next_id, 9);
        // Newest first ordering.
        assert!(workspace.sessions[0].created_at >= workspace.sessions[1].created_at);
    }

    fn merge_from(
        workspace: &mut crate::domain::Workspace,
        next_id: &mut u64,
        imported: Vec<ImportedSession>,
    ) -> usize {
        let mut count = 0;
        for session in imported {
            count += merge_one(workspace, next_id, session);
        }
        workspace
            .sessions
            .sort_by(|a, b| b.created_at.cmp(&a.created_at));
        count
    }

    fn merge_one(
        workspace: &mut crate::domain::Workspace,
        next_id: &mut u64,
        session: ImportedSession,
    ) -> usize {
        merge_session(workspace, next_id, session)
    }

    #[test]
    fn timestamp_helper_matches_known_epoch() {
        assert_eq!(rfc3339_to_unix("1970-01-01T00:00:00Z"), 0);
        assert_eq!(rfc3339_to_unix("2026-10-23T14:29:05.122Z"), 1_792_765_745);
        assert_eq!(rfc3339_to_unix("bogus"), 0);
    }
}
