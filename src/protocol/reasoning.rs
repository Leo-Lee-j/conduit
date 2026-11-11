//! Provider reasoning chunks form one visible activity between replies/tools.
use super::Event;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

#[derive(Default)]
pub(super) struct CodexReasoning {
    content: HashSet<String>,
    started: HashMap<String, Instant>,
}

impl CodexReasoning {
    pub(super) fn start(&mut self, item: &Value) {
        let id = super::text(item, "id");
        if !id.is_empty() {
            self.started
                .entry(id.to_owned())
                .or_insert_with(Instant::now);
        }
    }

    pub(super) fn delta(&mut self, params: &Value, content: bool, events: &mut Vec<Event>) {
        let id = super::text(params, "itemId");
        let delta = super::text(params, "delta");
        if delta.is_empty() || (!content && self.content.contains(id)) {
            return;
        }
        let replace = content && self.content.insert(id.to_owned());
        events.push(Event::ReasoningSummary {
            key: format!("{id}-summary"),
            text: delta.into(),
            summary_index: params[if content {
                "contentIndex"
            } else {
                "summaryIndex"
            }]
            .as_u64()
            .unwrap_or_default(),
            snapshot: false,
            summary: !content,
            elapsed_ms: self.started.get(id).map(|start| elapsed_ms(*start)),
            replace,
        });
    }

    pub(super) fn complete(&mut self, item: &Value, events: &mut Vec<Event>) {
        let id = super::text(item, "id");
        let streamed_content = self.content.remove(id);
        let elapsed_ms = self.started.remove(id).map(elapsed_ms);
        let parts = |field: &str| {
            item[field]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|part| {
                    part.as_str()
                        .or_else(|| part.get("text").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let content = parts("content");
        let summary = content.trim().is_empty() && !streamed_content;
        // A summary-only completion must not erase an already streamed body.
        // Encrypted/redacted fields and untyped item-level text are never read.
        let text = if !content.trim().is_empty() {
            content
        } else if streamed_content {
            String::new()
        } else {
            parts("summary")
        };
        events.push(Event::ReasoningSummary {
            key: format!("{id}-summary"),
            text,
            summary_index: 0,
            snapshot: true,
            summary,
            elapsed_ms,
            replace: false,
        });
    }
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn codex_reasoning_times_native_phase_before_late_summary_delivery() {
        let mut reasoning = CodexReasoning::default();
        let started = Instant::now() - Duration::from_millis(3380);
        reasoning.started.insert("r1".into(), started);
        // A repeated start must not reset the native phase.
        reasoning.start(&json!({"id":"r1"}));
        let mut events = Vec::new();
        reasoning.delta(
            &json!({"itemId":"r1","delta":"**Capping output length**"}),
            false,
            &mut events,
        );
        reasoning.complete(
            &json!({"id":"r1","summary":["**Capping output length**"],"content":[]}),
            &mut events,
        );
        for event in &events {
            assert!(
                matches!(event, Event::ReasoningSummary {summary:true,elapsed_ms:Some(ms), ..} if *ms >= 3380)
            );
        }
        assert!(reasoning.started.is_empty());
        events.clear();
        reasoning.complete(
            &json!({"id":"missing-start","summary":["**Searching specific files**"]}),
            &mut events,
        );
        assert!(matches!(
            events.as_slice(),
            [Event::ReasoningSummary {
                summary: true,
                elapsed_ms: None,
                ..
            }]
        ));
    }
}

#[derive(Default)]
pub(crate) struct ThoughtStream {
    next: u64,
    key: Option<String>,
}
impl ThoughtStream {
    /// Returns true when this chunk starts a new thought segment.
    pub(crate) fn push(&mut self, prefix: &str, text: &str, events: &mut Vec<Event>) -> bool {
        if text.is_empty() {
            return false;
        }
        let started = self.key.is_none();
        let key = self.key.get_or_insert_with(|| {
            let key = format!("{prefix}-thought-{}", self.next);
            self.next += 1;
            key
        });
        events.push(Event::ReasoningSummary {
            key: key.clone(),
            text: text.into(),
            summary_index: 0,
            snapshot: false,
            replace: false,
            summary: false,
            elapsed_ms: None,
        });
        started
    }
    pub(crate) fn finish(&mut self, events: &mut Vec<Event>) {
        if let Some(key) = self.key.take() {
            events.push(Event::ReasoningSummary {
                key,
                text: String::new(),
                summary_index: 0,
                snapshot: true,
                replace: false,
                summary: false,
                elapsed_ms: None,
            });
        }
    }
}
