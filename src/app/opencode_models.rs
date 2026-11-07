//! Workspace-specific discovery of OpenCode's native model catalog.
use super::*;

impl ConduitApp {
    fn opencode_model_workspace(&self) -> PathBuf {
        self.selected_workspace_path()
            .unwrap_or_else(std::env::temp_dir)
    }

    pub(super) fn engine_catalog(&self, engine: Provider) -> Option<&ModelCatalog> {
        if engine == Provider::OpenCode {
            self.opencode_catalogs.get(&self.opencode_model_workspace())
        } else {
            self.catalogs.get(&engine)
        }
    }

    pub(super) fn ensure_opencode_models(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.closing || self.cli_installing() {
            return;
        }
        let cwd = self.opencode_model_workspace();
        if self.opencode_model_probes.contains_key(&cwd)
            || (!refresh
                && self.opencode_catalogs.get(&cwd).is_some_and(|catalog| {
                    catalog.error.is_some()
                        || catalog.fetched_at.is_some_and(|time| {
                            time.elapsed()
                                .is_ok_and(|age| age < models::OPENCODE_CACHE_TTL)
                        })
                }))
        {
            return;
        }
        self.opencode_model_probes.insert(
            cwd.clone(),
            models::OpenCodeModelsProbe::start(self.store.cli.clone(), cwd, refresh),
        );
        cx.emit(crate::providers::ProviderCatalogChanged);
        cx.notify();
    }

    pub(super) fn poll_opencode_models(&mut self, cx: &mut Context<Self>) {
        let completed: Vec<_> = self
            .opencode_model_probes
            .iter()
            .filter_map(|(cwd, probe)| match probe.events.try_recv() {
                Ok(catalog) => Some((cwd.clone(), catalog)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    cwd.clone(),
                    ModelCatalog {
                        error: Some("OpenCode 模型目录查询已停止，请刷新重试".into()),
                        ..Default::default()
                    },
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            })
            .collect();
        if completed.is_empty() {
            return;
        }
        for (cwd, catalog) in completed {
            self.opencode_model_probes.remove(&cwd);
            self.opencode_catalogs.insert(cwd, catalog);
        }
        cx.emit(crate::providers::ProviderCatalogChanged);
        cx.notify();
    }
}
