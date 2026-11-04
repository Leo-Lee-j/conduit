//! Backend-independent workbench contributions. No GPUI entities, I/O or plugin code
//! are stored here. App adapters own rendering, business checks and cleanup.
pub mod builtins;
pub mod commands;
pub mod context;
pub mod contributions;
pub mod ids;
pub mod registry;
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
mod tests;
