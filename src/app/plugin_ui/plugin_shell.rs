//! Shell content inside the existing P1 navigation, right container and status bar.
use super::*;
use conduit::plugins::{
    commands::{Capability, CommandInvocation},
    ids::PluginId,
    registry::RendererSpec,
    state::ViewInstanceKey,
};
use conduit_shell_prototype::{
    adapter::{ApprovedPackage, DevelopmentHost, HostData, WorkspaceMetadata},
    manifest::Manifest,
};
use sha2::{Digest, Sha256};

pub(super) struct ShellPluginHost {
    pub(super) host: DevelopmentHost,
    manifests: BTreeMap<PluginId, Manifest>,
    registrations: BTreeMap<PluginId, conduit::plugins::registry::Registration>,
    context_version: u64,
    data: Option<HostData>,
    tasks: BTreeMap<ViewInstanceKey, Task<()>>,
    error: Option<String>,
    result: Option<String>,
}

impl ShellPluginHost {
    pub(super) fn reconcile_registrations(&mut self, registry: &Registry) {
        let revoked: Vec<_> = self
            .registrations
            .iter()
            .filter(|(_, registration)| !registry.is_registered(registration))
            .map(|(owner, _)| owner.clone())
            .collect();
        for owner in revoked {
            let _ = self.host.unregister(&owner);
            self.tasks.retain(|key, _| key.owner != owner);
            self.manifests.remove(&owner);
            self.registrations.remove(&owner);
            self.result = None;
        }
    }
    pub(super) fn invalidate_context(&mut self, version: u64) {
        if self.context_version != version {
            self.stop();
            self.context_version = version;
            self.data = None;
        }
    }

    fn stop(&mut self) {
        self.tasks.clear();
        self.host.stop_all();
        self.result = None;
        self.error = None;
    }

    fn owns(&self, view: &ViewId) -> bool {
        self.manifests.values().any(|m| m.entry(view).is_some())
    }
}

impl PluginUi {
    pub(super) fn stop_shell(&mut self) {
        if let Some(shell) = &mut self.shell {
            shell.stop();
        }
    }

    fn with_shell(packages: Vec<ApprovedPackage>) -> anyhow::Result<Self> {
        let core = builtins::core()?;
        let manifests: BTreeMap<_, _> = packages
            .iter()
            .map(|package| {
                let manifest = package.manifest();
                (manifest.owner().clone(), manifest.clone())
            })
            .collect();
        // One UI registry owns all three surfaces. The runtime revalidates the
        // same declarations at the VM boundary; no callbacks register UI entries.
        let mut catalog = Manifest::host_catalog()?;
        for view in &core.views {
            catalog.allow_renderer(
                &view.renderer,
                RendererSpec {
                    owner: core.owner.id().clone(),
                    location: view.location,
                    scope: view.scope,
                },
            )?;
        }
        for package in &packages {
            package.manifest().allow_renderers(&mut catalog)?;
        }
        let mut registry = Registry::new(catalog);
        registry.register(core, [Capability::SessionSummary].into())?;
        for package in &packages {
            let manifest = package.manifest();
            registry.register(
                manifest.contributions().clone(),
                manifest.permissions().clone(),
            )?;
        }
        let host = DevelopmentHost::new(packages, ContextInput::default(), HostData::default())?;
        let mut ui = Self::from_registry_with_example(registry, false);
        // Native and Shell examples share one PluginId and are mutually exclusive.
        for manifest in manifests.values() {
            for view in &manifest.contributions().views {
                match view.location {
                    Location::Navigation => {
                        ui.renderers.insert(
                            view.renderer.clone(),
                            NavigationAdapter {
                                render: ConduitApp::render_shell_navigation,
                                activate: |_, _| {},
                                element_id: "",
                                task_badge: false,
                                refresh_on_notify: true,
                            },
                        );
                    }
                    Location::Right => {
                        ui.right_renderers.insert(
                            view.renderer.clone(),
                            right::RightAdapter {
                                render: |app, key, _, cx| {
                                    right::RightBody::Content(app.render_shell_view(key, cx)).into()
                                },
                                icon: FullIconName::LayoutDashboard,
                                close_id: "close-shell-view",
                                with_branches: true,
                                activate: |app, window, cx| {
                                    if let Some(key) = app.plugins.right_key().cloned() {
                                        app.open_shell_view(&key.view, window, cx);
                                    }
                                },
                                deactivate: |_, _, _| {},
                            },
                        );
                    }
                }
            }
            for command in &manifest.contributions().commands {
                ui.bind_command(command.id.as_str(), ConduitApp::dispatch_shell_command);
            }
        }
        let registrations = manifests
            .keys()
            .map(|owner| {
                (
                    owner.clone(),
                    ui.registry.registration(owner).unwrap().clone(),
                )
            })
            .collect();
        ui.shell = Some(ShellPluginHost {
            host,
            manifests,
            registrations,
            context_version: 0,
            data: None,
            tasks: BTreeMap::new(),
            error: None,
            result: None,
        });
        Ok(ui)
    }
}

