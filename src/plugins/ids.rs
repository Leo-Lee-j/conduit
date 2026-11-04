use super::{Result, require};
use std::fmt;

fn validate(value: &str) -> Result<()> {
    require(
        value.len() <= 160
            && value.contains('.')
            && value.split('.').all(|part| {
                !part.is_empty()
                    && part.as_bytes()[0].is_ascii_lowercase()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            }),
        format!("invalid contribution identifier: {value}"),
    )
}
macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                validate(&value)?;
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
identifier!(PluginId);
identifier!(ViewId);
identifier!(CommandId);
identifier!(ItemId);

/// Privilege is supplied by host code, never deserialized from a manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    id: PluginId,
    host: bool,
}
impl Owner {
    pub fn external(id: PluginId) -> Result<Self> {
        require(
            !id.as_str().starts_with("conduit."),
            "reserved conduit namespace",
        )?;
        Ok(Self { id, host: false })
    }
    pub(crate) fn host(id: PluginId) -> Self {
        Self { id, host: true }
    }
    pub fn id(&self) -> &PluginId {
        &self.id
    }
    pub fn is_host(&self) -> bool {
        self.host
    }
    pub(super) fn owns(&self, id: &str) -> bool {
        id.strip_prefix(self.id.as_str())
            .is_some_and(|suffix| suffix.starts_with('.'))
    }
}
