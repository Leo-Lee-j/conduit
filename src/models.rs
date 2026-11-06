//! Read-only model catalogs from local CLI configuration.
//!
//! Only model identifiers and non-secret endpoint information are read. No
//! credentials are inspected, and neither configuration file is modified.
use crate::domain::Provider;
mod opencode;
pub mod preferences;
pub use opencode::CACHE_TTL as OPENCODE_CACHE_TTL;
pub use opencode::OpenCodeModelsProbe;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub note: String,
    /// Context window in tokens when known (catalog field or `[1M]`-style ID
    /// suffix); used only as a display fallback for the usage ring.
    pub context_window: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct ModelCatalog {
    pub fetched_at: Option<std::time::SystemTime>,
    pub config_path: PathBuf,
    pub catalog_path: Option<PathBuf>,
    pub current: Option<String>,
    pub models: Vec<ModelOption>,
    pub endpoint: Option<String>,
    pub detail: Option<String>,
    pub note: Option<String>,
    pub error: Option<String>,
}

impl ModelCatalog {
    pub fn option(&self, id: &str) -> Option<&ModelOption> {
        self.models.iter().find(|model| model.id == id)
    }
    pub fn label(&self, id: &str) -> String {
        self.option(id)
            .map(|model| model.label.clone())
            .unwrap_or_else(|| id.to_string())
    }
    /// Context window for a model, falling back to parsing the model ID's
    /// `[1M]`-style suffix. Returns `None` when both are unknown.
    pub fn context_window(&self, id: &str) -> Option<u64> {
        self.option(id)
            .and_then(|model| model.context_window)
            .or_else(|| context_from_suffix(id))
    }
    pub fn with_synced(&self, source: &str, synced: &[ModelOption]) -> Self {
        let mut catalog = self.clone();
        for model in synced {
            let note = match (model.note.as_str(), source) {
                ("", "") => String::new(),
                ("", source) => source.to_string(),
                (note, "") => note.to_string(),
                (note, source) => format!("{note} · {source}"),
            };
            push_model_with_context(
                &mut catalog.models,
                &model.id,
                &model.label,
                &note,
                model.context_window,
            );
        }
        catalog
    }
}

/// Vendor family behind a model ID, used to pick the official mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelBrand {
    Anthropic,
    Xai,
    DeepSeek,
    Qwen,
    Kimi,
    Zhipu,
    MiniMax,
    Doubao,
    Hunyuan,
    Gemini,
    Step,
    Mistral,
    Meta,
    Cohere,
    Gemma,
    Nvidia,
    Perplexity,
    Ai21,
    Baichuan,
    Yi,
    Spark,
    Nova,
    Wenxin,
    OpenAI,
    Other,
}

/// Family tokens ordered from the most specific vendor name to the generic
/// OpenAI-compatible aliases, because the aliases (`o1`, `o3`, `o4`) collide
/// with version digits in other vendors' IDs.
const BRAND_TOKENS: &[(ModelBrand, &[&str])] = &[
    (
        ModelBrand::Anthropic,
        &["claude", "anthropic", "sonnet", "opus", "haiku"],
    ),
    (ModelBrand::Xai, &["grok", "xai"]),
    (ModelBrand::DeepSeek, &["deepseek"]),
    (ModelBrand::Qwen, &["qwen", "qwq", "tongyi"]),
    (ModelBrand::Kimi, &["kimi", "moonshot", "moonshotai"]),
    (ModelBrand::Zhipu, &["glm", "chatglm", "zhipu", "cogvlm"]),
    (ModelBrand::MiniMax, &["minimax", "hailuo", "abab"]),
    (ModelBrand::Doubao, &["doubao", "seed", "seedream"]),
    (ModelBrand::Hunyuan, &["hunyuan"]),
    (ModelBrand::Gemini, &["gemini"]),
    (ModelBrand::Step, &["step", "stepfun"]),
    (
        ModelBrand::Mistral,
        &[
            "mistral",
            "mistralai",
            "mixtral",
            "ministral",
            "codestral",
            "devstral",
            "magistral",
        ],
    ),
    (ModelBrand::Nvidia, &["nvidia", "nemotron"]),
    (ModelBrand::Meta, &["llama", "meta-llama"]),
    (ModelBrand::Cohere, &["cohere", "command-r", "command-a"]),
    (ModelBrand::Gemma, &["gemma", "shieldgemma", "paligemma"]),
    (ModelBrand::Perplexity, &["perplexity", "sonar"]),
    (ModelBrand::Ai21, &["ai21", "jamba"]),
    (ModelBrand::Baichuan, &["baichuan"]),
    (ModelBrand::Yi, &["yi", "01-ai"]),
    (ModelBrand::Spark, &["spark", "sparkdesk"]),
    (ModelBrand::Nova, &["nova"]),
    (ModelBrand::Wenxin, &["ernie", "wenxin"]),
    (
        ModelBrand::OpenAI,
        &["gpt", "openai", "chatgpt", "codex", "o1", "o3", "o4"],
    ),
];