impl ConduitApp {
    pub(crate) fn install_development_packages(
        &mut self,
        packages: Vec<ApprovedPackage>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let plugins = PluginUi::with_shell(packages)?;
        self.plugins.stop_instances();
        self.plugins = plugins;
        self.sync_plugin_context();
        self.refresh_shell_context(cx);
        cx.notify();
        Ok(())
    }

    pub(in crate::app) fn refresh_shell_context(&mut self, cx: &mut Context<Self>) {
        if self.plugins.shell.is_none() {
            return;
        }
        let input = self.plugins.snapshot.input().clone();
        let workspace = input.workspace.as_ref().and_then(|key| {
            self.store
                .workspaces
                .iter()
                .find(|workspace| workspace.path == key.path())
                .map(|workspace| WorkspaceMetadata {
                    id: format!(
                        "workspace-{:x}",
                        Sha256::digest(key.path().as_os_str().as_encoded_bytes())
                    ),
                    name: {
                        let name = status::bounded_text(&workspace.name(), 160);
                        if name.trim().is_empty() {
                            conduit::i18n::text_static("git_compare.workspace").into()
                        } else {
                            name
                        }
                    },
                    session_count: u32::try_from(workspace.sessions.len()).unwrap_or(u32::MAX),
                })
        });
        let data = HostData { workspace };
        let visible = if self.startup.is_none() {
            self.plugins.visible_instance_keys()
        } else {
            vec![]
        };
        let shell = self.plugins.shell.as_mut().unwrap();
        // Metadata equality prevents a ScriptView refresh on every 33ms host poll.
        if shell.host.context().input() != &input || shell.data.as_ref() != Some(&data) {
            match shell.host.set_context(input, data.clone(), cx) {
                Ok(()) => shell.data = Some(data),
                Err(error) => {
                    shell.stop();
                    shell.error = Some(format!("{error:#}"));
                }
            }
        }
        shell.host.set_visible(&visible);
        shell.tasks.retain(|key, _| visible.contains(key));
        let faults = shell.host.reap_faults();
        if !faults.is_empty() {
            shell
                .tasks
                .retain(|key, _| !faults.iter().any(|(owner, _)| owner == &key.owner));
            shell.error = Some(
                faults
                    .into_iter()
                    .map(|(owner, error)| format!("{owner}: {error}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            cx.notify();
        }
    }

    fn open_shell_view(
        &mut self,
        id: &ViewId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_plugin_context();
        self.refresh_shell_context(cx);
        if self
            .plugins
            .registry
            .prepare_view(id, &self.plugins.snapshot)
            .is_err()
        {
            return false;
        }
        let Some(shell) = &mut self.plugins.shell else {
            return false;
        };
        if !shell.owns(id) {
            return false;
        }
        let result = shell.host.open_view(id, window, cx);
        let success = result.is_ok();
        shell.error = result.err().map(|error| format!("{error:#}"));
        self.plugins.reconcile_instance_visibility();
        cx.notify();
        success
    }

    pub(super) fn dispatch_shell_activity(
        &mut self,
        request: &ActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let ActionRequest::OpenView(view) = request else {
            return false;
        };
        if !self
            .plugins
            .shell
            .as_ref()
            .is_some_and(|shell| shell.owns(&view.key().view))
        {
            return false;
        }
        self.sync_plugin_context();
        if self
            .plugins
            .registry
            .validate_view(view, &self.plugins.snapshot)
            .is_err()
        {
            return true;
        }
        self.dispatch_navigation(request, cx);
        self.open_shell_view(&view.key().view, window, cx);
        true
    }

    fn dispatch_shell_command(
        &mut self,
        invocation: &CommandInvocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.refresh_shell_context(cx);
        let result = (|| -> anyhow::Result<_> {
            let shell = self
                .plugins
                .shell
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Shell host stopped"))?;
            let (owner, binding) = shell
                .manifests
                .iter()
                .find_map(|(owner, manifest)| {
                    manifest
                        .binding(invocation.command())
                        .map(|binding| (owner.clone(), binding.clone()))
                })
                .ok_or_else(|| anyhow::anyhow!("Shell command unavailable"))?;
            // Mount the target container before creating the result ticket: the
            // container's visibility reconciliation must not cancel a new ticket.
            let opened = match self
                .plugins
                .registry
                .view(&binding.view)
                .map(|view| view.location)
            {
                Some(Location::Navigation) => self.plugins.select(&binding.view).is_ok(),
                Some(Location::Right) => self.open_right_view(&binding.view, window, cx),
                None => false,
            };
            anyhow::ensure!(opened, "command target cannot be shown");
            self.refresh_shell_context(cx);
            let shell = self.plugins.shell.as_mut().unwrap();
            let command = shell.host.prepare_command(
                &owner,
                invocation.command(),
                invocation.args().clone(),
            )?;
            shell.host.dispatch(command, window, cx)
        })();
        let pending = match result {
            Ok(value) => value,
            Err(error) => {
                if let Some(shell) = &mut self.plugins.shell {
                    shell.error = Some(format!("{error:#}"));
                }
                cx.notify();
                return false;
            }
        };
        let key = pending.target().clone();
        let task_key = key.clone();
        let task = cx.spawn(async move |app, cx| {
            let completed = pending.wait().await;
            let _ = app.update(cx, |app, cx| {
                app.sync_plugin_context();
                app.refresh_shell_context(cx);
                if let Some(shell) = &mut app.plugins.shell {
                    let previous_error = shell.error.clone();
                    match shell.host.accept(completed, cx) {
                        Ok(Some(value)) => {
                            shell.result = Some(format!("{value:?}"));
                            shell.error = None;
                        }
                        Ok(None) => {}
                        Err(error) => shell.error = Some(format!("{error:#}")),
                    }
                    shell.tasks.remove(&task_key);
                    // Successful replies only update bookkeeping. Script changes
                    // notify their own view; repaint the host for error changes.
                    if shell.error != previous_error {
                        cx.notify();
                    }
                }
            });
        });
        self.plugins.shell.as_mut().unwrap().tasks.insert(key, task);
        cx.notify();
        true
    }

    fn render_shell_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.plugins
            .state
            .navigation_key()
            .cloned()
            .map(|key| self.render_shell_view(&key, cx))
            .unwrap_or_else(|| {
                div()
                    .child(conduit::i18n::text_static(
                        "plugin_ui.example.select_a_workspace",
                    ))
                    .into_any_element()
            })
    }

    fn render_shell_view(&mut self, key: &ViewInstanceKey, cx: &mut Context<Self>) -> AnyElement {
        self.refresh_shell_context(cx);
        let shell = self.plugins.shell.as_ref().unwrap();
        let content = shell.host.view(key);
        if let Some(view) = &content {
            view.update(cx, |view, cx| view.set_fill_container(true, cx));
        }
        let id = key.view.clone();
        let selector = format!("shell-view-{}", key.view);
        let stop_selector = format!("shell-stop-{}", key.view);
        let reopen_selector = format!("shell-reopen-{}", key.view);
        v_flex()
            .debug_selector(move || selector.clone())
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                h_flex().p_2().gap_2().flex_shrink_0().child(
                    Button::new(SharedString::from(stop_selector.clone()))
                        .debug_selector(move || stop_selector.clone())
                        .small()
                        .ghost()
                        .label(conduit::i18n::text_static(
                            "plugin_ui.plugin_shell.stop_development_plugin",
                        ))
                        .on_click(cx.listener(|app, _, window, cx| app.disable_shell(window, cx))),
                ),
            )
            .children(shell.error.as_ref().map(|error| {
                div().p_2().child(conduit::i18n::format(
                    "plugin_ui.plugin_shell.plugin_error",
                    &[("error", format!("{}", error))],
                ))
            }))
            .child(match content {
                Some(view) => div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    // Host input only changes its composer. ScriptView already
                    // notifies on script/retained-control changes; GPUI also
                    // invalidates this cache for bounds, style and refreshes.
                    .child(view.cached(StyleRefinement::default().size_full()))
                    .into_any_element(),
                None => div()
                    .p_3()
                    .child(
                        Button::new(SharedString::from(reopen_selector.clone()))
                            .debug_selector(move || reopen_selector.clone())
                            .label(conduit::i18n::text_static(
                                "plugin_ui.plugin_shell.open_plugin_view",
                            ))
                            .disabled(!self.plugins.snapshot.writable())
                            .on_click(cx.listener(move |app, _, window, cx| {
                                app.open_shell_view(&id, window, cx);
                            })),
                    )
                    .into_any_element(),
            })
            .into_any_element()
    }

    fn disable_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(mut shell) = self.plugins.shell.take() {
            shell.stop();
            for owner in shell.manifests.keys() {
                if let Some(registration) = self.plugins.registry.registration(owner).cloned() {
                    let _ = self.plugins.registry.unregister(&registration);
                }
            }
        }
        gpui_shell::ShellRoot::update(window, cx, |root, window, cx| {
            root.clear_stopped_overlays(window, cx);
        });
        self.sync_plugin_context();
        cx.notify();
    }
}

#[cfg(all(test, feature = "ui-tests"))]
#[path = "plugin_shell/tests.rs"]
pub(in crate::app) mod tests;
