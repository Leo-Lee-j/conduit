//! Read-only model catalogs from the local cc-switch database.
//!
//! Queries extract only model identifiers, display names, context sizes and
//! visibility. Only execution resolves the explicitly selected provider
//! credentials; those are never stored in application state or displayed.
use crate::{
    domain::Provider,
    models::{self, ModelOption},
};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct CcsProvider {
    pub provider: Provider,
    pub name: String,
    pub id: String,
    pub current: bool,
    pub models: Vec<ModelOption>,
    pub base_url: String,
}

#[derive(Clone, Debug, Default)]
pub struct CcsCatalog {
    pub database_path: PathBuf,
    pub providers: Vec<CcsProvider>,
    pub error: Option<String>,
    pub note: Option<String>,
}

impl CcsCatalog {
    pub fn imported_providers(&self) -> impl Iterator<Item = crate::providers::ModelProvider> + '_ {
        self.providers
            .iter()
            .filter(|p| p.provider != Provider::OpenCode)
            .map(|p| crate::providers::ModelProvider {
                id: crate::providers::ModelProvider::ccs_id(p.provider, &p.id),
                name: p.name.clone(),
                engine: p.provider,
                base_url: p.base_url.clone(),
                api_key_env: String::new(),
                models: p.models.iter().map(|m| m.id.clone()).collect(),
                source: crate::providers::ProviderSource::Ccs { id: p.id.clone() },
                model_metadata: p.models.clone(),
            })
    }
    pub fn current(&self, provider: Provider) -> Option<&CcsProvider> {
        self.providers
            .iter()
            .find(|entry| entry.provider == provider && entry.current)
    }
    #[cfg(test)]
    pub fn for_provider(&self, provider: Provider) -> impl Iterator<Item = &CcsProvider> {
        self.providers
            .iter()
            .filter(move |entry| entry.provider == provider)
    }
}

pub fn load() -> CcsCatalog {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return CcsCatalog {
            note: Some("无法定位用户目录，未读取 cc-switch。".into()),
            ..Default::default()
        };
    };
    load_from(&home.join(".cc-switch").join("cc-switch.db"))
}

/// Resolve only the explicitly selected route, without changing cc-switch's
/// current provider. Credentials stay in memory in the child process route.
pub fn runtime_route(
    path: &Path,
    engine: Provider,
    id: &str,
) -> anyhow::Result<crate::providers::RuntimeRoute> {
    use crate::providers::RuntimeRoute;
    use anyhow::{Context, bail};
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_millis(500))?;
    let app = match engine {
        Provider::Claude => "claude",
        Provider::Codex => "codex",
        Provider::OpenCode => "opencode",
    };
    let raw: String = connection
        .query_row(
            "SELECT settings_config FROM providers WHERE id = ?1 AND app_type = ?2",
            rusqlite::params![id, app],
            |row| row.get(0),
        )
        .context("所选 CCS Provider 已不存在，请重新选择")?;
    let settings: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| anyhow::anyhow!("CCS Provider 配置格式无效"))?;
    match engine {
        Provider::OpenCode => bail!("OpenCode 暂不支持 CCS 路由，请使用 CLI 默认渠道"),
        Provider::Claude => {
            let env = &settings["env"];
            let Some(url) = env["ANTHROPIC_BASE_URL"].as_str().filter(|v| !v.is_empty()) else {
                bail!(
                    "此 CCS Provider 未配置 Base URL，请使用 CLI 默认渠道或在 cc-switch 补全配置"
                );
            };
            let key = env["ANTHROPIC_AUTH_TOKEN"]
                .as_str()
                .or_else(|| env["ANTHROPIC_API_KEY"].as_str())
                .map(str::to_owned);
            let mut route = RuntimeRoute::endpoint(engine, "CCS", url, key);
            route.default_model = settings["model"]
                .as_str()
                .or_else(|| env["ANTHROPIC_MODEL"].as_str())
                .map(str::to_owned);
            for (name, value) in env.as_object().into_iter().flatten() {
                if (name.starts_with("ANTHROPIC_DEFAULT_")
                    || name == "CLAUDE_CODE_SUBAGENT_MODEL"
                    || name == "ANTHROPIC_REASONING_MODEL")
                    && let Some(value) = value.as_str()
                {
                    route.env.insert(name.clone(), value.into());
                }
            }
            Ok(route)
        }
        Provider::Codex => {
            let config = settings["config"].as_str().unwrap_or("");
            let config: toml::Value = config
                .parse()
                .map_err(|_| anyhow::anyhow!("CCS Codex 配置不是有效 TOML"))?;
            let name = config
                .get("model_provider")
                .and_then(toml::Value::as_str)
                .context("此 CCS Provider 未指定 model_provider，请使用 CLI 默认渠道")?;
            let provider = config
                .get("model_providers")
                .and_then(|v| v.get(name))
                .context("CCS Provider 缺少端点定义")?;
            let url = provider
                .get("base_url")
                .and_then(toml::Value::as_str)
                .context("CCS Provider 缺少 Base URL")?;
            let key = settings
                .pointer("/auth/OPENAI_API_KEY")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    provider
                        .get("env_key")
                        .and_then(toml::Value::as_str)
                        .and_then(|key| std::env::var(key).ok())
                });
            let mut route = RuntimeRoute::endpoint(engine, name, url, key);
            route.default_model = config
                .get("model")
                .and_then(toml::Value::as_str)
                .map(str::to_owned);
            let target = &mut route.config.get_mut("model_providers").unwrap()["conduit"];
            for field in [
                "wire_api",
                "http_headers",
                "env_http_headers",
                "query_params",
                "request_max_retries",
                "stream_max_retries",
                "stream_idle_timeout_ms",
            ] {
                if let Some(value) = provider.get(field) {
                    target[field] = serde_json::to_value(value)?;
                }
            }
            Ok(route)
        }
    }
}

