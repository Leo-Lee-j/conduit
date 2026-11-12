//! Provider events applied to the persisted conversation model.

use super::*;

pub(super) use crate::failure::{
    failure_guidance, failure_repeats, failure_summary, failure_title, same_failure,
};
use crate::failure::{is_engine_error_text, record_failure};

impl MessageIndex {
    pub(super) fn reasoning_elapsed(&self, key: &str, now: Instant) -> Option<Duration> {
        self.reasoning
            .as_ref()
            .filter(|timer| timer.key == key)
            .map(|timer| timer.elapsed(now))
    }

    pub(super) fn stop_reasoning(&mut self, now: Instant) {
        if let Some(timer) = &mut self.reasoning {
            timer.stopped.get_or_insert(now);
        }
    }

    /// Measure each visible thinking phase, excluding tools, replies and cleanup.
    /// Completion-only/imported summaries have no observed start, so no duration.
    pub(super) fn observe_reasoning(
        &mut self,
        session: &mut SessionRecord,
        prefix: &str,
        event: &Event,
        now: Instant,
    ) -> Option<usize> {
        let next = match event {
            Event::ReasoningSummary {
                key,
                text,
                snapshot: false,
                summary: false,
                ..
            } if !text.trim().is_empty() => Some(format!("{prefix}-{key}")),
            _ => None,
        };
        let closes = match event {
            Event::ReasoningSummary {
                key,
                snapshot: true,
                ..
            } => self
                .reasoning
                .as_ref()
                .is_some_and(|timer| timer.key == format!("{prefix}-{key}")),
            Event::ReasoningSummary { .. } => next.as_ref().is_some_and(|key| {
                self.reasoning
                    .as_ref()
                    .is_some_and(|timer| &timer.key != key)
            }),
            Event::Delta { text, .. } | Event::Text { text, .. } => !text.trim().is_empty(),
            Event::Tool { .. }
            | Event::ToolInput { .. }
            | Event::Approval(_)
            | Event::Finished { .. }
            | Event::CleanupBlocked(_)
            | Event::ErrorBanner(_) => true,
            _ => false,
        };
        let changed = if closes {
            self.reasoning.take().and_then(|timer| {
                let position = self.find(&session.messages, &timer.key)?;
                let item = &mut session.messages[position];
                if item.kind != ItemKind::Reasoning {
                    return None;
                }
                item.elapsed_ms = Some(timer.elapsed(now).as_millis().min(u64::MAX as u128) as u64);
                Some(position)
            })
        } else {
            None
        };
        if let Some(key) = next
            && self.reasoning.is_none()
        {
            let previous = self
                .find(&session.messages, &key)
                .and_then(|position| session.messages[position].elapsed_ms)
                .unwrap_or_default();
            self.reasoning = Some(super::state::ReasoningTimer {
                key,
                started: now,
                previous: Duration::from_millis(match event {
                    Event::ReasoningSummary {
                        elapsed_ms: Some(elapsed),
                        ..
                    } => *elapsed,
                    _ => previous,
                }),
                stopped: None,
            });
        }
        changed
    }
}

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
        if matches!(item.kind, ItemKind::Tool | ItemKind::Reasoning)
            && item.key.starts_with(&prefix)
            && matches!(
                item.status,
                crate::message_status::ToolStatus::Running
                    | crate::message_status::ToolStatus::Output
            )
        {
            item.status = if item.kind == ItemKind::Reasoning
                && status == crate::message_status::ToolStatus::MissingResult.as_str()
            {
                crate::message_status::ToolStatus::Completed
            } else {
                status.into()
            };
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
        replace,
        summary,
        elapsed_ms,
    } = event
    {
        let key = format!("{prefix}-{key}");
        let status = if snapshot {
            crate::message_status::ToolStatus::Completed
        } else {
            crate::message_status::ToolStatus::Running
        };
        if let Some(position) = index.find(&session.messages, &key) {
            let item = &mut session.messages[position];
            // Preserve legacy markers that only repeat a prefix. An explicit
            // replacement (including every Codex final item) is authoritative.
            let keep_streamed = !replace
                && item.reasoning_summary == Some(summary)
                && item.text.starts_with(text.trim_end());
            item.status = status;
            item.reasoning_summary = Some(summary);
            if snapshot {
                if elapsed_ms.is_some() {
                    item.elapsed_ms = elapsed_ms;
                }
                // Codex final items replace even with empty text. Other engines
                // can send a completion marker without repeating their body.
                if replace || (!text.trim().is_empty() && !keep_streamed) {
                    item.text = text;
                }
                item.detail.clear();
            } else {
                if replace {
                    item.text.clear();
                    item.detail.clear();
                }
                let part = summary_index.to_string();
                if !item.text.is_empty() && item.detail != part {
                    // Summary parts are independent Markdown paragraphs.
                    item.text.push_str("\n\n");
                }
                item.detail = part;
                item.text.push_str(&text);
            }
            return Some(position);
        }
        if text.trim().is_empty() {
            return None;
        }
        session
            .messages
            .push(ChatItem::from(crate::domain::ChatItemData {
                reasoning_summary: Some(summary),
                elapsed_ms: if snapshot { elapsed_ms } else { None },
                key,
                created_at: unix_now(),
                kind: ItemKind::Reasoning,
                text,
                input: String::new(),
                detail: if snapshot {
                    String::new()
                } else {
                    summary_index.to_string()
                },
                status,
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
                    reasoning_summary: None,
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
                    reasoning_summary: None,
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
