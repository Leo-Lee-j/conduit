//! Provider events applied to the persisted conversation model.

use super::*;

/// Whether a failure notice repeats text an earlier assistant/tool message
/// already shows. CLI wrappers often echo the same provider error in both the
/// assistant output and the task result, and showing it twice reads as a
/// duplicated reply.
/// Turn a raw CLI failure blob into a one-line banner summary. The runtime
/// reports some failures as pretty-printed JSON (e.g. a Codex `turn.error`
/// object); those get parsed so the banner shows the cause, not the payload.
pub(super) fn friendly_failure(error: &str) -> String {
    let trimmed = error.trim();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return trimmed.to_string();
    };
    let get = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| {
            json.get(*key)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .filter(|value| !value.is_empty())
        })
    };
    let message = get(&["message", "error"]);
    let detail = json
        .get("codexErrorInfo")
        .and_then(|info| info.get("responseStreamDisconnected"))
        .map(|disconnected| {
            let status = disconnected["httpStatusCode"].as_i64();
            let detail = disconnected["additionalDetails"].as_str().unwrap_or("");
            let brief = detail.split(['\n', ';']).next().unwrap_or(detail).trim();
            let brief = if brief.chars().count() > 120 {
                format!("{}…", brief.chars().take(120).collect::<String>())
            } else {
                brief.to_string()
            };
            match (status, brief.is_empty()) {
                (Some(code), false) => format!("连接中断（HTTP {code}）：{brief}"),
                (Some(code), true) => format!("连接中断（HTTP {code}）"),
                (None, false) => format!("连接中断：{brief}"),
                (None, true) => "连接中断".to_string(),
            }
        });
    detail.or(message).unwrap_or_else(|| trimmed.to_string())
}
pub(super) fn same_failure(message: &str, error: &str) -> bool {
    let message = message.trim();
    let error = error.trim();
    if message.is_empty() || error.is_empty() {
        return false;
    }
    message == error || message.contains(error) || error.contains(message)
}

/// Keep long diagnostics out of the transcript's default reading flow.
pub(super) fn failure_summary(error: &str) -> String {
    let error = error
        .trim()
        .strip_prefix("执行失败：")
        .unwrap_or(error.trim());
    let friendly = friendly_failure(error);
    let line = friendly
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("CLI 执行失败，请查看详情。");
    if line.chars().count() > 160 {
        format!("{}…", line.chars().take(160).collect::<String>())
    } else {
        line.to_string()
    }
}

pub(super) fn finish_tool_cards(session: &mut SessionRecord, prefix: &str, status: &str) {
    let prefix = format!("{prefix}-");
    for item in &mut session.messages {
        if item.kind == ItemKind::Tool
            && item.key.starts_with(&prefix)
            && let Some(approval) = item.approval.as_mut()
            && approval.status == "等待中"
        {
            approval.status = "已取消".into();
        }
        if item.kind == ItemKind::Tool
            && item.key.starts_with(&prefix)
            && matches!(item.status.as_str(), "运行中" | "输出中")
        {
            item.status = status.into();
        }
    }
}