pub fn load_from(path: &Path) -> CcsCatalog {
    let mut catalog = CcsCatalog {
        database_path: path.to_path_buf(),
        ..Default::default()
    };
    if !path.exists() {
        catalog.note = Some("未找到 cc-switch 数据库；仅显示 CLI 当前模型。".into());
        return catalog;
    }
    let connection = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(connection) => connection,
        Err(error) => {
            catalog.error = Some(format!("无法以只读方式打开 cc-switch 数据库：{error}"));
            return catalog;
        }
    };
    if let Err(error) = connection.busy_timeout(Duration::from_millis(500)) {
        catalog.error = Some(format!("无法设置 cc-switch 读取超时：{error}"));
        return catalog;
    }
    let mut providers = match claude_providers(&connection) {
        Ok(providers) => providers,
        Err(error) => {
            catalog.error = Some(format!("读取 cc-switch Claude 模型失败：{error}"));
            return catalog;
        }
    };
    match codex_providers(&connection) {
        Ok(codex) => providers.extend(codex),
        Err(error) => {
            catalog.error = Some(format!("读取 cc-switch Codex 模型失败：{error}"));
            return catalog;
        }
    }
    if providers.is_empty() {
        catalog.note = Some("cc-switch 中没有可列出的 Claude / Codex 模型。".into());
    }
    // Synchronize public configuration metadata without querying auth fields.
    for provider in &mut providers {
        let app = match provider.provider {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::OpenCode => "opencode",
        };
        let endpoint: rusqlite::Result<Option<String>> = connection.query_row(
            "SELECT CASE WHEN app_type = 'claude' THEN json_extract(settings_config, '$.env.ANTHROPIC_BASE_URL') ELSE json_extract(settings_config, '$.config') END FROM providers WHERE id = ?1 AND app_type = ?2",
            rusqlite::params![provider.id, app], |row| row.get(0));
        provider.base_url = match (provider.provider, endpoint.ok().flatten()) {
            (Provider::Claude, Some(url)) => url,
            (Provider::Codex, Some(config)) => config
                .parse::<toml::Value>()
                .ok()
                .and_then(|v| {
                    let name = v.get("model_provider")?.as_str()?;
                    v.get("model_providers")?
                        .get(name)?
                        .get("base_url")?
                        .as_str()
                        .map(str::to_owned)
                })
                .unwrap_or_default(),
            _ => String::new(),
        };
    }
    catalog.providers = providers;
    catalog
}

