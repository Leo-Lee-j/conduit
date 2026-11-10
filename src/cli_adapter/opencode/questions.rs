//! ACP does not forward OpenCode's native question events. Poll the same
//! authenticated loopback server for the active session's pending requests.
use crate::protocol::Approval;
use anyhow::Context;
use anyhow::{Result, bail};
use base64::Engine as _;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

pub(super) struct QuestionBridge {
    agent: ureq::Agent,
    base: String,
    authorization: String,
    directory: String,
    known: HashSet<String>,
}

impl QuestionBridge {
    pub(super) fn new(port: u16, password: &str, cwd: &std::path::Path) -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(2)))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            base: format!("http://127.0.0.1:{port}"),
            authorization: format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("conduit:{password}"))
            ),
            directory: cwd.to_string_lossy().into_owned(),
            known: Default::default(),
        }
    }

    pub(super) fn commands(&self) -> Result<Value> {
        let mut response = self
            .agent
            .get(format!("{}/command", self.base))
            .header("Authorization", &self.authorization)
            .query("directory", &self.directory)
            .call()?;
        Ok(response
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_json()?)
    }

    pub(super) fn poll(&mut self, session: &str) -> Result<Vec<Value>> {
        let mut response = self
            .agent
            .get(format!("{}/question", self.base))
            .header("Authorization", &self.authorization)
            .query("directory", &self.directory)
            .call()
            .context(conduit::i18n::text_static(
                "cli_adapter.opencode.questions.unable_to_read_pending_opencode_questions",
            ))?;
        let requests: Vec<Value> = response
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_json()
            .context(conduit::i18n::text_static(
                "cli_adapter.opencode.questions.invalid_opencode_question_list",
            ))?;
        let mut current = HashSet::new();
        let mut messages = vec![];
        for request in requests {
            if request["sessionID"].as_str() != Some(session) {
                continue;
            }
            let id = request["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .context(conduit::i18n::text_static(
                    "cli_adapter.opencode.questions.opencode_question_is_missing_a_request_id",
                ))?
                .to_owned();
            if !current.insert(id.clone()) {
                bail!(conduit::i18n::text(
                    "cli_adapter.opencode.questions.opencode_question_list_contains_duplicate_request_ids"
                ));
            }
            if !self.known.contains(&id) {
                messages.push(
                    serde_json::json!({"method":"conduit/question","id":id,"params":request}),
                );
            }
        }
        for id in self.known.difference(&current) {
            messages
                .push(serde_json::json!({"method":"conduit/questionCancelled","params":{"id":id}}));
        }
        self.known = current;
        Ok(messages)
    }

    pub(super) fn respond(&self, approval: &Approval, response: Value) -> Result<()> {
        let id = approval
            .wire_id
            .as_str()
            .context(conduit::i18n::text_static(
                "cli_adapter.opencode.questions.invalid_opencode_question_id",
            ))?;
        let id: String = id
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let action = if response["reject"] == true {
            "reject"
        } else {
            "reply"
        };
        self.agent
            .post(format!("{}/question/{id}/{action}", self.base))
            .header("Authorization", &self.authorization)
            .query("directory", &self.directory)
            .send_json(response)
            .context(conduit::i18n::text_static(
                "cli_adapter.opencode.questions.failed_to_submit_opencode_answers",
            ))?;
        Ok(())
    }
}
