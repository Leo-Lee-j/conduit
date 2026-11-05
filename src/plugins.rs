//! Backend-independent workbench contributions with no concrete GPUI or I/O dependency.
//! App adapters instantiate the generic cache and own rendering and task resources.
// Explicit paths also let the isolated Shell prototype compile the exact P1 core.
#[path = "plugins/builtins.rs"]
pub mod builtins;
#[path = "plugins/commands.rs"]
pub mod commands;
#[path = "plugins/context.rs"]
pub mod context;
#[path = "plugins/contributions.rs"]
pub mod contributions;
#[path = "plugins/ids.rs"]
pub mod ids;
#[path = "plugins/instances.rs"]
pub mod instances;
#[path = "plugins/registry.rs"]
pub mod registry;
#[path = "plugins/state.rs"]
pub mod state;

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
pub(super) fn require(ok: bool, message: impl Into<String>) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error(message.into()))
    }
}

#[cfg(test)]
#[path = "plugins/tests.rs"]
mod tests;
