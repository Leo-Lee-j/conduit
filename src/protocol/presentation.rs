//! Bounded presentation of optional protocol features. Never retain opaque
//! payloads, credentials, or encoded media in compatibility notices.
use super::{Event, Step};
use serde_json::Value;
use std::collections::BTreeSet;

const DIAGNOSTIC_LOG_LIMIT: u64 = 256 * 1024;

fn log_ignored(path: &std::path::Path, kind: &str) -> std::io::Result<()> {
    conduit::diagnostics::append_bounded(
        path,
        &format!("ignored protocol notification {kind}"),
        DIAGNOSTIC_LOG_LIMIT,
    )
}

#[derive(Default)]
pub(crate) struct Presentation {
    reported: BTreeSet<String>,
    ignored: BTreeSet<String>,
}

pub(crate) fn bounded(text: &str) -> String {
    text.chars().take(2048).collect()
}

impl Presentation {
    /// A notification expects no reply and CLIs add new ones in every release,
    /// so an unrecognized one is a compatibility detail for developers and
    /// never transcript content. Use `unsupported` only when the user loses
    /// something: dropped reply content or a request Conduit had to refuse.
    pub(crate) fn ignored(&mut self, kind: &str) {
        let kind: String = kind
            .chars()
            .take(96)
            .map(|ch| if ch.is_control() { '_' } else { ch })
            .collect();
        if self.ignored.len() < 64 && self.ignored.insert(kind.clone()) {
            // Record only the protocol name, never the discarded payload. The
            // bounded file also works in Windows release builds without stderr.
            let _ = log_ignored(
                &crate::store::Store::path().with_file_name("protocol.log"),
                &kind,
            );
            if cfg!(debug_assertions) {
                eprintln!("conduit: ignored protocol notification {kind}");
            }
        }
    }

    pub(crate) fn unsupported(&mut self, kind: &str, step: &mut Step) {
        let kind: String = kind.chars().take(96).collect();
        self.notice(
            &format!("unsupported:{kind}"),
            conduit::i18n::format("protocol.unsupported_message", &[("kind", kind.clone())]),
            step,
        );
    }

    pub(crate) fn notice(&mut self, key: &str, message: String, step: &mut Step) {
        if self.reported.len() < 32 && self.reported.insert(bounded(key)) {
            step.events.push(Event::Notice(bounded(&message)));
        }
    }

    pub(crate) fn session_notice(&mut self, key: &str, message: String, step: &mut Step) {
        let key = bounded(key);
        let message = bounded(&message);
        if self.reported.len() < 32 && self.reported.insert(format!("{key}:{message}")) {
            step.events.push(Event::SessionNotice { key, message });
        }
    }

    /// ACP resources are readable text or a safe URI; unsupported media gets a
    /// visible compatibility notice without copying its base64 body.
    pub(crate) fn content(&mut self, content: &Value, step: &mut Step) -> Option<String> {
        let kind = content["type"].as_str().unwrap_or("unknown-content");
        match kind {
            "text" => match content["text"].as_str() {
                Some(text) => Some(text.to_owned()),
                None => {
                    self.unsupported("text/malformed", step);
                    None
                }
            },
            "resource" => {
                let resource = &content["resource"];
                if let Some(text) = resource["text"].as_str() {
                    Some(format!("\n\n{text}\n\n"))
                } else {
                    self.unsupported("resource/blob", step);
                    None
                }
            }
            "resource_link" => {
                let uri = content["uri"].as_str().unwrap_or_default();
                if let Ok(url) = url::Url::parse(uri)
                    && matches!(url.scheme(), "https" | "http" | "file")
                    && url.username().is_empty()
                    && url.password().is_none()
                {
                    Some(format!(
                        "\n\n{}\n{}\n\n",
                        bounded(content["name"].as_str().unwrap_or("Resource")),
                        bounded(uri)
                    ))
                } else {
                    self.unsupported("resource_link/uri", step);
                    None
                }
            }
            _ => {
                self.unsupported(kind, step);
                None
            }
        }
    }

    pub(crate) fn activity(&self, key: &str, args: &[(&str, String)], step: &mut Step) {
        step.events
            .push(Event::Activity(bounded(&conduit::i18n::format(key, args))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diagnostic_log_stays_bounded_and_has_no_transcript_events() {
        let path = crate::store::Store::path().with_file_name("protocol-log-test.log");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![b'x'; DIAGNOSTIC_LOG_LIMIT as usize]).unwrap();
        log_ignored(&path, "Codex/futureNotice").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("ignored protocol notification Codex/futureNotice"));
        assert!(contents.len() < 256);
        std::fs::remove_file(&path).unwrap();
        let mut presentation = Presentation::default();
        presentation.ignored("Codex/futureNotice");
        presentation.ignored("Codex/futureNotice");
        assert_eq!(presentation.ignored.len(), 1);
    }

    #[test]
    fn readable_resources_survive_and_opaque_or_unsafe_payloads_never_leak() {
        let mut presentation = Presentation::default();
        let mut step = Step::new();
        assert_eq!(
            presentation
                .content(
                    &json!({"type":"resource","resource":{"text":"readable"}}),
                    &mut step
                )
                .as_deref(),
            Some("\n\nreadable\n\n")
        );
        for content in [
            json!({"type":"image","data":"SECRET_BASE64"}),
            json!({"type":"resource","resource":{"blob":"SECRET_BASE64"}}),
            json!({"type":"resource_link","uri":"https://user:SECRET_BASE64@example.com"}),
            json!({"type":"resource_link","uri":"javascript:SECRET_BASE64"}),
            json!({"type":"text"}),
        ] {
            assert!(presentation.content(&content, &mut step).is_none());
        }
        assert!(step.events.iter().all(|event| matches!(event, Event::Notice(message) if !message.contains("SECRET_BASE64") && message.chars().count() <= 2048)));
        let before = step.events.len();
        presentation.content(&json!({"type":"image","data":"REPEATED"}), &mut step);
        assert_eq!(step.events.len(), before);
    }
}
