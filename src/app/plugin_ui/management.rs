//! Local data-plugin lifecycle and cached file icon contributions.
use super::*;
use conduit::plugin_packages::{self as packages, Discovery, Package};
use conduit::plugins::registry::Registration;
mod catalog;
pub(in crate::app) use catalog::Filter as CatalogFilter;

pub(in crate::app) struct Installed {
    pub package: Package,
    pub registration: Option<Registration>,
    icons: BTreeMap<String, Icon>,
}
#[derive(Default)]
pub(in crate::app) struct Manager {
    pub catalog: catalog::CatalogUi,
    pub entries: Vec<Installed>,
    pub errors: Vec<String>,
    pub busy: bool,
    pub initialized: bool,
    pub feedback: Option<String>,
    pub revision: u64,
}
enum Operation {
    Refresh,
    Install(PathBuf),
    InstallCatalog(Box<packages::catalog::Entry>),
    Uninstall(String),
}
pub(in crate::app) fn installation_root() -> PathBuf {
    Store::path()
        .parent()
        .expect("state directory")
        .join("plugins")
}
impl PluginUi {
    fn apply_packages(
        &mut self,
        discovery: Discovery,
        disabled: &std::collections::BTreeSet<String>,
    ) {
        for entry in &self.packages.entries {
            if let Some(handle) = &entry.registration {
                let _ = self.registry.unregister(handle);
            }
        }
        self.packages.entries.clear();
        self.packages.errors = discovery.errors;
        for package in discovery.packages {
            let registration = if disabled.contains(&package.manifest.id) {
                None
            } else {
                match package
                    .contributions()
                    .and_then(|set| Ok(self.registry.register(set, Default::default())?))
                {
                    Ok(handle) => Some(handle),
                    Err(error) => {
                        self.packages
                            .errors
                            .push(format!("{}: {error}", package.manifest.name));
                        None
                    }
                }
            };
            let icons = package
                .assets
                .iter()
                .map(|(key, bytes)| (key.clone(), Icon::default().data(bytes)))
                .collect();
            self.packages.entries.push(Installed {
                package,
                registration,
                icons,
            });
        }
        self.packages.initialized = true;
        self.packages.revision = self.packages.revision.wrapping_add(1);
    }
}
impl ConduitApp {
    pub(in crate::app) fn refresh_installed_plugins(&mut self, cx: &mut Context<Self>) {
        self.run_plugin_operation(Operation::Refresh, cx);
    }
    fn run_plugin_operation(&mut self, operation: Operation, cx: &mut Context<Self>) {
        if self.plugins.packages.busy || self.closing {
            return;
        }
        if !matches!(operation, Operation::Refresh) && !self.writable {
            return;
        }
        self.plugins.packages.busy = true;
        let root = installation_root();
        cx.spawn(async move |this, cx| {
            let (result, discovery) = cx
                .background_executor()
                .spawn(async move {
                    let result = match operation {
                        Operation::Refresh => Ok(None),
                        Operation::Install(source) => {
                            packages::install(&source, &root).map(|id| Some((id, true)))
                        }
                        Operation::InstallCatalog(entry) => {
                            packages::catalog::install(entry.as_ref(), &root)
                                .map(|id| Some((id, true)))
                        }
                        Operation::Uninstall(id) => {
                            packages::uninstall(&root, &id).map(|()| Some((id, false)))
                        }
                    };
                    (result, packages::discover(&root))
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.plugins.packages.busy = false;
                let mut changed = false;
                match result {
                    Ok(Some((id, installed))) => {
                        app.store.ui.plugins.disabled.remove(&id);
                        if !installed
                            && app.store.ui.plugins.file_icon_theme.as_deref() == Some(&id)
                        {
                            app.store.ui.plugins.file_icon_theme = None;
                        }
                        app.plugins.packages.feedback = Some(conduit::i18n::text(if installed {
                            "settings.plugins.installed"
                        } else {
                            "settings.plugins.uninstalled"
                        }));
                        changed = true;
                    }
                    Ok(None) => app.plugins.packages.feedback = None,
                    Err(error) => app.plugins.packages.feedback = Some(format!("{error:#}")),
                }
                app.plugins
                    .apply_packages(discovery, &app.store.ui.plugins.disabled);
                if app
                    .store
                    .ui
                    .plugins
                    .file_icon_theme
                    .as_ref()
                    .is_some_and(|id| {
                        !app.plugins
                            .packages
                            .entries
                            .iter()
                            .any(|e| e.package.manifest.id == *id && e.registration.is_some())
                    })
                {
                    app.store.ui.plugins.file_icon_theme = None;
                    changed = true;
                }
                if changed && app.writable {
                    app.persist();
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(in crate::app) fn install_plugin_folder(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.plugins.packages.busy {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(conduit::i18n::text_static("settings.plugins.select_folder").into()),
        });
        self.plugins.packages.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = paths.await;
            let _ = this.update_in(cx, |app, _, cx| {
                app.plugins.packages.busy = false;
                match result {
                    Ok(Ok(Some(paths))) if !paths.is_empty() => {
                        app.run_plugin_operation(Operation::Install(paths[0].clone()), cx)
                    }
                    Ok(Ok(_)) => {}
                    _ => {
                        app.plugins.packages.feedback =
                            Some(conduit::i18n::text("settings.plugins.picker_failed"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(in crate::app) fn install_catalog_plugin(
        &mut self,
        entry: packages::catalog::Entry,
        cx: &mut Context<Self>,
    ) {
        self.run_plugin_operation(Operation::InstallCatalog(Box::new(entry)), cx);
    }
    pub(in crate::app) fn uninstall_plugin(&mut self, id: String, cx: &mut Context<Self>) {
        self.run_plugin_operation(Operation::Uninstall(id), cx);
    }
    pub(in crate::app) fn set_plugin_enabled(
        &mut self,
        id: &str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.plugins.packages.busy {
            return;
        }
        let Some(entry) = self
            .plugins
            .packages
            .entries
            .iter_mut()
            .find(|e| e.package.manifest.id == id)
        else {
            return;
        };
        if enabled {
            if entry.registration.is_some() {
                return;
            }
            match entry
                .package
                .contributions()
                .and_then(|set| Ok(self.plugins.registry.register(set, Default::default())?))
            {
                Ok(handle) => {
                    entry.registration = Some(handle);
                    self.store.ui.plugins.disabled.remove(id);
                }
                Err(error) => {
                    self.plugins.packages.feedback = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            if let Some(handle) = entry.registration.take() {
                let _ = self.plugins.registry.unregister(&handle);
            }
            self.store.ui.plugins.disabled.insert(id.to_owned());
            if self.store.ui.plugins.file_icon_theme.as_deref() == Some(id) {
                self.store.ui.plugins.file_icon_theme = None;
            }
        }
        self.plugins.packages.revision = self.plugins.packages.revision.wrapping_add(1);
        self.persist();
        cx.notify();
    }
    pub(in crate::app) fn select_file_icon_theme(
        &mut self,
        id: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.plugins.packages.busy {
            return;
        }
        if id.as_ref().is_some_and(|id| {
            !self
                .plugins
                .packages
                .entries
                .iter()
                .any(|e| e.package.manifest.id == *id && e.registration.is_some())
        }) {
            return;
        }
        self.store.ui.plugins.file_icon_theme = id;
        self.plugins.packages.revision = self.plugins.packages.revision.wrapping_add(1);
        self.persist();
        cx.notify();
    }
    pub(in crate::app) fn file_tree_icon(
        &self,
        path: &Path,
        directory: bool,
        expanded: bool,
    ) -> Icon {
        if let Some(id) = &self.store.ui.plugins.file_icon_theme
            && let Some(entry) = self.plugins.packages.entries.iter().find(|entry| {
                &entry.package.manifest.id == id
                    && entry
                        .registration
                        .as_ref()
                        .is_some_and(|r| self.plugins.registry.is_registered(r))
            })
            && let Some(key) = entry
                .package
                .manifest
                .file_icons
                .resolve(path, directory, expanded)
            && let Some(icon) = entry.icons.get(key)
        {
            let definition = &entry.package.manifest.file_icons.icons[key];
            let color = if is_dark_mode() {
                &definition.dark
            } else {
                &definition.light
            };
            return icon.clone().text_color(rgb(color
                .as_deref()
                .and_then(packages::color)
                .unwrap_or(palette().muted)));
        }
        Icon::new(if directory {
            if expanded {
                FullIconName::FolderOpen
            } else {
                FullIconName::Folder
            }
        } else {
            file_ref::icon_for(path)
        })
        .text_color(rgb(palette().muted))
    }
}
