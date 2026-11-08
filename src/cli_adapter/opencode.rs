//! OpenCode-specific startup and authenticated question transport.
use super::*;
use crate::{
    domain::Provider,
    protocol::{ApprovalKind, Protocol},
};
use anyhow::Context;
mod questions;
use questions::QuestionBridge;

pub fn opencode(request: AdapterRequest) -> Box<dyn CliAdapter> {
    let cwd = request.cwd.clone();
    Box::new(OpenCodeAdapter::new(
        builtin::protocol(Provider::OpenCode, request),
        cwd,
    ))
}

pub(super) struct OpenCodeAdapter {
    core: builtin::StdioAdapter,
    cwd: PathBuf,
    questions: Option<QuestionBridge>,
}

impl OpenCodeAdapter {
    pub fn new(protocol: Protocol, cwd: PathBuf) -> Self {
        Self {
            core: builtin::StdioAdapter::new(protocol),
            cwd,
            questions: None,
        }
    }
}

impl CliAdapter for OpenCodeAdapter {
    fn launch(&mut self) -> anyhow::Result<LaunchSpec> {
        let launch = self.core.launch()?;
        self.questions = self
            .core
            .protocol
            .opencode_question_connection()
            .map(|(port, password)| QuestionBridge::new(port, password, &self.cwd));
        Ok(launch)
    }
    fn accept(&mut self, value: Value) -> Step {
        self.core.accept(value)
    }
    fn startup_pending(&self) -> bool {
        self.core.startup_pending()
    }
    fn cancel_message(&self) -> Option<Value> {
        self.core.cancel_message()
    }
    fn poll(&mut self) -> anyhow::Result<Vec<Value>> {
        match (
            self.core.protocol.opencode_question_session(),
            self.questions.as_mut(),
        ) {
            (Some(session), Some(bridge)) => bridge.poll(session),
            _ => Ok(vec![]),
        }
    }
    fn route_response(
        &mut self,
        approval: &Approval,
        response: Value,
    ) -> anyhow::Result<Option<Value>> {
        if approval.kind == ApprovalKind::OpenCodeUserInput {
            self.questions
                .as_ref()
                .context(conduit::i18n::text_static(
                    "cli_adapter.opencode.opencode_question_connection_unavailable",
                ))?
                .respond(approval, response)?;
            Ok(None)
        } else {
            Ok(Some(response))
        }
    }
    fn automatic_approval(&self, approval: &Approval) -> Option<bool> {
        self.core.automatic_approval(approval)
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        self.core.approval_response(approval, allow)
    }
    fn user_input_response(
        &self,
        approval: &Approval,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        self.core.user_input_response(approval, answers)
    }
    fn enable_full_access(&mut self) {
        self.core.enable_full_access();
    }
}
