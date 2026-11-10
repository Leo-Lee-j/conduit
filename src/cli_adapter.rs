//! Engine-independent contract between CLI integrations and the process runtime.
mod builtin;
mod opencode;
mod qoder;
pub use builtin::{claude, codex};
pub use opencode::COMMAND_SOURCE as OPENCODE_COMMANDS;
pub use opencode::opencode;
pub use qoder::qoder;

use crate::{
    domain::PermissionLevel,
    protocol::{Approval, Step},
    providers::{ReasoningEffort, RuntimeRoute},
};
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

pub struct AdapterRequest {
    pub intent: crate::commands::RequestIntent,
    pub cwd: PathBuf,
    pub prompt: String,
    pub native_id: Option<String>,
    pub model: Option<String>,
    pub permission: PermissionLevel,
    pub effort: ReasoningEffort,
    pub route: RuntimeRoute,
    pub text_only: bool,
}

/// Secrets and prompts must not be formatted into logs.
pub struct LaunchSpec {
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub input: InitialInput,
    pub completion: Completion,
}

pub enum InitialInput {
    Json(Value),
    /// Write the supplied bytes, then close stdin before reading the response.
    TextToEof(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    ProtocolEvent,
    SuccessfulExit,
}

/// One instance per turn. Factories only construct state; launch runs after the
/// save gate. The runtime retains this object until its process tree is empty.
/// JSON-lines over stdio is the base transport; poll/route_response provide
/// bounded native side channels without exposing them to the runtime.
pub trait CliAdapter: Send {
    fn launch(&mut self) -> anyhow::Result<LaunchSpec>;
    fn accept(&mut self, value: Value) -> Step;
    fn startup_pending(&self) -> bool;
    fn cancel_message(&self) -> Option<Value> {
        None
    }
    fn poll(&mut self) -> anyhow::Result<Vec<Value>> {
        Ok(vec![])
    }
    /// Return Some for a stdio response, None after handling it on a side channel.
    fn route_response(
        &mut self,
        _approval: &Approval,
        response: Value,
    ) -> anyhow::Result<Option<Value>> {
        Ok(Some(response))
    }
    fn automatic_approval(&self, approval: &Approval) -> Option<bool>;
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value;
    fn user_input_response(
        &self,
        approval: &Approval,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value>;
    fn enable_full_access(&mut self);
}

pub type AdapterFactory = fn(AdapterRequest) -> Box<dyn CliAdapter>;

#[cfg(test)]
pub(crate) fn from_protocol(
    protocol: crate::protocol::Protocol,
    cwd: PathBuf,
) -> Box<dyn CliAdapter> {
    if protocol.provider() == crate::domain::Provider::OpenCode {
        Box::new(opencode::OpenCodeAdapter::new(protocol, cwd))
    } else {
        Box::new(builtin::StdioAdapter::new(protocol))
    }
}

#[cfg(test)]
mod tests;
