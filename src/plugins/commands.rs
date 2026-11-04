use super::{
    Result,
    context::{PluginContext, Scope},
    ids::{CommandId, Owner},
    require,
};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    WorkspaceSummary,
    SessionSummary,
    HostMutation,
}
#[derive(Clone, Debug)]
pub enum ArgumentPolicy {
    None,
    Object { max_bytes: usize, max_depth: usize },
}
impl ArgumentPolicy {
    pub(super) fn validate(&self, args: &Value) -> Result<()> {
        match self {
            Self::None => require(args.is_null(), "command accepts no arguments"),
            Self::Object {
                max_bytes,
                max_depth,
            } => {
                require(args.is_object(), "command arguments must be an object")?;
                let mut pending = vec![(args, 0)];
                let mut bytes = 0usize;
                while let Some((value, depth)) = pending.pop() {
                    require(depth <= *max_depth, "command arguments are too deep")?;
                    // Count a lower bound before adding child references to the worklist.
                    // The final serialization check includes escaping and exact JSON bytes.
                    let amount = match value {
                        Value::Null => 4,
                        Value::Bool(value) => {
                            if *value {
                                4
                            } else {
                                5
                            }
                        }
                        Value::Number(value) => value.to_string().len(),
                        Value::String(s) => s.len().saturating_add(2),
                        Value::Array(items) => 2usize.saturating_add(items.len().saturating_sub(1)),
                        Value::Object(items) => items.keys().fold(
                            2usize.saturating_add(items.len().saturating_sub(1)),
                            |size, key| size.saturating_add(key.len()).saturating_add(3),
                        ),
                    };
                    bytes = bytes.saturating_add(amount);
                    require(bytes <= *max_bytes, "command arguments exceed budget")?;
                    match value {
                        Value::Array(items) => pending.extend(items.iter().map(|v| (v, depth + 1))),
                        Value::Object(items) => {
                            pending.extend(items.values().map(|v| (v, depth + 1)))
                        }
                        _ => {}
                    }
                }
                require(
                    serde_json::to_vec(args).is_ok_and(|json| json.len() <= *max_bytes),
                    "encoded command arguments exceed budget",
                )
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct CommandDescriptor {
    pub id: CommandId,
    pub title: String,
    pub scope: Scope,
    pub arguments: ArgumentPolicy,
    pub required_capabilities: BTreeSet<Capability>,
    pub requires_writable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandOrigin {
    Activity,
    Status,
    View,
    Host,
}
/// Issued by the host after validating a user action. Fields cannot be forged by a manifest.
#[derive(Clone, Debug)]
pub struct CommandInvocation {
    pub(super) registration: super::registry::Registration,
    pub(super) command: CommandId,
    pub(super) args: Value,
    pub(super) origin: CommandOrigin,
    pub(super) context_version: u64,
    pub(super) scope: super::context::ScopeKey,
}
impl CommandInvocation {
    pub fn command(&self) -> &CommandId {
        &self.command
    }
    pub fn args(&self) -> &Value {
        &self.args
    }
    pub fn origin(&self) -> &CommandOrigin {
        &self.origin
    }
    pub fn context_version(&self) -> u64 {
        self.context_version
    }
    pub fn scope(&self) -> &super::context::ScopeKey {
        &self.scope
    }
}
pub(super) fn eligible(
    owner: &Owner,
    grants: &BTreeSet<Capability>,
    command: &CommandDescriptor,
    cx: &PluginContext,
) -> Result<()> {
    require(!cx.input().closing, "host is closing")?;
    require(
        (owner.is_host() && !command.requires_writable) || cx.writable(),
        "host is read-only",
    )?;
    require(
        command.required_capabilities.is_subset(grants),
        "command capability not granted",
    )?;
    require(
        cx.scope(command.scope).is_some(),
        "command scope is unavailable",
    )
}
