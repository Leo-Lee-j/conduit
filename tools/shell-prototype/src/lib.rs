//! Candidate development adapter shared by the probe and desktop validation build.
// Compile the actual P1 implementation, rather than maintaining a second registry.
#[cfg(not(feature = "desktop"))]
#[path = "../../../src/plugins.rs"]
pub mod plugins;
#[cfg(feature = "desktop")]
pub use conduit::plugins;

pub mod adapter;
pub mod manifest;
pub mod package;
