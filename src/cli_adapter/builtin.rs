//! Compatibility backend for the existing, fixture-tested wire implementations.
use super::*;
use crate::{domain::Provider, protocol::Protocol};

pub fn claude(request: AdapterRequest) -> Box<dyn CliAdapter> {
    Box::new(StdioAdapter::new(protocol(Provider::Claude, request)))
}

pub fn codex(request: AdapterRequest) -> Box<dyn CliAdapter> {
    Box::new(StdioAdapter::new(protocol(Provider::Codex, request)))
}

pub(super) fn protocol(provider: Provider, request: AdapterRequest) -> Protocol {
    let protocol = Protocol::new(
        provider,
        request.cwd.to_string_lossy().into_owned(),
        request.prompt,
        request.native_id,
    )
    .with_model(request.model)
    .with_permission(request.permission)
    .with_options(request.effort, request.route);
    if request.text_only {
        protocol.text_only()
    } else {
        protocol
    }
}

pub(super) struct StdioAdapter {
    pub protocol: Protocol,
}
impl StdioAdapter {
    pub fn new(protocol: Protocol) -> Self {
        Self { protocol }
    }
}

impl CliAdapter for StdioAdapter {
    fn launch(&mut self) -> anyhow::Result<LaunchSpec> {
        self.protocol.prepare()?;
        let (input, completion) = match self.protocol.opencode_text_input() {
            Some(prompt) => (InitialInput::TextToEof(prompt), Completion::SuccessfulExit),
            None => (
                InitialInput::Json(self.protocol.initialize()),
                Completion::ProtocolEvent,
            ),
        };
        Ok(LaunchSpec {
            args: self.protocol.args(),
            environment: self.protocol.environment().clone(),
            input,
            completion,
        })
    }
    fn accept(&mut self, value: Value) -> Step {
        self.protocol.accept(value)
    }
    fn startup_pending(&self) -> bool {
        self.protocol.startup_pending()
    }
    fn cancel_message(&self) -> Option<Value> {
        self.protocol.cancel_message()
    }
    fn automatic_approval(&self, approval: &Approval) -> Option<bool> {
        self.protocol.automatic_approval(approval)
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        self.protocol.approval_response(approval, allow)
    }
    fn user_input_response(
        &self,
        approval: &Approval,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        self.protocol.user_input_response(approval, answers)
    }
    fn enable_full_access(&mut self) {
        self.protocol.enable_full_access();
    }
}
