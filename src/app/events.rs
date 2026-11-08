//! Provider events applied to the persisted conversation model.

use super::*;

pub(super) use crate::failure::{
    failure_guidance, failure_repeats, failure_summary, failure_title, same_failure,
};
use crate::failure::{is_engine_error_text, record_failure};

pub(super) fn finish_tool_cards(session: &mut SessionRecord, prefix: &str, status: &str) {
    let prefix = format!("{prefix}-");
    for item in &mut session.messages {
        if item.kind == ItemKind::Tool
            && item.key.starts_with(&prefix)
            && let Some(approval) = item.approval.as_mut()
            && approval.status == crate::message_status::ApprovalStatus::Waiting
        {
            approval.status = "已取消".into();
        }
        if item.kind == ItemKind::Tool
            && item.key.starts_with(&prefix)
            && matches!(
                item.status,
                crate::message_status::ToolStatus::Running
                    | crate::message_status::ToolStatus::Output
            )
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

/// Elapsed time since the last activity, shown on each sidebar row as S/M/H/D.
pub(super) fn session_age_label(activity_at: u64) -> Option<String> {
    if activity_at == 0 {
        return None;
    }
    DateTime::from_timestamp(activity_at as i64, 0)?;
    let seconds = unix_now().saturating_sub(activity_at);
    Some(match seconds {
        seconds if seconds < 60 => format!("{seconds} S"),
        seconds if seconds < 60 * 60 => format!("{} M", seconds / 60),
        seconds if seconds < 24 * 60 * 60 => format!("{} H", seconds / (60 * 60)),
        seconds => format!("{} D", seconds / (24 * 60 * 60)),
    })
}

#[cfg(test)]
pub fn apply_event(session: &mut SessionRecord, prefix: &str, event: Event) {
    apply_event_indexed(session, prefix, event, &mut MessageIndex::default());
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
        session
            .messages
            .push(ChatItem::from(crate::domain::ChatItemData {
                elapsed_ms: None,
                key,
                created_at: unix_now(),
                kind: ItemKind::Reasoning,
                text,
                input: String::new(),
                detail: summary_index.to_string(),
                status: Default::default(),
                approval: None,
                model: None,
            }));
        return Some(session.messages.len() - 1);
    }
    if let Event::ToolInput { key, input } = event {
        let key = format!("{prefix}-{key}");
        if let Some(position) = index.find(&session.messages, &key) {
            session.messages[position].input = input;
            return Some(position);
        } else {
            session
                .messages
                .push(ChatItem::from(crate::domain::ChatItemData {
                    elapsed_ms: None,
                    key,
                    created_at: 0,
                    kind: ItemKind::Tool,
                    text: conduit::i18n::text_static("events.tool_call").into(),
                    input,
                    detail: String::new(),
                    status: "运行中".into(),
                    approval: None,
                    model: None,
                }));
        }
        return Some(session.messages.len() - 1);
    }
    if let Event::ErrorBanner(error) = &event {
        return Some(record_failure(session, prefix, error, false));
    }
    if let Event::Text { key, text } = &event
        && is_engine_error_text(text)
    {
        let full_key = format!("{prefix}-{key}");
        if let Some(position) = index.find(&session.messages, &full_key) {
            let item = &mut session.messages[position];
            item.kind = ItemKind::Error;
            item.text = text.clone();
            item.detail = text.clone();
            return Some(position);
        }
        return Some(record_failure(session, prefix, text, false));
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
            let delta = crate::message_status::ToolStatus::Output == status;
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
            item.status = status.into();
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
            session
                .messages
                .push(ChatItem::from(crate::domain::ChatItemData {
                    elapsed_ms: None,
                    key,
                    created_at: unix_now(),
                    kind,
                    text,
                    input: String::new(),
                    detail,
                    status: status.into(),
                    approval: None,
                    model: None,
                }));
            Some(session.messages.len() - 1)
        } else {
            None
        }
    }
}
