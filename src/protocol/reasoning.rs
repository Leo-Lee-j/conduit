//! Consecutive ACP thought chunks form one visible activity between replies/tools.
use super::Event;

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
            });
        }
    }
}