/// Match family names at ID boundaries so routed IDs work without confusing
/// short aliases such as `yi` and `o1` with letters/version digits in custom IDs.
pub fn brand_of(id: &str) -> ModelBrand {
    let id = id.to_lowercase();
    BRAND_TOKENS
        .iter()
        .find(|(_, tokens)| {
            tokens.iter().any(|token| {
                id.match_indices(token).any(|(start, token)| {
                    let before = id[..start].chars().next_back();
                    let after = id[start + token.len()..].chars().next();
                    before.is_none_or(|c| !c.is_ascii_alphanumeric())
                        && after.is_none_or(|c| !c.is_ascii_alphabetic())
                        && (!matches!(token, "o1" | "o3" | "o4")
                            || after.is_none_or(|c| !c.is_ascii_alphanumeric()))
                })
            })
        })
        .map_or(ModelBrand::Other, |(brand, _)| *brand)
}

pub fn load(provider: Provider) -> ModelCatalog {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    load_from(provider, &home)
}

pub fn load_from(provider: Provider, home: &Path) -> ModelCatalog {
    match provider {
        Provider::Claude => claude(home),
        Provider::Codex => codex(home),
        Provider::OpenCode => ModelCatalog {
            note: Some("打开模型面板后从 OpenCode CLI 读取原生模型目录".into()),
            ..Default::default()
        },
    }
}

pub(crate) fn push_model(models: &mut Vec<ModelOption>, id: &str, label: &str, note: &str) {
    push_model_with_context(models, id, label, note, None)
}

/// Context window parsed from a `[1M]` / `[200K]`-style model ID suffix.
fn context_from_suffix(id: &str) -> Option<u64> {
    let suffix = id.strip_suffix(']')?.rsplit_once('[')?.1;
    let (value, multiplier) = if let Some(value) = suffix.strip_suffix('K') {
        (value, 1_000u64)
    } else {
        (suffix.strip_suffix('M')?, 1_000_000u64)
    };
    if value.is_empty() || !value.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<u64>()
        .ok()?
        .checked_mul(multiplier)
        .filter(|size| *size > 0)
}

pub(crate) fn push_model_with_context(
    models: &mut Vec<ModelOption>,
    id: &str,
    label: &str,
    note: &str,
    context_window: Option<u64>,
) {
    let id = id.trim();
    if id.is_empty() {
        return;
    }
    // A catalog-supplied window wins; otherwise derive it from the ID suffix.
    let context_window = context_window.or_else(|| context_from_suffix(id));
    let label = label.trim();
    if let Some(existing) = models.iter_mut().find(|model| model.id == id) {
        for part in note.split(" · ").filter(|part| !part.is_empty()) {
            if !existing.note.split(" · ").any(|current| current == part) {
                if !existing.note.is_empty() {
                    existing.note.push_str(" · ");
                }
                existing.note.push_str(part);
            }
        }
        if (existing.label.is_empty() || existing.label == existing.id)
            && !label.is_empty()
            && label != id
        {
            existing.label = label.to_string();
        }
        if existing.context_window.is_none() {
            existing.context_window = context_window;
        }
        return;
    }
    models.push(ModelOption {
        id: id.to_string(),
        label: if label.is_empty() { id } else { label }.to_string(),
        note: note.to_string(),
        context_window,
    });
}

fn read_text(path: &Path, label: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("无法读取{label}：{error}"))?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

