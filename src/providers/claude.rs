//! Per-service Claude aliases. Values are scoped to the launched CLI process.
use super::RuntimeRoute;
use crate::models::ModelOption;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Sonnet,
    Opus,
    Fable,
    Haiku,
    Subagent,
}

impl Role {
    pub const ALL: [Self; 5] = [
        Self::Sonnet,
        Self::Opus,
        Self::Fable,
        Self::Haiku,
        Self::Subagent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Sonnet => "Sonnet",
            Self::Opus => "Opus",
            Self::Fable => "Fable",
            Self::Haiku => "Haiku",
            Self::Subagent => "Subagent",
        }
    }

    pub fn alias(self) -> &'static str {
        match self {
            Self::Sonnet => "sonnet",
            Self::Opus => "opus",
            Self::Fable => "fable",
            Self::Haiku => "haiku",
            Self::Subagent => "subagent",
        }
    }

    pub fn env(self) -> &'static str {
        match self {
            Self::Sonnet => "ANTHROPIC_DEFAULT_SONNET_MODEL",
            Self::Opus => "ANTHROPIC_DEFAULT_OPUS_MODEL",
            Self::Fable => "ANTHROPIC_DEFAULT_FABLE_MODEL",
            Self::Haiku => "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            Self::Subagent => "CLAUDE_CODE_SUBAGENT_MODEL",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub model: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
}

/// `[1M]` is the Claude CLI's capability marker, not part of the upstream ID.
pub fn split_one_m(model: &str) -> (&str, bool) {
    let model = model.trim();
    if model
        .get(model.len().saturating_sub(4)..)
        .is_some_and(|tail| tail.eq_ignore_ascii_case("[1m]"))
    {
        (model[..model.len() - 4].trim_end(), true)
    } else {
        (model, false)
    }
}

