//! Workspace-owned native terminals. The byte stream never enters chat history.
mod input;
mod pty;
mod runtime;
mod screen;
mod shell;

pub(crate) use input::encode_key;
pub(crate) use runtime::{RunState, Runtime};
pub(crate) use screen::{Snapshot, palette};
pub(crate) use shell::ShellProfile;

#[cfg(test)]
mod tests;
