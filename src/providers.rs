//! Per-session model routes. No global CLI or cc-switch configuration is changed.
use crate::domain::Provider;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
            Self::Default => "默认",
            Self::Low => "低",
            Self::Medium => "中",
            Self::High => "高",
            Self::Xhigh => "极高",
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
        let engine = match engine {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::OpenCode => "opencode",
        };
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
            bail!("OpenCode 首版使用 CLI 自身 Provider 配置，请在 opencode 中配置模型");
        }
        if self.name.trim().is_empty() {
            bail!("请输入 Provider 名称");
        }
        if !(self.base_url.starts_with("https://") || self.base_url.starts_with("http://"))
            || self.base_url.chars().any(char::is_whitespace)
            || self.base_url.contains(['\0', '"', '\\'])
        {
            bail!("Base URL 必须是有效的 http:// 或 https:// 地址");
        }
        if !self.api_key_env.is_empty()
            && (!self
                .api_key_env
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
                || self.api_key_env.starts_with(|c: char| c.is_ascii_digit()))
        {
            bail!("API Key 环境变量名只能包含字母、数字和下划线，且不能以数字开头");
        }
        if self.models.is_empty()
            || self
                .models
                .iter()
                .any(|id| id.trim().is_empty() || id.contains('\0'))
        {
            bail!("请至少填写一个模型 ID");
        }
        Ok(())
    }

    pub fn route(&self) -> Result<RuntimeRoute> {
        self.validate()?;
        let key = if self.api_key_env.is_empty() {
            None
        } else {
            Some(
                std::env::var(&self.api_key_env)
                    .ok()
                    .filter(|v| !v.trim().is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("请先设置环境变量 {} 并重新启动 Conduit", self.api_key_env)
                    })?,
            )
        };
        let mut route = RuntimeRoute::endpoint(self.engine, &self.name, &self.base_url, key);
        route.default_model = self.models.first().cloned();
        Ok(route)
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