pub(super) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) fn message_timestamp(timestamp: u64) -> String {
    if timestamp == 0 {
        return String::new();
    }
    DateTime::from_timestamp(timestamp as i64, 0)
        .map(|time| {
            time.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_default()
}

/// Elapsed minutes, hours, or days since creation, shown on each sidebar row.
pub(super) fn session_age_label(created_at: u64) -> Option<String> {
    if created_at == 0 {
        return None;
    }
    let created = DateTime::from_timestamp(created_at as i64, 0)?;
    let minutes = Local::now()
        .signed_duration_since(created)
        .num_minutes()
        .max(0);
    Some(match minutes {
        minutes if minutes < 60 => format!("{minutes} 分"),
        minutes if minutes < 24 * 60 => format!("{} 时", minutes / 60),
        minutes => format!("{} 日", minutes / (24 * 60)),
    })
}

#[cfg(test)]
pub fn apply_event(session: &mut SessionRecord, prefix: &str, event: Event) {
    apply_event_indexed(session, prefix, event, &mut MessageIndex::default());
}

pub(super) fn attach_turn_model(session: &mut SessionRecord, prefix: &str, model: &str) {
    let turn = format!("{prefix}-");
    for item in &mut session.messages {
        if item.kind == ItemKind::Assistant && item.key.starts_with(&turn) && item.model.is_none() {
            item.model = Some(model.to_string());
        }
    }
}

pub(super) fn apply_event_indexed(
    session: &mut SessionRecord,
    prefix: &str,
    event: Event,
    index: &mut MessageIndex,
) -> Option<usize> {
    if let Event::ReasoningSummary {
        key,
        text,
        summary_index,
        snapshot,
    } = event
    {
        let key = format!("{prefix}-{key}");
        if let Some(position) = index.find(&session.messages, &key) {
            let item = &mut session.messages[position];
            if snapshot {
                item.text = text;
                item.detail.clear();
            } else {
                let part = summary_index.to_string();
                if !item.text.is_empty() && item.detail != part {
                    item.text.push('\n');
                }
                item.detail = part;
                item.text.push_str(&text);
            }
            return Some(position);
        }
        session.messages.push(ChatItem {
            elapsed_ms: None,
            key,
            created_at: unix_now(),
            kind: ItemKind::Reasoning,
            text,
            input: String::new(),
            detail: summary_index.to_string(),
            status: String::new(),
            approval: None,
            model: None,
        });
        return Some(session.messages.len() - 1);
    }
    if let Event::ToolInput { key, input } = event {
        let key = format!("{prefix}-{key}");
        if let Some(position) = index.find(&session.messages, &key) {
            session.messages[position].input = input;
            return Some(position);
        } else {
            session.messages.push(ChatItem {
                elapsed_ms: None,
                key,
                created_at: 0,
                kind: ItemKind::Tool,
                text: "工具调用".into(),
                input,
                detail: String::new(),
                status: "运行中".into(),
                approval: None,
                model: None,
            });
        }
        return Some(session.messages.len() - 1);
    }
    let (key, kind, text, detail, status, delta) = match event {
        Event::Delta { key, text } => (
            key,
            ItemKind::Assistant,
            text,
            String::new(),
            String::new(),
            true,
        ),
        Event::Text { key, text } => (
            key,
            ItemKind::Assistant,
            text,
            String::new(),
            String::new(),
            false,
        ),
        Event::Tool {
            key,
            title,
            detail,
            status,
        } => {
            let delta = status == "输出中";
            (key, ItemKind::Tool, title, detail, status, delta)
        }
        Event::Notice(text) => (
            format!("notice-{}", session.messages.len()),
            ItemKind::Notice,
            text,
            String::new(),
            String::new(),
            false,
        ),
        Event::ErrorBanner(text) => (
            format!("error-{}", session.messages.len()),
            ItemKind::Error,
            text,
            String::new(),
            String::new(),
            false,
        ),
        _ => return None,
    };
    let key = format!("{prefix}-{key}");
    if let Some(position) = index.find(&session.messages, &key) {
        let item = &mut session.messages[position];
        if kind == ItemKind::Assistant {
            if delta {
                item.text.push_str(&text);
            } else {
                item.text = text;
            }
        } else {
            if !text.is_empty() && !delta {
                item.text = text;
            }
            if delta {
                item.detail.push_str(&detail);
            } else if !detail.is_empty() {
                item.detail = detail;
            }
            item.status = status;
        }
        Some(position)
    } else {
        // Guard against a provider resending the same completed block under a
        // new item key; keep one visible copy per consecutive run.
        let duplicate_text = kind == ItemKind::Assistant
            && !text.trim().is_empty()
            && session.messages.last().is_some_and(|last| {
                last.kind == ItemKind::Assistant && last.text.trim() == text.trim()
            });
        let duplicate_notice = kind == ItemKind::Notice
            && session
                .messages
                .last()
                .is_some_and(|last| last.kind == ItemKind::Notice && last.text == text);
        if !duplicate_text && !duplicate_notice {
            session.messages.push(ChatItem {
                elapsed_ms: None,
                key,
                created_at: unix_now(),
                kind,
                text,
                input: String::new(),
                detail,
                status,
                approval: None,
                model: None,
            });
            Some(session.messages.len() - 1)
        } else {
            None
        }
    }
}
