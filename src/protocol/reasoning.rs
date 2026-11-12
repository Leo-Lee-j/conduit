//! Provider reasoning chunks form one visible activity between replies/tools.
use super::Event;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

#[derive(Default)]
pub(super) struct CodexReasoning {
    items: HashMap<String, CodexReasoningItem>,
}

#[derive(Default)]
struct CodexReasoningItem {
    content: BTreeMap<u64, String>,
    summaries: BTreeMap<u64, String>,
    started: Option<Instant>,
    visible: Option<(bool, u64)>,
}

impl CodexReasoning {
    pub(super) fn start(&mut self, item: &Value) {
        let id = super::text(item, "id");
        if !id.is_empty() {
            self.items
                .entry(id.to_owned())
                .or_default()
                .started
                .get_or_insert_with(Instant::now);
        }
    }

    pub(super) fn summary_part(&mut self, params: &Value) {
        let id = super::text(params, "itemId");
        if !id.is_empty() {
            self.items
                .entry(id.to_owned())
                .or_default()
                .summaries
                .entry(params["summaryIndex"].as_u64().unwrap_or_default())
                .or_default();
        }
    }

    pub(super) fn delta(&mut self, params: &Value, content: bool, events: &mut Vec<Event>) {
        let id = super::text(params, "itemId");
        let delta = super::text(params, "delta");
        if id.is_empty() || delta.is_empty() {
            return;
        }
        let summary_index = params[if content {
            "contentIndex"
        } else {
            "summaryIndex"
        }]
        .as_u64()
        .unwrap_or_default();
        let item = self.items.entry(id.to_owned()).or_default();
        let parts = if content {
            &mut item.content
        } else {
            &mut item.summaries
        };
        parts.entry(summary_index).or_default().push_str(delta);
        // Keep the two published sources separate. A summary delta must never
        // append to a raw reasoning block already being displayed.
        if !content && !item.content.is_empty() {
            return;
        }
        let replace = item.visible.map_or(content, |(was_content, last)| {
            was_content != content || summary_index < last
        });
        let parts = if content {
            &item.content
        } else {
            &item.summaries
        };
        // Usually append one delta. An update to an earlier part needs an
        // ordered replacement, rather than another paragraph at the end.
        let (text, summary_index) = if replace {
            let mut visible = parts.iter().filter(|(_, text)| !text.is_empty());
            (
                visible
                    .clone()
                    .map(|(_, text)| text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                visible
                    .next_back()
                    .map_or(summary_index, |(index, _)| *index),
            )
        } else {
            (delta.to_owned(), summary_index)
        };
        item.visible = Some((content, summary_index));
        events.push(Event::ReasoningSummary {
            key: format!("{id}-summary"),
            text,
            summary_index,
            snapshot: false,
            summary: !content,
            elapsed_ms: item.started.map(elapsed_ms),
            replace,
        });
    }

    pub(super) fn complete(&mut self, item: &Value, events: &mut Vec<Event>) {
        let id = super::text(item, "id");
        if id.is_empty() {
            return;
        }
        let elapsed_ms = self
            .items
            .remove(id)
            .and_then(|item| item.started)
            .map(elapsed_ms);
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
        let summary = content.trim().is_empty();
        // App-server documents item/completed as authoritative, even when it
        // shortens or clears the streamed text. Only published fields are read.
        let text = if summary { parts("summary") } else { content };
        events.push(Event::ReasoningSummary {
            key: format!("{id}-summary"),
            text,
            summary_index: 0,
            snapshot: true,
            summary,
            elapsed_ms,
            replace: true,
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
        reasoning.items.entry("r1".into()).or_default().started = Some(started);
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
        assert!(reasoning.items.is_empty());
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

    #[test]
    fn codex_reasoning_completion_replaces_shortened_and_corrected_sections() {
        let mut reasoning = CodexReasoning::default();
        let mut events = Vec::new();
        for (index, text) in [
            "**Old title**\n\nOriginal details.",
            "**Second title**\n\nKeep these streamed details.",
        ]
        .into_iter()
        .enumerate()
        {
            reasoning.delta(
                &json!({"itemId":"r1","summaryIndex":index,"delta":text}),
                false,
                &mut events,
            );
        }
        reasoning.complete(
            &json!({"id":"r1","summary":[
                "**Corrected title**\n\nCorrected details.",
                "**Second title**",
                "**Final section**"
            ]}),
            &mut events,
        );
        assert!(
            matches!(events.last(), Some(Event::ReasoningSummary { text, snapshot: true, .. })
            if text == "**Corrected title**\n\nCorrected details.\n\n**Second title**\n\n**Final section**")
        );
        assert!(reasoning.items.is_empty());
    }

    #[test]
    fn thought_message_ids_end_segments_without_replacing_their_body() {
        let mut stream = ThoughtStream::default();
        let mut events = vec![];
        assert!(stream.push_with_id("acp", Some("one"), "First", &mut events));
        assert!(!stream.push_with_id("acp", Some("one"), " thought", &mut events));
        assert!(!stream.push_with_id("acp", None, " continues", &mut events));
        // Even an empty first chunk can announce a new message boundary.
        assert!(!stream.push_with_id("acp", Some("two"), "", &mut events));
        assert!(matches!(events.last(), Some(Event::ReasoningSummary {
            key, text, snapshot:true, replace:false, ..
        }) if key == "acp-thought-0" && text.is_empty()));
        assert!(stream.push_with_id("acp", Some("two"), "Second", &mut events));
        assert!(matches!(events.last(), Some(Event::ReasoningSummary {
            key, text, snapshot:false, replace:false, ..
        }) if key == "acp-thought-1" && text == "Second"));
        stream.finish(&mut events);
        assert!(matches!(events.last(), Some(Event::ReasoningSummary {
            key, text, snapshot:true, replace:false, ..
        }) if key == "acp-thought-1" && text.is_empty()));
        let count = events.len();
        stream.finish(&mut events);
        assert_eq!(events.len(), count);
        // Callers without message IDs retain the original segment behavior.
        assert!(stream.push("acp", "Third", &mut events));
        assert!(!stream.push("acp", " thought", &mut events));
    }
}

#[derive(Default)]
pub(crate) struct ThoughtStream {
    next: u64,
    key: Option<String>,
    message_id: Option<String>,
}
impl ThoughtStream {
    /// Returns true when this chunk starts a new thought segment.
    pub(crate) fn push(&mut self, prefix: &str, text: &str, events: &mut Vec<Event>) -> bool {
        self.push_with_id(prefix, None, text, events)
    }
    pub(crate) fn push_with_id(
        &mut self,
        prefix: &str,
        message_id: Option<&str>,
        text: &str,
        events: &mut Vec<Event>,
    ) -> bool {
        if let Some(id) = message_id {
            if self.message_id.as_deref().is_some_and(|old| old != id) {
                self.finish(events);
            }
            self.message_id = Some(id.into());
        }
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
        self.message_id = None;
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
