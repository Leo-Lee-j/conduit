//! Per-session model routes. No global CLI or cc-switch configuration is changed.
use crate::domain::Provider;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub mod credentials;
pub mod discovery;

/// Only catalog changes invalidate the model panel; ordinary parent renders do not.
pub struct ProviderCatalogChanged;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelChannel {
    #[default]
    Cli,
    Ccs(String),
    #[serde(alias = "custom")]
    Provider(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    #[default]
    Default,
    Low,
    Medium,
    High,
    Xhigh,
}

impl ReasoningEffort {
    /// OpenCode negotiates its native effort catalog after selecting the model.
    pub fn for_provider(provider: Provider) -> Self {
        match provider {
            Provider::Claude | Provider::Codex => Self::Medium,
            Provider::OpenCode => Self::Default,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Default,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
    ];
    pub fn wire(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::Xhigh => Some("xhigh"),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => conduit::i18n::text_static("chat.default"),
            Self::Low => conduit::i18n::text_static("providers.low"),
            Self::Medium => conduit::i18n::text_static("providers.medium"),
            Self::High => conduit::i18n::text_static("providers.high"),
            Self::Xhigh => conduit::i18n::text_static("providers.extra_high"),
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelProvider {
    pub id: String,
    pub name: String,
    pub engine: Provider,
    pub base_url: String,
    /// Name of an inherited environment variable, never the credential itself.
    pub api_key_env: String,
    /// Windows account protected ciphertext, never a plaintext credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protected_api_key: Option<String>,
    pub models: Vec<String>,
    /// Origin is metadata; all providers share one registry and picker.
    #[serde(default)]
    pub source: ProviderSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_metadata: Vec<crate::models::ModelOption>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSource {
    #[default]
    Local,
    Ccs {
        id: String,
    },
}

impl ModelProvider {
    pub fn ccs_id(engine: Provider, id: &str) -> String {
        let engine = engine.descriptor().id;
        format!("ccs:{engine}:{id}")
    }

    pub fn catalog(&self) -> Vec<crate::models::ModelOption> {
        self.models
            .iter()
            .map(|id| {
                self.model_metadata
                    .iter()
                    .find(|m| m.id == *id)
                    .cloned()
                    .unwrap_or_else(|| crate::models::ModelOption {
                        id: id.clone(),
                        label: id.clone(),
                        note: String::new(),
                        context_window: None,
                    })
            })
            .collect()
    }

    pub fn runtime_route(&self, database: &std::path::Path) -> Result<RuntimeRoute> {
        match &self.source {
            ProviderSource::Local => self.route(),
            ProviderSource::Ccs { id } => crate::ccs::runtime_route(database, self.engine, id),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.engine == Provider::OpenCode {
            bail!(conduit::i18n::text(
                "providers.opencode_uses_its_native_provider_configuration_configure"
            ));
        }
        if self.name.trim().is_empty() {
            bail!(conduit::i18n::text("providers.enter_a_provider_name"));
        }
        discovery::endpoint(&self.base_url)?;
        if !self.api_key_env.is_empty()
            && (!self
                .api_key_env
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                || self.api_key_env.starts_with(|c: char| c.is_ascii_digit()))
        {
            bail!(conduit::i18n::text(
                "providers.api_key_environment_variable_names_may_contain"
            ));
        }
        if self.models.is_empty()
            || self
                .models
                .iter()
                .any(|id| id.trim().is_empty() || id.contains('\0'))
        {
            bail!(conduit::i18n::text("providers.enter_at_least_one_model_id"));
        }
        Ok(())
    }

    pub fn route(&self) -> Result<RuntimeRoute> {
        self.validate()?;
        let key = self.api_key()?;
        let mut route = RuntimeRoute::endpoint(self.engine, &self.name, &self.base_url, key);
        route.default_model = self.models.first().cloned();
        Ok(route)
    }

    pub fn api_key(&self) -> Result<Option<String>> {
        if let Some(protected) = &self.protected_api_key {
            credentials::unprotect(protected).map(Some)
        } else if self.api_key_env.is_empty() {
            Ok(None)
        } else {
            Ok(Some(
                std::env::var(&self.api_key_env)
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(conduit::i18n::format(
                            "providers.set_environment_variable_and_restart_conduit_first",
                            &[("arg0", self.api_key_env.to_string())]
                        ))
                    })?,
            ))
        }
    }
}

#[derive(Clone, Default)]
pub struct RuntimeRoute {
    pub config: serde_json::Map<String, Value>,
    pub default_model: Option<String>,
    pub opencode_variant: Option<String>,
    pub env: BTreeMap<String, String>,
}

impl RuntimeRoute {
    pub fn endpoint(engine: Provider, name: &str, url: &str, key: Option<String>) -> Self {
        let mut route = Self::default();
        match engine {
            Provider::OpenCode => {}
            Provider::Claude => {
                route.env.insert("ANTHROPIC_BASE_URL".into(), url.into());
                route
                    .env
                    .insert("CLAUDE_CODE_USE_BEDROCK".into(), "0".into());
                route
                    .env
                    .insert("CLAUDE_CODE_USE_VERTEX".into(), "0".into());
                route
                    .env
                    .insert("CLAUDE_CODE_USE_FOUNDRY".into(), "0".into());
                // Keep credentials scoped to the child process.
                for name in [
                    "ANTHROPIC_AUTH_TOKEN",
                    "ANTHROPIC_API_KEY",
                    "CLAUDE_CODE_OAUTH_TOKEN",
                ] {
                    route.env.insert(name.into(), String::new());
                }
                if let Some(key) = key {
                    route.env.insert("ANTHROPIC_AUTH_TOKEN".into(), key);
                }
            }
            Provider::Codex => {
                let mut provider = json!({"name":name,"base_url":url,"wire_api":"responses","requires_openai_auth":false});
                if let Some(key) = key {
                    route.env.insert("CONDUIT_PROVIDER_API_KEY".into(), key);
                    provider["env_key"] = json!("CONDUIT_PROVIDER_API_KEY");
                }
                route
                    .config
                    .insert("model_provider".into(), json!("conduit"));
                route
                    .config
                    .insert("model_providers".into(), json!({"conduit":provider}));
            }
        }
        route
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn legacy_providers_load_and_protected_credentials_only_enter_runtime_environment() {
        let _locale = crate::test_support::chinese();
        let mut provider: ModelProvider = serde_json::from_value(json!({
            "id":"test", "name":"Test", "engine":"Codex", "base_url":"https://example.invalid/v1", "api_key_env":"", "models":["test-model"]
        })).unwrap();
        assert!(provider.protected_api_key.is_none());
        provider.protected_api_key = Some(credentials::protect("fixture-only-key").unwrap());
        let serialized = serde_json::to_string(&provider).unwrap();
        assert!(!serialized.contains("fixture-only-key"));
        let restored: ModelProvider = serde_json::from_str(&serialized).unwrap();
        let route = restored.route().unwrap();
        assert_eq!(route.env["CONDUIT_PROVIDER_API_KEY"], "fixture-only-key");
        assert_eq!(route.default_model.as_deref(), Some("test-model"));
        assert!(
            !serde_json::to_string(&route.config)
                .unwrap()
                .contains("fixture-only-key")
        );
    }
}
