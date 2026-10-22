//! Read-only model catalogs from the local cc-switch database.
//!
//! Queries extract only model identifiers, display names, context sizes and
//! visibility. Credentials, endpoints and full provider configurations are
//! never selected, stored or displayed.
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
    pub current: bool,
    pub models: Vec<ModelOption>,
}

#[derive(Clone, Debug, Default)]
pub struct CcsCatalog {
    pub database_path: PathBuf,
    pub providers: Vec<CcsProvider>,
    pub error: Option<String>,
    pub note: Option<String>,
}

impl CcsCatalog {
    pub fn current(&self, provider: Provider) -> Option<&CcsProvider> {
        self.providers
            .iter()
            .find(|entry| entry.provider == provider && entry.current)
    }
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
            json_extract(settings_config, '$.env.ANTHROPIC_REASONING_MODEL')
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
        Ok((name, current, models))
    })?;
    let mut providers = Vec::new();
    for row in rows {
        let (name, current, models) = row?;
        if !models.is_empty() {
            providers.push(CcsProvider {
                provider: Provider::Claude,
                name,
                current,
                models,
            });
        }
    }
    Ok(providers)
}

fn codex_providers(connection: &Connection) -> rusqlite::Result<Vec<CcsProvider>> {
    let mut statement = connection.prepare(
        "SELECT p.name, p.is_current,
            json_extract(m.value, '$.model'),
            json_extract(m.value, '$.displayName'),
            json_extract(m.value, '$.contextWindow'),
            json_extract(m.value, '$.visibility')
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
        Ok((name, current, id, display, context, visibility))
    })?;
    let mut providers: Vec<CcsProvider> = Vec::new();
    for row in rows {
        let (name, current, id, display, context, visibility) = row?;
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
            .position(|entry| entry.provider == Provider::Codex && entry.name == name)
        {
            Some(index) => index,
            None => {
                providers.push(CcsProvider {
                    provider: Provider::Codex,
                    name: name.clone(),
                    current,
                    models: Vec::new(),
                });
                providers.len() - 1
            }
        };
        let label = display
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| id.clone());
        let note = context_note(context);
        models::push_model(&mut providers[index].models, &id, &label, &note);
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
        assert!(!debug.contains("ignore.invalid"));
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
}
