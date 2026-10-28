//! OpenCode owns provider discovery, project JSONC and model availability.
//! Ask its CLI instead of reconstructing those rules from CCS/local files.
use super::{ModelCatalog, push_model_with_context};
use crate::{cli, domain::Provider};
use anyhow::{Context, Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, SystemTime},
};

pub const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedCatalog {
    workspace: PathBuf,
    executable: PathBuf,
    fetched_at: SystemTime,
    models: Vec<super::ModelOption>,
    note: Option<String>,
}

fn cache_path(cwd: &Path, executable: &Path) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut key = std::collections::hash_map::DefaultHasher::new();
    cwd.hash(&mut key);
    executable.hash(&mut key);
    std::fs::metadata(executable)
        .and_then(|m| m.modified())
        .ok()
        .hash(&mut key);
    crate::store::Store::path()
        .with_file_name("opencode-models")
        .join(format!("{:016x}.json", key.finish()))
}

fn fresh(fetched_at: SystemTime) -> bool {
    fetched_at.elapsed().is_ok_and(|age| age < CACHE_TTL)
}

pub struct OpenCodeModelsProbe {
    pub events: Receiver<ModelCatalog>,
    cancelled: Arc<AtomicBool>,
}

impl OpenCodeModelsProbe {
    pub fn start(settings: cli::CliSettings, cwd: PathBuf, refresh: bool) -> Self {
        let (sender, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        thread::spawn(move || {
            let result = load_cached(&settings, &cwd, &cancel, refresh);
            let catalog = result.unwrap_or_else(|error| ModelCatalog {
                error: Some(format!(
                    "OpenCode 原生模型目录读取失败：{error:#}；请检查 CLI 配置后刷新"
                )),
                ..Default::default()
            });
            let _ = sender.send(catalog);
        });
        Self { events, cancelled }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Drop for OpenCodeModelsProbe {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn load_cached(
    settings: &cli::CliSettings,
    cwd: &Path,
    cancelled: &AtomicBool,
    refresh: bool,
) -> Result<ModelCatalog> {
    let executable = cli::resolve(Provider::OpenCode, settings)?.path;
    let path = cache_path(cwd, &executable);
    if !refresh
        && let Ok(bytes) = std::fs::read(&path)
        && let Ok(cache) = serde_json::from_slice::<CachedCatalog>(&bytes)
        && cache.workspace == cwd
        && cache.executable == executable
        && fresh(cache.fetched_at)
    {
        return Ok(ModelCatalog {
            fetched_at: Some(cache.fetched_at),
            config_path: executable,
            models: cache.models,
            note: cache.note,
            ..Default::default()
        });
    }
    let catalog = load(settings, cwd, cancelled)?;
    if !cancelled.load(Ordering::Acquire) {
        let cache = CachedCatalog {
            workspace: cwd.into(),
            executable,
            fetched_at: catalog.fetched_at.unwrap(),
            models: catalog.models.clone(),
            note: catalog.note.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&cache) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, bytes);
        }
    }
    Ok(catalog)
}

fn load(settings: &cli::CliSettings, cwd: &Path, cancelled: &AtomicBool) -> Result<ModelCatalog> {
    let executable = cli::resolve(Provider::OpenCode, settings)?.path;
    let environment =
        std::collections::BTreeMap::from([("OPENCODE_DISABLE_AUTOUPDATE".into(), "1".into())]);
    // Never copy raw CLI stderr to UI: provider/plugin errors may contain
    // configuration values. Surface bounded diagnostics without credentials.
    let output = cli::capture_command(
        &executable,
        &["models".into(), "--verbose".into(), "--pure".into()],
        cwd,
        &environment,
        cancelled,
        Duration::from_secs(20),
        16 * 1024 * 1024,
    )
    .map_err(|_| anyhow::anyhow!("CLI 查询失败、超时或输出超限"))?;
    let mut catalog = parse_catalog(&output)?;
    catalog.config_path = executable;
    catalog.fetched_at = Some(SystemTime::now());
    Ok(catalog)
}

fn parse_catalog(output: &str) -> Result<ModelCatalog> {
    let mut rest = output.trim_start_matches('\u{feff}').trim();
    let mut catalog = ModelCatalog::default();
    while !rest.is_empty() {
        let (id, tail) = rest
            .split_once('\n')
            .context("CLI 模型目录缺少模型元数据")?;
        let id = id.trim();
        let (provider, model) = id.split_once('/').context("CLI 模型 ID 格式无效")?;
        if provider.is_empty() || model.is_empty() || id.chars().any(char::is_whitespace) {
            bail!("CLI 模型 ID 格式无效");
        }
        // --verbose emits provider/model followed by a pretty-printed object,
        // not one JSON document. Stream one object and advance by byte offset.
        let mut objects =
            serde_json::Deserializer::from_str(tail.trim_start()).into_iter::<serde_json::Value>();
        let metadata = objects
            .next()
            .context("CLI 模型元数据为空")?
            .context("CLI 模型元数据不是有效 JSON")?;
        if !metadata.is_object() {
            bail!("CLI 模型元数据格式无效");
        }
        push_model_with_context(
            &mut catalog.models,
            id,
            metadata["name"].as_str().unwrap_or(id),
            "OpenCode CLI",
            metadata["limit"]["context"].as_u64(),
        );
        rest = tail.trim_start()[objects.byte_offset()..].trim_start();
    }
    if catalog.models.is_empty() {
        catalog.note =
            Some("OpenCode CLI 未返回可用模型，请先在 OpenCode 配置 Provider 后刷新".into());
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_cache_survives_restart_expires_daily_and_manual_refresh_bypasses_it() {
        let root =
            std::env::temp_dir().join(format!("conduit-catalog-cache-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("fake-opencode.exe");
        // Invalid executable: a cache hit must never try to launch it.
        std::fs::write(&executable, b"not executable").unwrap();
        let settings = cli::CliSettings {
            opencode_path: Some(executable),
            ..Default::default()
        };
        let executable = cli::resolve(Provider::OpenCode, &settings).unwrap().path;
        let path = cache_path(&root, &executable);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut cache = CachedCatalog {
            workspace: root.clone(),
            executable,
            fetched_at: SystemTime::now(),
            models: parse_catalog("test/model\n{\"name\":\"Cached\"}")
                .unwrap()
                .models,
            note: None,
        };
        let cancel = AtomicBool::new(false);
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        for _ in 0..2 {
            let catalog = load_cached(&settings, &root, &cancel, false).unwrap();
            assert_eq!(catalog.models[0].label, "Cached");
        }
        assert!(load_cached(&settings, &root, &cancel, true).is_err());
        cache.fetched_at = SystemTime::now() - CACHE_TTL - Duration::from_secs(1);
        std::fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        assert!(load_cached(&settings, &root, &cancel, false).is_err());
        assert!(!fresh(SystemTime::now() + Duration::from_secs(3600)));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_catalog_reads_cli_names_contexts_and_discards_credentials() {
        let output = "opencode/free\r\n{\"name\":\"Free model\",\"limit\":{\"context\":200000},\"options\":{\"apiKey\":\"secret\"}}\r\nproxy/model/with/slash\n{\"name\":\"Custom model\",\"limit\":{\"context\":32000}}\n";
        let catalog = parse_catalog(output).unwrap();
        assert_eq!(catalog.models.len(), 2);
        assert_eq!(catalog.models[0].label, "Free model");
        assert_eq!(catalog.models[0].context_window, Some(200_000));
        assert_eq!(catalog.models[1].id, "proxy/model/with/slash");
        assert!(catalog.current.is_none(), "never substitute a CCS default");
        assert!(!format!("{catalog:?}").contains("secret"));
    }

    #[test]
    fn native_catalog_rejects_partial_output_and_reports_empty_results() {
        for output in [
            "not-a-model\n{}",
            "opencode/test\n{",
            "opencode/test\n[]",
            "opencode/test",
        ] {
            assert!(parse_catalog(output).is_err(), "{output}");
        }
        let catalog = parse_catalog("\r\n").unwrap();
        assert!(catalog.models.is_empty());
        assert!(catalog.note.is_some());
    }

    #[test]
    #[ignore = "reads installed OpenCode native model directory; no model request"]
    fn installed_opencode_native_catalog() {
        let catalog = load(
            &cli::CliSettings::default(),
            &std::env::current_dir().unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!catalog.models.is_empty());
        assert!(catalog.models.iter().all(|m| m.id.contains('/')));
    }
}