pub fn with_one_m(model: &str, enabled: bool) -> String {
    let (base, _) = split_one_m(model);
    if enabled && !base.is_empty() {
        format!("{base}[1M]")
    } else {
        base.to_owned()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeModels {
    #[serde(default)]
    pub roles: BTreeMap<Role, Mapping>,
    #[serde(default)]
    pub default_model: String,
}

impl ClaudeModels {
    pub fn effective_model(&self, role: Role) -> &str {
        if let Some(mapping) = self.roles.get(&role).filter(|m| !m.model.is_empty()) {
            return &mapping.model;
        }
        match role {
            Role::Subagent => "", // An unset subagent inherits the selected main model.
            Role::Fable => self.effective_model(Role::Opus),
            _ => &self.default_model,
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let valid_model = |value: &str| {
            value.len() <= 512
                && !value.chars().any(|c| c.is_whitespace() || c.is_control())
                && (value.is_empty() || !split_one_m(value).0.is_empty())
        };
        anyhow::ensure!(
            valid_model(&self.default_model)
                && self.roles.values().all(|m| valid_model(&m.model)
                    && m.display_name.len() <= 512
                    && !m.display_name.chars().any(char::is_control)
                    && (m.display_name.is_empty() || !m.model.is_empty())),
            conduit::i18n::text("settings.models.invalid_mapping")
        );
        anyhow::ensure!(
            Role::ALL
                .into_iter()
                .filter(|role| *role != Role::Subagent)
                .any(|role| !self.effective_model(role).is_empty()),
            conduit::i18n::text("providers.enter_at_least_one_model_id")
        );
        Ok(())
    }

    pub fn apply(&self, route: &mut RuntimeRoute) {
        for role in Role::ALL {
            let model = self.effective_model(role);
            route.env.insert(role.env().into(), model.into());
            if role != Role::Subagent {
                let name = self
                    .roles
                    .get(&role)
                    .map(|m| m.display_name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| split_one_m(model).0);
                route
                    .env
                    .insert(format!("{}_NAME", role.env()), name.into());
            }
        }
        // Do not let inherited legacy aliases override this service's mapping.
        route.env.insert(
            "ANTHROPIC_SMALL_FAST_MODEL".into(),
            self.effective_model(Role::Haiku).into(),
        );
        route
            .env
            .insert("ANTHROPIC_REASONING_MODEL".into(), String::new());
        route.default_model = if self.default_model.is_empty() {
            Role::ALL
                .into_iter()
                .find(|role| *role != Role::Subagent && !self.effective_model(*role).is_empty())
                .map(|role| role.alias().to_owned())
        } else {
            Some(self.default_model.clone())
        };
        route.env.insert(
            "ANTHROPIC_MODEL".into(),
            route.default_model.clone().unwrap_or_default(),
        );
    }

    pub fn extend_catalog(&self, catalog: &mut Vec<ModelOption>) {
        for role in Role::ALL.into_iter().filter(|role| *role != Role::Subagent) {
            let model = self.effective_model(role);
            if model.is_empty() {
                continue;
            }
            let name = self
                .roles
                .get(&role)
                .map(|m| m.display_name.as_str())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| split_one_m(model).0);
            // Aliases remain distinct even when they route to the same upstream ID.
            catalog.retain(|m| m.id != role.alias());
            catalog.push(ModelOption {
                id: role.alias().into(),
                label: name.into(),
                note: format!("{} · {}", role.label(), model),
                context_window: split_one_m(model).1.then_some(1_000_000),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mappings_roundtrip_and_reach_runtime_without_exposing_subagent_in_picker() {
        let mut mapping = ClaudeModels {
            default_model: "fallback[1M]".into(),
            ..Default::default()
        };
        mapping.roles.insert(
            Role::Sonnet,
            Mapping {
                model: "fast".into(),
                display_name: "Everyday".into(),
            },
        );
        mapping.roles.insert(
            Role::Opus,
            Mapping {
                model: "deep[1M]".into(),
                display_name: "Think".into(),
            },
        );
        mapping.roles.insert(
            Role::Subagent,
            Mapping {
                model: "worker".into(),
                ..Default::default()
            },
        );
        let restored: ClaudeModels =
            serde_json::from_str(&serde_json::to_string(&mapping).unwrap()).unwrap();
        assert_eq!(restored, mapping);
        let mut route = RuntimeRoute::default();
        restored.apply(&mut route);
        assert_eq!(route.default_model.as_deref(), Some("fallback[1M]"));
        assert_eq!(route.env[Role::Sonnet.env()], "fast");
        assert_eq!(route.env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"], "Everyday");
        assert_eq!(route.env[Role::Fable.env()], "deep[1M]");
        assert_eq!(route.env[Role::Haiku.env()], "fallback[1M]");
        assert_eq!(route.env[Role::Subagent.env()], "worker");
        let mut catalog = vec![];
        restored.extend_catalog(&mut catalog);
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0].id, "sonnet");
        assert_eq!(catalog[0].label, "Everyday");
        assert_eq!(catalog[1].context_window, Some(1_000_000));
        assert!(
            catalog
                .iter()
                .all(|m| m.id != "subagent" && m.id != "worker")
        );
    }

    #[test]
    fn capability_marker_and_empty_mapping_validation() {
        assert_eq!(with_one_m(" routed/model[1m] ", true), "routed/model[1M]");
        assert_eq!(with_one_m("routed/model[1M]", false), "routed/model");
        assert_eq!(with_one_m("", true), "");
        assert!(ClaudeModels::default().validate().is_err());
        let mut mapping = ClaudeModels::default();
        mapping.roles.insert(
            Role::Subagent,
            Mapping {
                model: "worker".into(),
                ..Default::default()
            },
        );
        assert!(mapping.validate().is_err());
        mapping.roles.insert(
            Role::Haiku,
            Mapping {
                model: "small".into(),
                ..Default::default()
            },
        );
        assert!(mapping.validate().is_ok());
        let mut route = RuntimeRoute::default();
        mapping.apply(&mut route);
        assert_eq!(route.default_model.as_deref(), Some("haiku"));
        mapping.default_model = "bad\nmodel".into();
        assert!(mapping.validate().is_err());
    }
}