fn claude_providers(connection: &Connection) -> rusqlite::Result<Vec<CcsProvider>> {
    let mut statement = connection.prepare(
        "SELECT name, is_current,
            json_extract(settings_config, '$.model'),
            json_extract(settings_config, '$.env.ANTHROPIC_MODEL'),
            json_extract(settings_config, '$.env.ANTHROPIC_DEFAULT_OPUS_MODEL'),
            json_extract(settings_config, '$.env.ANTHROPIC_DEFAULT_SONNET_MODEL'),
            json_extract(settings_config, '$.env.ANTHROPIC_DEFAULT_HAIKU_MODEL'),
            json_extract(settings_config, '$.env.ANTHROPIC_DEFAULT_FABLE_MODEL'),
            json_extract(settings_config, '$.env.CLAUDE_CODE_SUBAGENT_MODEL'),
            json_extract(settings_config, '$.env.ANTHROPIC_REASONING_MODEL'), id
         FROM providers
         WHERE app_type = 'claude' AND json_valid(settings_config)
         ORDER BY is_current DESC, sort_index, name",
    )?;
    let rows = statement.query_map([], |row| {
        let name: String = row.get(0)?;
        let current: bool = row.get::<_, i64>(1)? != 0;
        let mut models = Vec::new();
        for (index, role) in [
            "默认",
            "默认",
            "Opus",
            "Sonnet",
            "Haiku",
            "Fable",
            "子代理",
            "推理",
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(id) = row
                .get::<_, Option<String>>(index + 2)?
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
            {
                models::push_model(&mut models, &id, &id, role);
            }
        }
        let id: String = row.get(10)?;
        Ok((name, current, models, id))
    })?;
    let mut providers = Vec::new();
    for row in rows {
        let (name, current, models, id) = row?;
        providers.push(CcsProvider {
            provider: Provider::Claude,
            id,
            name,
            current,
            models,
            base_url: String::new(),
        });
    }
    Ok(providers)
}

