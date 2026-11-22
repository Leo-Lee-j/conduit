//! Transient Claude parameter streams. Missing identity or malformed JSON ends
//! the preview; only complete assistant snapshots become transcript ToolInput.
use super::{Event, text};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const MAX_BYTES: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Input {
    pub path: String,
    pub old: Option<String>,
    pub new: String,
    pub complete: bool,
}
#[derive(Clone, Debug)]
pub(crate) enum Update {
    Started,
    Content(Arc<Input>),
    Ended,
    /// Local queue backpressure: discard every proposal for this request.
    /// Never decoded from a CLI frame or persisted in the transcript.
    Reset,
}
impl Update {
    pub fn bytes(&self) -> usize {
        match self {
            Self::Content(input) => {
                input.path.len() + input.old.as_ref().map_or(0, String::len) + input.new.len()
            }
            _ => 0,
        }
    }
}
#[derive(Default)]
pub(super) struct Streams {
    message: String,
    blocks: HashMap<u64, Stream>,
    seen: HashSet<String>,
}
struct Stream {
    key: String,
    write: bool,
    parser: Fields,
    last: Option<Arc<Input>>,
    ended: bool,
    stopped: bool,
    emitted_at: Instant,
    emitted_bytes: usize,
}
fn event(key: &str, update: Update) -> Event {
    Event::FileProposal {
        key: key.into(),
        update,
    }
}
impl Streams {
    pub fn accept(&mut self, value: &Value) -> Vec<Event> {
        let mut events = Vec::new();
        if !value["parent_tool_use_id"].is_null() {
            return events;
        }
        if value["type"] == "assistant" {
            if let Some(blocks) = value["message"]["content"].as_array() {
                for block in blocks {
                    for stream in self
                        .blocks
                        .values_mut()
                        .filter(|s| s.key == text(block, "id") && !s.ended)
                    {
                        if value["message"]["id"] != self.message
                            || text(block, "name") != if stream.write { "Write" } else { "Edit" }
                            || !stream.parser.done()
                            || serde_json::from_str::<Value>(&stream.parser.wire)
                                .ok()
                                .as_ref()
                                != Some(&block["input"])
                        {
                            stream.ended = true;
                            events.push(event(&stream.key, Update::Ended));
                        }
                    }
                }
            }
            return events;
        }
        if value["type"] == "result" {
            for stream in self.blocks.values() {
                events.push(event(&stream.key, Update::Ended));
            }
            self.blocks.clear();
            return events;
        }
        if value["type"] != "stream_event" {
            return events;
        }
        let frame = &value["event"];
        let kind = text(frame, "type");
        if kind == "message_start" {
            for stream in self.blocks.values() {
                events.push(event(&stream.key, Update::Ended));
            }
            self.blocks.clear();
            let id = text(&frame["message"], "id");
            self.message = if id.len() <= 512 {
                id.into()
            } else {
                String::new()
            };
            return events;
        }
        if self.message.is_empty() {
            return events;
        }
        let Some(index) = frame["index"].as_u64() else {
            return events;
        };
        if kind == "content_block_start" {
            let block = &frame["content_block"];
            if let Some(previous) = self.blocks.get_mut(&index) {
                previous.ended = true;
                events.push(event(&previous.key, Update::Ended));
                return events;
            }
            let key = text(block, "id");
            let name = text(block, "name");
            if text(block, "type") != "tool_use"
                || !matches!(name, "Edit" | "Write")
                || key.is_empty()
                || key.len() > 512
            {
                return events;
            }
            if self.blocks.len() >= 8 || self.seen.len() >= 1024 {
                for stream in self.blocks.values_mut() {
                    stream.ended = true;
                    events.push(event(&stream.key, Update::Ended));
                }
                self.message.clear();
                return events;
            }
            if !self.seen.insert(key.into()) {
                for stream in self.blocks.values_mut().filter(|s| s.key == key) {
                    stream.ended = true;
                }
                events.push(event(key, Update::Ended));
                return events;
            }
            // Nonempty start inputs are snapshots, not parameter streams.
            if !block["input"].as_object().is_some_and(|v| v.is_empty()) {
                return events;
            }
            self.blocks.insert(
                index,
                Stream {
                    key: key.into(),
                    write: name == "Write",
                    parser: Fields::default(),
                    last: None,
                    ended: false,
                    stopped: false,
                    emitted_at: Instant::now(),
                    emitted_bytes: 0,
                },
            );
            events.push(event(key, Update::Started));
            return events;
        }
        let Some(stream) = self.blocks.get_mut(&index).filter(|s| !s.ended) else {
            return events;
        };
        if kind == "content_block_delta" && text(&frame["delta"], "type") == "input_json_delta" {
            let Some(delta) = frame["delta"]["partial_json"].as_str() else {
                stream.ended = true;
                events.push(event(&stream.key, Update::Ended));
                return events;
            };
            if stream.stopped
                || !stream.parser.feed(delta)
                || !stream.parser.valid_fields(stream.write)
            {
                stream.ended = true;
                events.push(event(&stream.key, Update::Ended));
            } else if (stream.last.is_none()
                || stream.parser.done()
                || stream.parser.wire.len() - stream.emitted_bytes >= 1024
                || stream.emitted_at.elapsed() >= Duration::from_millis(33))
                && let Some(input) = stream.parser.input(stream.write)
            {
                let input = Arc::new(input);
                if stream.last.as_ref() != Some(&input) {
                    stream.emitted_at = Instant::now();
                    stream.emitted_bytes = stream.parser.wire.len();
                    stream.last = Some(input.clone());
                    events.push(event(&stream.key, Update::Content(input)));
                }
            }
        } else if kind == "content_block_stop" {
            stream.stopped = true;
            if !stream.parser.done() || stream.parser.input(stream.write).is_none() {
                stream.ended = true;
                events.push(event(&stream.key, Update::Ended));
            }
        }
        events
    }
}

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Open,
    Key,
    Colon,
    Value,
    String,
    Literal,
    Separator,
    Done,
    Invalid,
}
#[derive(Default)]
struct Fields {
    wire: String,
    cursor: usize,
    phase: Phase,
    key: String,
    string: String,
    reading_key: bool,
    after_comma: bool,
    values: HashMap<String, String>,
}
impl Fields {
    fn done(&self) -> bool {
        self.phase == Phase::Done
    }
    fn feed(&mut self, delta: &str) -> bool {
        if self.wire.len() + delta.len() > MAX_BYTES || self.phase == Phase::Invalid {
            return false;
        }
        self.wire.push_str(delta);
        loop {
            let remaining = &self.wire[self.cursor..];
            let Some(ch) = remaining.chars().next() else {
                return true;
            };
            if self.phase != Phase::String && matches!(ch, ' ' | '\r' | '\n' | '\t') {
                self.cursor += 1;
                continue;
            }
            match self.phase {
                Phase::Open if ch == '{' => self.phase = Phase::Key,
                Phase::Key if ch == '"' => {
                    self.reading_key = true;
                    self.string.clear();
                    self.phase = Phase::String;
                }
                Phase::Key if ch == '}' && !self.after_comma => self.phase = Phase::Done,
                Phase::Colon if ch == ':' => self.phase = Phase::Value,
                Phase::Value if ch == '"' && self.key != "replace_all" => {
                    self.reading_key = false;
                    self.string.clear();
                    self.phase = Phase::String;
                }
                Phase::Value if self.key == "replace_all" => {
                    self.phase = Phase::Literal;
                    continue;
                }
                Phase::Literal => {
                    let literal = if remaining.starts_with('t') {
                        "true"
                    } else {
                        "false"
                    };
                    if literal.starts_with(remaining) && remaining.len() < literal.len() {
                        return true;
                    }
                    if !remaining.starts_with(literal) {
                        break;
                    }
                    self.values.insert(self.key.clone(), literal.into());
                    self.cursor += literal.len();
                    self.phase = Phase::Separator;
                    continue;
                }
                Phase::String if ch == '"' => {
                    if self.reading_key {
                        if !matches!(
                            self.string.as_str(),
                            "file_path" | "old_string" | "new_string" | "content" | "replace_all"
                        ) || self.values.contains_key(&self.string)
                        {
                            break;
                        }
                        self.key = std::mem::take(&mut self.string);
                        self.phase = Phase::Colon;
                    } else {
                        self.values
                            .insert(self.key.clone(), std::mem::take(&mut self.string));
                        self.phase = Phase::Separator;
                    }
                }
                Phase::String => {
                    let length = if ch == '\\' {
                        if remaining.len() < 2 {
                            return true;
                        }
                        if remaining.as_bytes()[1] == b'u' {
                            if remaining.len() < 6 {
                                return true;
                            }
                            let Some(hex) = remaining
                                .get(2..6)
                                .and_then(|s| u16::from_str_radix(s, 16).ok())
                            else {
                                break;
                            };
                            if (0xd800..=0xdbff).contains(&hex) {
                                12
                            } else {
                                6
                            }
                        } else {
                            2
                        }
                    } else {
                        ch.len_utf8()
                    };
                    if remaining.len() < length {
                        return true;
                    }
                    let Some(token) = remaining.get(..length) else {
                        break;
                    };
                    if ch == '\\' {
                        let Ok(decoded) = serde_json::from_str::<String>(&format!("\"{token}\""))
                        else {
                            break;
                        };
                        self.string.push_str(&decoded);
                    } else if ch < '\u{20}' {
                        break;
                    } else {
                        self.string.push(ch);
                    }
                    if self.reading_key && self.string.len() > 32 {
                        break;
                    }
                    self.cursor += length;
                    continue;
                }
                Phase::Separator if ch == ',' => {
                    self.phase = Phase::Key;
                    self.after_comma = true;
                }
                Phase::Separator if ch == '}' => self.phase = Phase::Done,
                _ => break,
            }
            self.cursor += ch.len_utf8();
        }
        self.phase = Phase::Invalid;
        false
    }
    fn valid_fields(&self, write: bool) -> bool {
        !self.values.keys().any(|key| {
            if write {
                matches!(key.as_str(), "old_string" | "new_string" | "replace_all")
            } else {
                key == "content"
            }
        }) && self
            .values
            .get("file_path")
            .is_none_or(|path| !path.is_empty() && path.len() <= 32768 && !path.contains('\0'))
    }
    fn input(&self, write: bool) -> Option<Input> {
        let path = self.values.get("file_path")?.clone();
        if path.is_empty() || path.len() > 32768 || path.contains('\0') {
            return None;
        }
        let old = if write {
            None
        } else {
            Some(self.values.get("old_string")?.clone())
        };
        let field = if write { "content" } else { "new_string" };
        let new = if let Some(value) = self.values.get(field) {
            value.clone()
        } else if self.phase == Phase::String && !self.reading_key && self.key == field {
            // A trailing grapheme may still acquire a combining mark or ZWJ.
            let end = self
                .string
                .grapheme_indices(true)
                .next_back()
                .map_or(0, |(i, _)| i);
            self.string[..end].into()
        } else {
            return None;
        };
        if (write
            && (self.values.contains_key("old_string")
                || self.values.contains_key("new_string")
                || self.values.contains_key("replace_all")))
            || (!write && self.values.contains_key("content"))
        {
            return None;
        }
        Some(Input {
            path,
            old,
            new,
            complete: self.done(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn start(streams: &mut Streams, key: &str, index: u64) -> Vec<Event> {
        streams.accept(&json!({"type":"stream_event","event":{"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":key,"name":"Edit","input":{}}}}))
    }
    fn message(streams: &mut Streams, id: &str) {
        streams.accept(
            &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":id}}}),
        );
    }
    fn delta(streams: &mut Streams, index: u64, input: &str) -> Vec<Event> {
        streams.accept(&json!({"type":"stream_event","event":{"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":input}}}))
    }
    #[test]
    fn content_proposal_json_boundaries_unicode_and_budgets() {
        let wire = r#"{"file_path":"note.txt","old_string":"old","new_string":"\u4e2d\u6587 e\u0301 \ud83d\udc68\u200d\ud83d\udc69\u200d\ud83d\udc67\u200d\ud83d\udc66\n\"quoted\"\\","replace_all":false}"#;
        let expected: Value = serde_json::from_str(wire).unwrap();
        for split in 1..=wire.len() {
            let mut parser = Fields::default();
            for chunk in wire.as_bytes().chunks(split) {
                assert!(parser.feed(std::str::from_utf8(chunk).unwrap()));
            }
            let input = parser.input(false).unwrap();
            assert!(input.complete);
            assert_eq!(input.new, expected["new_string"]);
        }
        let mut write = Fields::default();
        assert!(write.feed(r#"{"file_path":"note.txt","content":"new\n"}"#));
        assert_eq!(
            write.input(true).unwrap(),
            Input {
                path: "note.txt".into(),
                old: None,
                new: "new\n".into(),
                complete: true
            }
        );
        assert!(write.input(false).is_none());
        let mut parser = Fields::default();
        assert!(parser.feed(r#"{"file_path":"n","old_string":"o","new_string":"e"#));
        assert_eq!(parser.input(false).unwrap().new, "");
        assert!(parser.feed(r#"\u0301x"#));
        assert_eq!(parser.input(false).unwrap().new, "e\u{301}");
        assert!(parser.feed("\"}"));
        assert_eq!(parser.input(false).unwrap().new, "e\u{301}x");
        for bad in [
            r#"{"file_path":"a","file_path":"b"}"#,
            r#"{"file_path":"a",}"#,
            r#"{"other":[]}"#,
            r#"{"file_path":"\udc00"}"#,
            r#"{"file_path":"\ud800abcdxy"}"#,
            r#"{"file_path":"\q"}"#,
            r#"{}{}"#,
            "{\u{b}}",
        ] {
            assert!(!Fields::default().feed(bad), "{bad}");
        }
        assert!(!Fields::default().feed(&" ".repeat(MAX_BYTES + 1)));
    }
    #[test]
    fn content_proposal_identity_snapshot_and_missing_events_fail_closed() {
        let mut streams = Streams::default();
        assert!(start(&mut streams, "tool", 0).is_empty());
        message(&mut streams, "m");
        assert!(delta(&mut streams, 0, "{}").is_empty());
        assert!(matches!(
            &start(&mut streams, "tool", 0)[0],
            Event::FileProposal {
                update: Update::Started,
                ..
            }
        ));
        let input = r#"{"file_path":"n","old_string":"old","new_string":"new"}"#;
        assert!(matches!(
            &delta(&mut streams, 0, input)[0],
            Event::FileProposal {
                update: Update::Content(_),
                ..
            }
        ));
        let mismatch = streams.accept(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"tool","name":"Edit","input":{"file_path":"another"}}]}}));
        assert!(matches!(
            &mismatch[0],
            Event::FileProposal {
                update: Update::Ended,
                ..
            }
        ));
        assert!(delta(&mut streams, 0, input).is_empty());
        message(&mut streams, "m2");
        assert!(matches!(
            &start(&mut streams, "tool", 0)[0],
            Event::FileProposal {
                update: Update::Ended,
                ..
            }
        ));
        start(&mut streams, "other", 1);
        let stop = streams.accept(
            &json!({"type":"stream_event","event":{"type":"content_block_stop","index":1}}),
        );
        assert!(matches!(
            &stop[0],
            Event::FileProposal {
                update: Update::Ended,
                ..
            }
        ));
        start(&mut streams, "parent", 2);
        let child = streams.accept(&json!({"type":"stream_event","parent_tool_use_id":"parent","event":{"type":"message_start","message":{"id":"child"}}}));
        assert!(child.is_empty());
        assert_eq!(streams.message, "m2");
        assert_eq!(streams.blocks[&2].key, "parent");
    }
    #[test]
    fn content_proposal_native_claude_fixture_precedes_real_disk_writes() {
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/claude-file-arguments.json")).unwrap();
        let mut protocol = super::super::Protocol::new(
            crate::domain::Provider::Claude,
            ".".into(),
            "probe".into(),
            None,
        );
        let mut partials = 0;
        let mut complete = 0;
        let mut tool_inputs = 0;
        for frame in fixture["frames"].as_array().unwrap() {
            let step = protocol.accept(frame["event"].clone());
            for event in step.events {
                match event {
                    Event::FileProposal {
                        key,
                        update: Update::Content(input),
                    } => {
                        if input.complete {
                            complete += 1;
                        } else {
                            partials += 1;
                        }
                        assert_eq!(input.path, "note.txt");
                        if key == "tool_probe_1" {
                            assert_eq!(frame["file"], "before\n");
                        }
                        if key == "tool_probe_2" {
                            assert!(frame["file"].as_str().unwrap().starts_with("after\n"));
                        }
                    }
                    Event::ToolInput { .. } => tool_inputs += 1,
                    _ => (),
                }
            }
        }
        assert!(partials > 0);
        assert_eq!(complete, 2);
        assert_eq!(tool_inputs, 3);
        assert!(
            fixture["frames"].as_array().unwrap().last().unwrap()["file"]
                .as_str()
                .unwrap()
                .starts_with("updated\n")
        );
    }
}