fn claude(home: &Path) -> ModelCatalog {
    let path = home.join(".claude").join("settings.json");
    let mut catalog = ModelCatalog {
        config_path: path.clone(),
        ..Default::default()
    };
    if !path.exists() {
        catalog.note = Some("未找到 Claude 配置；将使用 CLI 自身默认模型。".into());
        return catalog;
    }
    let text = match read_text(&path, " Claude 配置") {
        Ok(text) => text,
        Err(error) => {
            catalog.error = Some(error);
            return catalog;
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            catalog.error = Some(format!("Claude 配置不是有效 JSON：{error}"));
            return catalog;
        }
    };
    let env = value.get("env");
    let env_text = |key: &str| {
        env.and_then(|env| env.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    let top_text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    catalog.current = env_text("ANTHROPIC_MODEL")
        .or_else(|| top_text("model"))
        .map(str::to_string);
    catalog.endpoint = env_text("ANTHROPIC_BASE_URL").map(str::to_string);
    if let Some(current) = &catalog.current {
        push_model(&mut catalog.models, current, current, "当前默认");
    }
    for (key, role) in [
        ("ANTHROPIC_DEFAULT_OPUS_MODEL", "Opus"),
        ("ANTHROPIC_DEFAULT_SONNET_MODEL", "Sonnet"),
        ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "Haiku"),
        ("ANTHROPIC_DEFAULT_FABLE_MODEL", "Fable"),
        ("CLAUDE_CODE_SUBAGENT_MODEL", "子代理"),
    ] {
        if let Some(id) = env_text(key) {
            push_model(&mut catalog.models, id, id, role);
        }
    }
    if catalog.models.is_empty() {
        catalog.note = Some("配置中未声明模型；将使用 CLI 自身默认模型。".into());
    }
    catalog
}

fn codex(home: &Path) -> ModelCatalog {
    let directory = home.join(".codex");
    let path = directory.join("config.toml");
    let mut catalog = ModelCatalog {
        config_path: path.clone(),
        ..Default::default()
    };
    if !path.exists() {
        catalog.note = Some("未找到 Codex 配置；将使用 CLI 自身默认模型。".into());
        return catalog;
    }
    let text = match read_text(&path, " Codex 配置") {
        Ok(text) => text,
        Err(error) => {
            catalog.error = Some(error);
            return catalog;
        }
    };
    let value: toml::Value = match text.parse() {
        Ok(value) => value,
        Err(error) => {
            catalog.error = Some(format!("Codex 配置不是有效 TOML：{error}"));
            return catalog;
        }
    };
    let table = value.as_table();
    let text_value = |key: &str| {
        table
            .and_then(|table| table.get(key))
            .and_then(toml::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    catalog.current = text_value("model").map(str::to_string);
    let provider = text_value("model_provider");
    let effort = text_value("model_reasoning_effort");
    catalog.endpoint = provider
        .and_then(|provider| {
            table?
                .get("model_providers")?
                .get(provider)?
                .get("base_url")
        })
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    catalog.detail = {
        let mut parts = vec![];
        if let Some(provider) = provider {
            parts.push(format!("provider {provider}"));
        }
        if let Some(effort) = effort {
            parts.push(format!("推理 {effort}"));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    };
    if let Some(relative) = text_value("model_catalog_json") {
        let catalog_path = PathBuf::from(relative);
        let catalog_path = if catalog_path.is_absolute() {
            catalog_path
        } else {
            directory.join(catalog_path)
        };
        catalog.catalog_path = Some(catalog_path.clone());
        match fs::read(&catalog_path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                match serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}'))
                {
                    Ok(value) => {
                        let entries = value
                            .get("models")
                            .and_then(serde_json::Value::as_array)
                            .or_else(|| value.as_array());
                        if let Some(entries) = entries {
                            for entry in entries {
                                let Some(slug) = entry
                                    .get("slug")
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::trim)
                                    .filter(|slug| !slug.is_empty())
                                else {
                                    continue;
                                };
                                if entry
                                    .get("visibility")
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|visibility| visibility == "hide")
                                {
                                    continue;
                                }
                                let label = entry
                                    .get("display_name")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or(slug);
                                let note = entry
                                    .get("default_reasoning_level")
                                    .and_then(serde_json::Value::as_str)
                                    .map(|level| format!("推理 {level}"))
                                    .unwrap_or_default();
                                let context_window = entry
                                    .get("context_window")
                                    .and_then(serde_json::Value::as_u64);
                                push_model_with_context(
                                    &mut catalog.models,
                                    slug,
                                    label,
                                    &note,
                                    context_window,
                                );
                            }
                        }
                    }
                    Err(error) => {
                        catalog.note =
                            Some(format!("模型目录不是有效 JSON，仅显示当前配置：{error}"));
                    }
                }
            }
            Err(error) => {
                catalog.note = Some(format!(
                    "未读取模型目录 {}：{error}",
                    catalog_path.display()
                ));
            }
        }
    }
    if let Some(current) = &catalog.current {
        push_model(&mut catalog.models, current, current, "当前配置");
    }
    if catalog.models.is_empty() {
        catalog.note = Some("未发现模型定义；将使用 CLI 自身默认模型。".into());
    }
    catalog
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn home(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "conduit-models-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn claude_models_dedupe_mappings_and_ignore_secrets() {
        let home = home("claude");
        let directory = home.join(".claude");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("settings.json"),
            serde_json::to_vec(&serde_json::json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "must-not-be-read",
                    "ANTHROPIC_BASE_URL": "https://example.invalid",
                    "ANTHROPIC_MODEL": "proxy-model",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "proxy-model",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "sonnet-model",
                    "CLAUDE_CODE_SUBAGENT_MODEL": "subagent-model"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let catalog = load_from(Provider::Claude, &home);
        assert_eq!(catalog.current.as_deref(), Some("proxy-model"));
        assert_eq!(catalog.endpoint.as_deref(), Some("https://example.invalid"));
        assert_eq!(catalog.models.len(), 3);
        assert_eq!(catalog.models[0].id, "proxy-model");
        assert!(catalog.models[0].note.contains("当前默认"));
        assert!(catalog.models[0].note.contains("Opus"));
        assert_eq!(catalog.label("sonnet-model"), "sonnet-model");
        assert_eq!(catalog.label("unknown"), "unknown");
        assert!(catalog.error.is_none());
    }

    #[test]
    fn codex_catalog_resolves_relative_path_and_hides_hidden_models() {
        let home = home("codex");
        let directory = home.join(".codex");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("config.toml"),
            r#"model = "gpt-current"
model_provider = "custom"
model_reasoning_effort = "high"
model_catalog_json = "catalog.json"
[model_providers.custom]
base_url = "https://example.invalid"
"#,
        )
        .unwrap();
        fs::write(
            directory.join("catalog.json"),
            serde_json::to_vec(&serde_json::json!({"models": [
                {"slug": "gpt-current", "display_name": "GPT Current", "default_reasoning_level": "high"},
                {"slug": "gpt-other", "display_name": "GPT Other", "default_reasoning_level": "low"},
                {"slug": "gpt-hidden", "display_name": "Hidden", "visibility": "hide"}
            ]}))
            .unwrap(),
        )
        .unwrap();
        let catalog = load_from(Provider::Codex, &home);
        assert_eq!(catalog.current.as_deref(), Some("gpt-current"));
        assert_eq!(catalog.endpoint.as_deref(), Some("https://example.invalid"));
        assert_eq!(
            catalog.catalog_path.as_deref(),
            Some(directory.join("catalog.json").as_path())
        );
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.label("gpt-current"), "GPT Current");
        assert_eq!(catalog.models[0].note, "推理 high · 当前配置");
        assert!(catalog.models.iter().all(|model| model.id != "gpt-hidden"));
        assert!(
            catalog
                .detail
                .as_deref()
                .unwrap()
                .contains("provider custom")
        );
        assert!(catalog.error.is_none());
    }

    #[test]
    fn synced_models_merge_without_losing_cli_notes() {
        let mut catalog = ModelCatalog {
            current: Some("shared-model".into()),
            ..Default::default()
        };
        push_model(
            &mut catalog.models,
            "shared-model",
            "Shared Model",
            "当前配置",
        );
        let merged = catalog.with_synced(
            "CCS · anyrouter",
            &[
                ModelOption {
                    id: "shared-model".into(),
                    label: "Shared Model".into(),
                    note: "上下文 1.1M".into(),
                    context_window: None,
                },
                ModelOption {
                    id: "ccs-only".into(),
                    label: "CCS Only".into(),
                    note: String::new(),
                    context_window: None,
                },
            ],
        );
        assert_eq!(merged.models.len(), 2);
        assert!(merged.models[0].note.contains("当前配置"));
        assert!(merged.models[0].note.contains("上下文 1.1M"));
        assert!(merged.models[0].note.contains("CCS · anyrouter"));
        assert_eq!(merged.label("ccs-only"), "CCS Only");
        assert_eq!(merged.models[1].note, "CCS · anyrouter");
    }

    #[test]
    fn context_suffix_supports_k_and_m_and_rejects_invalid_or_overflow() {
        for (id, expected) in [
            ("model[200K]", Some(200_000)),
            ("model[1M]", Some(1_000_000)),
            ("model[2M]", Some(2_000_000)),
            ("model[200K", None),
            ("model200K]", None),
            ("model[200K]extra", None),
            ("model[0K]", None),
            ("model[+1M]", None),
            ("model[-1M]", None),
            ("model[1.5M]", None),
            ("model[M]", None),
            ("model[18446744073709551615M]", None),
            ("model[18446744073709551616K]", None),
        ] {
            assert_eq!(ModelCatalog::default().context_window(id), expected, "{id}");
        }
    }

    #[test]
    fn synced_context_window_survives_merge_for_existing_and_new_models() {
        let mut catalog = ModelCatalog::default();
        push_model(&mut catalog.models, "shared", "Shared", "CLI");
        let merged = catalog.with_synced(
            "CCS",
            &[
                ModelOption {
                    id: "shared".into(),
                    label: "Shared".into(),
                    note: String::new(),
                    context_window: Some(200_000),
                },
                ModelOption {
                    id: "synced-only".into(),
                    label: "Synced".into(),
                    note: String::new(),
                    context_window: Some(1_000_000),
                },
            ],
        );
        assert_eq!(merged.context_window("shared"), Some(200_000));
        assert_eq!(merged.context_window("synced-only"), Some(1_000_000));
    }

    #[test]
    fn brand_of_resolves_each_marked_family() {
        for (id, brand) in [
            ("claude-sonnet-4-6", ModelBrand::Anthropic),
            ("claude-fable-5-1-reversed[1M]", ModelBrand::Anthropic),
            ("gpt-6.1-sol", ModelBrand::OpenAI),
            ("gpt-6-astra-cc-format", ModelBrand::OpenAI),
            ("grok-4.7[1M]", ModelBrand::Xai),
            ("kimi-k3[1M]", ModelBrand::Kimi),
            ("deepseek-v4-flash", ModelBrand::DeepSeek),
            ("deepseek-reasoner", ModelBrand::DeepSeek),
            ("glm-5.3[1M]", ModelBrand::Zhipu),
            ("qwen3-max", ModelBrand::Qwen),
            ("MiniMax-M2", ModelBrand::MiniMax),
            ("doubao-seed-1-6", ModelBrand::Doubao),
            ("Seed-OSS-36B-Instruct", ModelBrand::Doubao),
            ("hunyuan-t1", ModelBrand::Hunyuan),
            ("gemini-2.5-pro", ModelBrand::Gemini),
            ("step-3.5-flash", ModelBrand::Step),
            ("stepfun/step3", ModelBrand::Step),
            ("mistralai/mistral-large", ModelBrand::Mistral),
            ("codestral-latest", ModelBrand::Mistral),
            ("meta-llama/llama-4-scout", ModelBrand::Meta),
            ("cohere/command-a", ModelBrand::Cohere),
            ("google/gemma-3-27b-it", ModelBrand::Gemma),
            ("nvidia/nemotron-70b", ModelBrand::Nvidia),
            ("nvidia/llama-3.1-nemotron-70b", ModelBrand::Nvidia),
            ("perplexity/sonar-pro", ModelBrand::Perplexity),
            ("ai21/jamba-large", ModelBrand::Ai21),
            ("baichuan4", ModelBrand::Baichuan),
            ("01-ai/yi-large", ModelBrand::Yi),
            ("sparkdesk-v4", ModelBrand::Spark),
            ("amazon.nova-pro-v1:0", ModelBrand::Nova),
            ("ernie-4.5", ModelBrand::Wenxin),
            ("openrouter/moonshotai/kimi-k2", ModelBrand::Kimi),
            ("o3-mini", ModelBrand::OpenAI),
        ] {
            assert_eq!(brand_of(id), brand, "{id}");
        }
        // Unmarked or proxy-rewritten IDs must not borrow another vendor's mark.
        for id in [
            "proxy-model",
            "ccs-only",
            "custom-route",
            "my-stepper",
            "buying-guide",
            "custom-o123",
            "version-o35",
            "notgpt",
        ] {
            assert_eq!(brand_of(id), ModelBrand::Other, "{id}");
        }
    }

    #[test]
    fn missing_and_invalid_configs_are_reported_without_models() {
        let home = home("missing");
        let claude = load_from(Provider::Claude, &home);
        assert!(claude.models.is_empty());
        assert!(claude.note.is_some());
        assert!(claude.error.is_none());

        let directory = home.join(".codex");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("config.toml"), "model = [").unwrap();
        let codex = load_from(Provider::Codex, &home);
        assert!(codex.models.is_empty());
        assert!(codex.error.as_deref().unwrap().contains("TOML"));
    }
}