fn codex_providers(connection: &Connection) -> rusqlite::Result<Vec<CcsProvider>> {
    let mut statement = connection.prepare(
        "SELECT p.name, p.is_current,
            json_extract(m.value, '$.model'),
            json_extract(m.value, '$.displayName'),
            json_extract(m.value, '$.contextWindow'),
            json_extract(m.value, '$.visibility'), p.id
         FROM providers p, json_each(p.settings_config, '$.modelCatalog.models') m
         WHERE p.app_type = 'codex' AND json_valid(p.settings_config)
         ORDER BY p.is_current DESC, p.sort_index, p.name",
    )?;
    let rows = statement.query_map([], |row| {
        let name: String = row.get(0)?;
        let current: bool = row.get::<_, i64>(1)? != 0;
        let id: Option<String> = row.get(2)?;
        let display: Option<String> = row.get(3)?;
        let context: Option<i64> = row.get(4)?;
        let visibility: Option<String> = row.get(5)?;
        Ok((
            name,
            current,
            id,
            display,
            context,
            visibility,
            row.get::<_, String>(6)?,
        ))
    })?;
    let mut providers: Vec<CcsProvider> = Vec::new();
    for row in rows {
        let (name, current, id, display, context, visibility, provider_id) = row?;
        if visibility.as_deref() == Some("hide") {
            continue;
        }
        let Some(id) = id
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        let index = match providers
            .iter()
            .position(|entry| entry.provider == Provider::Codex && entry.id == provider_id)
        {
            Some(index) => index,
            None => {
                providers.push(CcsProvider {
                    provider: Provider::Codex,
                    id: provider_id,
                    name: name.clone(),
                    current,
                    models: Vec::new(),
                    base_url: String::new(),
                });
                providers.len() - 1
            }
        };
        let label = display
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let note = context_note(context);
        models::push_model_with_context(
            &mut providers[index].models,
            &id,
            &label,
            &note,
            context.map(|tokens| tokens.max(0) as u64),
        );
    }
    // Older CCS records may only contain config.toml, without modelCatalog.
    let mut statement = connection.prepare(
        "SELECT id, name, is_current, json_extract(settings_config, '$.config')
         FROM providers WHERE app_type = 'codex' AND json_valid(settings_config)
         ORDER BY is_current DESC, sort_index, name",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)? != 0,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;
    for row in rows {
        let (id, name, current, config) = row?;
        if providers.iter().any(|p| p.id == id) {
            continue;
        }
        let mut models = vec![];
        if let Some(config) = config.and_then(|v| v.parse::<toml::Value>().ok())
            && let Some(model) = config.get("model").and_then(toml::Value::as_str)
        {
            models::push_model(&mut models, model, model, "默认");
        }
        providers.push(CcsProvider {
            provider: Provider::Codex,
            id,
            name,
            current,
            models,
            base_url: String::new(),
        });
    }
    Ok(providers)
}

fn context_note(context: Option<i64>) -> String {
    match context {
        Some(tokens) if tokens >= 1_000_000 => {
            format!("上下文 {:.1}M", tokens as f64 / 1_000_000.0)
        }
        Some(tokens) if tokens >= 1_000 => format!("上下文 {}K", tokens / 1_000),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn database(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "conduit-ccs-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("cc-switch.db")
    }

    fn create_database(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE providers (
                    id TEXT NOT NULL,
                    app_type TEXT NOT NULL,
                    name TEXT NOT NULL,
                    settings_config TEXT NOT NULL,
                    sort_index INTEGER,
                    is_current BOOLEAN NOT NULL DEFAULT 0,
                    PRIMARY KEY (id, app_type)
                );",
            )
            .unwrap();
    }

    #[test]
    fn reads_model_fields_without_credentials() {
        let path = database("models");
        create_database(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, settings_config, sort_index, is_current)
                 VALUES (?1, 'claude', 'proxy-a', ?2, 0, 1)",
                params![
                    "claude-a",
                    serde_json::json!({
                        "env": {
                            "ANTHROPIC_AUTH_TOKEN": "claude-secret",
                            "ANTHROPIC_BASE_URL": "https://ignore.invalid",
                            "ANTHROPIC_MODEL": "claude-main",
                            "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-main",
                            "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet"
                        }
                    })
                    .to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, settings_config, sort_index, is_current)
                 VALUES (?1, 'claude', 'proxy-b', ?2, 1, 0)",
                params![
                    "claude-b",
                    serde_json::json!({"model": "claude-top"}).to_string()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO providers (id, app_type, name, settings_config, sort_index, is_current)
                 VALUES (?1, 'codex', 'codex-main', ?2, 0, 1)",
                params![
                    "codex-a",
                    serde_json::json!({
                        "auth": {"OPENAI_API_KEY": "codex-secret"},
                        "config": "ignored",
                        "modelCatalog": {"models": [
                            {"model": "gpt-main", "displayName": "GPT Main", "contextWindow": 1050000},
                            {"model": "gpt-hidden", "visibility": "hide"}
                        ]}
                    })
                    .to_string()
                ],
            )
            .unwrap();
        drop(connection);

        let catalog = load_from(&path);
        assert!(catalog.error.is_none());
        assert_eq!(catalog.providers.len(), 3);
        let claude = catalog.current(Provider::Claude).unwrap();
        assert_eq!(claude.name, "proxy-a");
        assert_eq!(claude.models.len(), 2);
        assert_eq!(claude.models[0].id, "claude-main");
        assert!(claude.models[0].note.contains("默认"));
        assert!(claude.models[0].note.contains("Opus"));
        let codex = catalog.current(Provider::Codex).unwrap();
        assert_eq!(codex.name, "codex-main");
        assert_eq!(codex.models.len(), 1);
        assert_eq!(codex.models[0].label, "GPT Main");
        assert!(codex.models[0].note.contains("1.1M"));
        let debug = format!("{catalog:?}");
        assert!(!debug.contains("claude-secret"));
        assert!(!debug.contains("codex-secret"));
        assert_eq!(claude.base_url, "https://ignore.invalid");
    }

    #[test]
    fn missing_or_invalid_database_is_reported() {
        let missing = database("missing");
        let catalog = load_from(&missing);
        assert!(catalog.note.is_some());
        assert!(catalog.error.is_none());

        let invalid = database("invalid");
        std::fs::write(&invalid, b"not a database").unwrap();
        let catalog = load_from(&invalid);
        assert!(catalog.error.is_some());
    }

    #[test]
    fn config_only_ccs_routes_use_stable_ids_and_selected_credentials() {
        let path = database("routes");
        create_database(&path);
        let connection = Connection::open(&path).unwrap();
        for (id, key, url) in [
            ("a", "fixture-a", "https://a.invalid/v1"),
            ("b", "fixture-b", "https://b.invalid/v1"),
        ] {
            let settings = serde_json::json!({"auth":{"OPENAI_API_KEY":key},"config":format!("model = \"model-{id}\"\nmodel_provider = \"proxy\"\n[model_providers.proxy]\nname = \"proxy\"\nbase_url = \"{url}\"\nwire_api = \"responses\"\n")});
            connection.execute("INSERT INTO providers (id, app_type, name, settings_config, sort_index, is_current) VALUES (?1, 'codex', 'same-name', ?2, 0, ?3)", params![id, settings.to_string(), i64::from(id == "a")]).unwrap();
        }
        let catalog = load_from(&path);
        assert_eq!(catalog.providers.len(), 2);
        assert_eq!(catalog.for_provider(Provider::Codex).count(), 2);
        assert_eq!(catalog.current(Provider::Codex).unwrap().id, "a");
        let route = runtime_route(&path, Provider::Codex, "b").unwrap();
        assert_eq!(
            route.config["model_providers"]["conduit"]["base_url"],
            "https://b.invalid/v1"
        );
        assert_eq!(route.env["CONDUIT_PROVIDER_API_KEY"], "fixture-b");
        assert_eq!(route.default_model.as_deref(), Some("model-b"));
        assert!(!format!("{catalog:?}").contains("fixture-b"));
        assert!(runtime_route(&path, Provider::Claude, "b").is_err());
        assert_eq!(
            connection
                .query_row("SELECT is_current FROM providers WHERE id='a'", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }
}
