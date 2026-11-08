//! Host data models shared by desktop adapters and focused tests.
extern crate self as conduit;
pub mod plugin_packages;
pub mod plugins;
pub mod update;

rust_i18n::i18n!("locales", fallback = "en");
pub mod i18n;

#[cfg(test)]
mod test_support;
