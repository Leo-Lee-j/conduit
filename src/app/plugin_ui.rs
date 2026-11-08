//! Native navigation adapter for the backend-independent contribution registry.
use super::*;
use conduit::plugins::{
    builtins,
    context::{ContextInput, ContextTracker, PluginContext, SessionKey, WorkspaceKey},
    contributions::{ActivityAction, ActivityItem, Location},
    ids::ViewId,
    registry::{ActionRequest, Registry},
    state::WorkbenchState,
};
use std::collections::BTreeMap;

mod commands;
mod example;
pub(super) mod right;
#[cfg(conduit_shell_desktop)]
#[path = "plugin_ui/plugin_shell.rs"]
pub(super) mod shell;
mod status;
mod summary;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

#[derive(Clone, Copy)]
struct NavigationAdapter {
    render: fn(&mut ConduitApp, &mut Context<ConduitApp>) -> AnyElement,
    activate: fn(&mut ConduitApp, &mut Context<ConduitApp>),
    element_id: &'static str,
    task_badge: bool,
    refresh_on_notify: bool,
}
pub(super) struct PluginUi {
    #[cfg(conduit_shell_desktop)]
    shell: Option<shell::ShellPluginHost>,
    registry: Registry,
    context: ContextTracker,
    snapshot: PluginContext,
    state: WorkbenchState,
    renderers: BTreeMap<String, NavigationAdapter>,
    right_renderers: BTreeMap<String, right::RightAdapter>,
    renderer_registrations: BTreeMap<String, conduit::plugins::registry::Registration>,
    commands: BTreeMap<conduit::plugins::ids::CommandId, commands::NativeCommand>,
    status_providers: BTreeMap<conduit::plugins::ids::ItemId, status::StatusProvider>,
    status_width: f32,
    instances: conduit::plugins::instances::InstanceCache<example::OverviewInstance, Task<()>>,
    pub(super) dock: right::DockLayout,
    reconciled_revision: Option<u64>,
}
/// Compared by SidebarView without walking chat bodies or rebuilding the navigation.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct NavigationRevision {
    registry: u64,
    context: u64,
    selected: ViewId,
    collapsed: bool,
    running: bool,
    appearance: (u32, u32, u32),
}
impl PluginUi {
    fn from_registry(registry: Registry) -> Self {
        Self::from_registry_with_example(registry, true)
    }
    fn from_registry_with_example(registry: Registry, bind_example: bool) -> Self {
        let state = WorkbenchState::new(
            &registry,
            ViewId::new(builtins::WORKSPACES).unwrap(),
            ViewId::new(builtins::TOOL_DETAILS).unwrap(),
        )
        .expect("core fallback views");
        let mut context = ContextTracker::default();
        let snapshot = context
            .update(ContextInput::default())
            .expect("initial context")
            .clone();
        let renderers = [
            (
                builtins::WORKSPACES,
                NavigationAdapter {
                    render: ConduitApp::render_sidebar,
                    activate: |_, _| {},
                    element_id: "nav-workspaces",
                    task_badge: true,
                    refresh_on_notify: false,
                },
            ),
            (
                builtins::FILES,
                NavigationAdapter {
                    render: ConduitApp::render_file_tree,
                    activate: ConduitApp::activate_file_navigation,
                    element_id: "nav-files",
                    task_badge: false,
                    refresh_on_notify: true,
                },
            ),
            (
                builtins::GIT,
                NavigationAdapter {
                    render: ConduitApp::render_git_navigation,
                    activate: ConduitApp::refresh_git,
                    element_id: "nav-git",
                    task_badge: false,
                    refresh_on_notify: true,
                },
            ),
            (
                builtins::BRANCHES,
                NavigationAdapter {
                    render: ConduitApp::render_branch_navigation,
                    activate: |app, _| app.refresh_branches(),
                    element_id: "nav-branches",
                    task_badge: false,
                    refresh_on_notify: true,
                },
            ),
        ]
        .into_iter()
        .map(|(key, adapter)| (key.into(), adapter))
        .collect();
        let renderer_registrations = registry
            .views()
            .filter_map(|view| {
                let owner = registry.owner_for(view.id.as_str())?;
                Some((view.renderer.clone(), registry.registration(owner)?.clone()))
            })
            .collect();
        let mut ui = Self {
            #[cfg(conduit_shell_desktop)]
            shell: None,
            registry,
            context,
            snapshot,
            state,
            renderers,
            right_renderers: right::core_renderers(),
            renderer_registrations,
            commands: BTreeMap::new(),
            status_providers: BTreeMap::new(),
            status_width: 0.,
            instances: Default::default(),
            dock: right::DockLayout::default(),
            reconciled_revision: None,
        };
        commands::bind_core(&mut ui);
        status::bind_core(&mut ui);
        if bind_example {
            example::bind(&mut ui);
        }
        ui
    }
    pub(super) fn new() -> Self {
        Self::from_registry(
            builtins::register(cfg!(feature = "plugin-example"))
                .expect("valid core declarations")
                .registry,
        )
    }
    pub(super) fn selected(&self) -> &ViewId {
        self.state.navigation()
    }
    pub(super) fn is_selected(&self, id: &str) -> bool {
        self.selected().as_str() == id
    }
    pub(super) fn collapsed(&self) -> bool {
        self.state.collapsed()
    }
    pub(super) fn set_collapsed(&mut self, collapsed: bool) {
        self.state.set_collapsed(collapsed);
        self.reconcile_instance_visibility();
    }
    fn update_context(&mut self, input: ContextInput) {
        if self.snapshot.input() == &input
            && self.reconciled_revision == Some(self.registry.revision())
        {
            return;
        }
        self.snapshot = self
            .context
            .update(input)
            .expect("context version available")
            .clone();
        #[cfg(conduit_shell_desktop)]
        if let Some(shell) = &mut self.shell {
            shell.reconcile_registrations(&self.registry);
            shell.invalidate_context(self.snapshot.version());
        }
        self.state
            .reconcile(&self.registry, &self.snapshot)
            .expect("core fallbacks remain registered");
        self.commands
            .retain(|_, binding| self.registry.is_registered(&binding.registration));
        self.status_providers
            .retain(|_, binding| self.registry.is_registered(&binding.registration));
        self.renderer_registrations
            .retain(|_, registration| self.registry.is_registered(registration));
        self.renderers
            .retain(|key, _| self.renderer_registrations.contains_key(key));
        self.right_renderers
            .retain(|key, _| self.renderer_registrations.contains_key(key));
        if !self.state.right_open() {
            self.dock.drag = None;
        }
        self.reconciled_revision = Some(self.registry.revision());
    }
    pub(super) fn revision(&self) -> NavigationRevision {
        NavigationRevision {
            registry: self.registry.revision(),
            context: self.snapshot.version(),
            selected: self.selected().clone(),
            collapsed: self.collapsed(),
            running: self.snapshot.input().session_running,
            appearance: (palette().canvas, palette().sidebar_bg, palette().text),
        }
    }
    fn adapter(&self, view: &ViewId) -> Option<NavigationAdapter> {
        let descriptor = self.registry.view(view)?;
        if !self.renderer_bound(&descriptor.renderer) {
            return None;
        }
        self.renderers.get(&descriptor.renderer).copied()
    }
    fn renderer_bound(&self, key: &str) -> bool {
        self.renderer_registrations
            .get(key)
            .is_some_and(|registration| self.registry.is_registered(registration))
    }
    fn bind_renderer(&mut self, key: &str) {
        let view = self
            .registry
            .views()
            .find(|view| view.renderer == key)
            .expect("declared native renderer");
        let owner = self.registry.owner_for(view.id.as_str()).unwrap();
        self.renderer_registrations.insert(
            key.into(),
            self.registry.registration(owner).unwrap().clone(),
        );
        self.instances.retain_registered(&self.registry);
    }
    pub(super) fn navigation_refresh_on_notify(&self) -> bool {
        !self.collapsed()
            && self
                .adapter(self.selected())
                .is_some_and(|adapter| adapter.refresh_on_notify)
    }
    fn visible_instance_keys(&self) -> Vec<conduit::plugins::state::ViewInstanceKey> {
        [
            (!self.collapsed())
                .then(|| self.state.navigation_key())
                .flatten(),
            self.right_visible(self.is_selected(builtins::BRANCHES))
                .then(|| self.right_key())
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
    }
    fn reconcile_instance_visibility(&mut self) {
        let visible = self.visible_instance_keys();
        #[cfg(conduit_shell_desktop)]
        if let Some(shell) = &mut self.shell {
            shell.host.set_visible(&visible);
        }
        self.instances
            .reconcile(&self.registry, &self.snapshot, &visible, |_| true);
    }
    pub(super) fn stop_instances(&mut self) {
        self.instances.clear();
        #[cfg(conduit_shell_desktop)]
        self.stop_shell();
    }
    fn request(&self, item: &ActivityItem) -> Option<ActionRequest> {
        let request = self
            .registry
            .prepare_activity(&item.id, &self.snapshot)
            .ok()?;
        match &request {
            ActionRequest::OpenView(view) if self.adapter(&view.key().view).is_some() => {
                Some(request)
            }
            ActionRequest::Command(command) if self.command_bound(command.command()) => {
                Some(request)
            }
            _ => None,
        }
    }
    fn select(&mut self, id: &ViewId) -> conduit::plugins::Result<NavigationAdapter> {
        let request = self.registry.prepare_view(id, &self.snapshot)?;
        let view = self.registry.validate_view(&request, &self.snapshot)?;
        if view.location != Location::Navigation {
            return Err(conduit::plugins::Error("not a navigation view".into()));
        }
        let adapter = self
            .adapter(id)
            .ok_or_else(|| conduit::plugins::Error("navigation renderer unavailable".into()))?;
        self.state.open(&self.registry, &request, &self.snapshot)?;
        self.reconcile_instance_visibility();
        Ok(adapter)
    }
}

// Rail rows are 36px + 4px gap. Reserve the two fixed bottom controls,
// their gaps, the spacer gap and 4px top padding. An overflow button needs one slot.
fn visible_activity_count(count: usize, height: f32) -> usize {
    let slots = ((height - 78.).max(0.) / 40.).floor() as usize;
    if count <= slots {
        count
    } else {
        slots.saturating_sub(1)
    }
}
fn activity_icon(name: &str) -> Option<FullIconName> {
    match name {
        "folder" => Some(FullIconName::Folder),
        "file" => Some(FullIconName::File),
        "git-fork" => Some(FullIconName::GitFork),
        "layout-dashboard" => Some(FullIconName::LayoutDashboard),
        _ => None,
    }
}
impl ConduitApp {
    pub(super) fn sync_plugin_context(&mut self) {
        use conduit::plugins::context::ScopeKey;
        let removed =
            self.plugins
                .right_key()
                .is_some_and(|key| match &key.scope {
                    ScopeKey::Global => false,
                    ScopeKey::Workspace(key) => {
                        !self.store.workspaces.iter().any(|w| w.path == key.path())
                    }
                    ScopeKey::Session { workspace, id } => !self.store.workspaces.iter().any(|w| {
                        w.path == workspace.path() && w.sessions.iter().any(|s| s.id == *id)
                    }),
                });
        let workspace = self
            .selected_workspace
            .and_then(|index| self.store.workspaces.get(index));
        let key = workspace.and_then(|w| WorkspaceKey::from_host_path(w.path.clone()).ok());
        let session = workspace
            .and_then(|w| {
                w.sessions
                    .iter()
                    .find(|s| Some(s.id) == self.selected_session)
            })
            .and_then(|s| {
                key.clone().map(|workspace| SessionKey {
                    workspace,
                    id: s.id,
                })
            });
        let session_running = session
            .as_ref()
            .is_some_and(|s| self.session_control.runs.contains_key(&s.id));
        let document = self
            .active_file
            .map(|id| format!("file:{id}"))
            .or_else(|| self.active_diff.map(|id| format!("diff:{id}")));
        self.plugins.update_context(ContextInput {
            workspace: key,
            session,
            document,
            session_running,
            writable: self.writable,
            closing: self.closing,
        });
        if removed {
            self.plugins
                .state
                .reset_right(&self.plugins.registry, &self.plugins.snapshot)
                .expect("core fallback remains registered");
            self.plugins.dock.drag = None;
        }
        let visible = if self.startup.is_some() {
            vec![]
        } else {
            self.plugins.visible_instance_keys()
        };
        self.plugins.instances.reconcile(
            &self.plugins.registry,
            &self.plugins.snapshot,
            &visible,
            |key| match &key.scope {
                ScopeKey::Global => true,
                ScopeKey::Workspace(key) => {
                    self.store.workspaces.iter().any(|w| w.path == key.path())
                }
                ScopeKey::Session { workspace, id } => {
                    self.store.workspaces.iter().any(|w| {
                        w.path == workspace.path() && w.sessions.iter().any(|s| s.id == *id)
                    })
                }
            },
        );
        #[cfg(conduit_shell_desktop)]
        if let Some(shell) = &mut self.plugins.shell {
            shell.host.set_visible(&visible);
        }
    }
    pub(super) fn select_navigation(&mut self, id: &ViewId) -> conduit::plugins::Result<()> {
        self.sync_plugin_context();
        self.plugins.select(id)?;
        Ok(())
    }
    pub(super) fn open_navigation(&mut self, id: &ViewId, cx: &mut Context<Self>) {
        self.sync_plugin_context();
        match self.plugins.select(id) {
            Ok(adapter) => (adapter.activate)(self, cx),
            Err(error) => {
                self.message = Some(conduit::i18n::format(
                    "plugin_ui.unable_to_open_navigation",
                    &[("error", format!("{}", error))],
                ))
            }
        }
        cx.notify();
    }
    fn dispatch_navigation(&mut self, request: &ActionRequest, cx: &mut Context<Self>) {
        self.sync_plugin_context();
        let ActionRequest::OpenView(view) = request else {
            return;
        };
        let validated = self
            .plugins
            .registry
            .validate_view(view, &self.plugins.snapshot);
        if validated.is_err() {
            return;
        }
        if let Some(adapter) = self.plugins.adapter(&view.key().view)
            && self
                .plugins
                .state
                .open(&self.plugins.registry, view, &self.plugins.snapshot)
                .is_ok()
        {
            self.plugins.reconcile_instance_visibility();
            (adapter.activate)(self, cx);
            cx.notify();
        }
    }
    pub(super) fn workspace_navigation_active(&self) -> bool {
        self.plugins.is_selected(builtins::WORKSPACES)
    }
    fn render_branch_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.branch_manager
            .as_ref()
            .map(|v| v.read(cx).tree.clone().into_any_element())
            .unwrap_or_else(|| div().into_any_element())
    }
    fn activity_selected(&self, item: &ActivityItem) -> bool {
        matches!(&item.action, ActivityAction::OpenView(id) if id == self.plugins.selected())
    }
    fn activity_button(&self, item: &ActivityItem, cx: &mut Context<Self>) -> AnyElement {
        let adapter = match &item.action {
            ActivityAction::OpenView(id) => self.plugins.adapter(id),
            _ => None,
        };
        let id = adapter
            .filter(|a| !a.element_id.is_empty())
            .map(|a| a.element_id.to_owned())
            .unwrap_or_else(|| item.id.to_string());
        let request = self.plugins.request(item);
        let pending = self.session_control.unread_completed.len() + self.session_control.runs.len();
        let badge = adapter.is_some_and(|a| a.task_badge);
        let tooltip = if badge {
            conduit::i18n::format(
                "plugin_ui.running_or_unread_tasks",
                &[
                    ("arg0", contribution_title(item.id.as_str(), &item.title)),
                    ("pending", format!("{}", pending)),
                ],
            )
        } else {
            contribution_title(item.id.as_str(), &item.title)
        };
        let button = Button::new(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .ghost()
            .focus_ring(true)
            .small()
            .compact()
            .w(px(36.))
            .h(px(36.))
            .flex_shrink_0()
            .icon(activity_icon(&item.icon).unwrap_or(FullIconName::CirclePile))
            .selected(self.activity_selected(item))
            .tooltip(tooltip)
            .accessibility_label(contribution_title(item.id.as_str(), &item.title))
            .disabled(request.is_none())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(request) = &request {
                    this.dispatch_activity(request, window, cx);
                }
            }));
        div()
            .relative()
            .w(px(36.))
            .h(px(36.))
            .flex_shrink_0()
            .child(button)
            .when(badge && pending > 0, |row| {
                row.child(
                    h_flex()
                        .debug_selector(|| "nav-workspaces-badge".into())
                        .absolute()
                        .top_0()
                        .right_0()
                        .min_w(px(14.))
                        .h(px(14.))
                        .px(px(3.))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(palette().text))
                        .text_color(rgb(palette().on_text))
                        .text_size(ui_text_size(10.))
                        .line_height(relative(1.))
                        .child(if pending > 99 {
                            "99+".to_owned()
                        } else {
                            pending.to_string()
                        }),
                )
            })
            .into_any_element()
    }
    pub(super) fn render_navigation(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_plugin_context();
        let items: Vec<_> = self
            .plugins
            .registry
            .activities(&self.plugins.snapshot)
            .into_iter()
            .cloned()
            .collect();
        let height =
            f32::from(window.viewport_size().height - gpui_kit::component::TITLE_BAR_HEIGHT);
        let visible = visible_activity_count(items.len(), height);
        let mut rail = v_flex()
            .w(px(38.))
            .h_full()
            .flex_shrink_0()
            .pt_1()
            .gap_1()
            .bg(rgb(palette().sidebar_bg))
            .border_r_1()
            .border_color(rgb(palette().border))
            .children(
                items[..visible]
                    .iter()
                    .map(|item| self.activity_button(item, cx)),
            );
        if visible < items.len() {
            let hidden: Vec<_> = items[visible..]
                .iter()
                .map(|item| {
                    (
                        item.clone(),
                        self.plugins.request(item),
                        self.activity_selected(item),
                    )
                })
                .collect();
            let selected = hidden.iter().any(|(_, _, selected)| *selected);
            let app = cx.entity();
            rail = rail.child(
                Button::new("nav-more")
                    .debug_selector(|| "nav-more".into())
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(36.))
                    .flex_shrink_0()
                    .icon(IconName::Ellipsis)
                    .selected(selected)
                    .tooltip(conduit::i18n::text_static("plugin_ui.more_features"))
                    .accessibility_label(conduit::i18n::text_static("plugin_ui.more_features"))
                    .dropdown_menu(move |mut menu, _, _| {
                        // Keep every contribution reachable even when the menu has
                        // more rows than the rail or the window can display. The
                        // menu recomputes its default height limit on each resize.
                        menu = menu.scrollable(true);
                        for (item, request, selected) in &hidden {
                            let app = app.clone();
                            let request = request.clone();
                            let label = contribution_title(item.id.as_str(), &item.title);
                            let debug = format!("nav-more-{}", item.id);
                            let entry = PopupMenuItem::element(move |_, _| {
                                div()
                                    .id(SharedString::from(debug.clone()))
                                    .test_support()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .debug_selector({
                                        let debug = debug.clone();
                                        move || debug.clone()
                                    })
                                    .child(label.clone())
                            })
                            .checked(*selected)
                            .disabled(request.is_none())
                            .icon(activity_icon(&item.icon).unwrap_or(FullIconName::CirclePile))
                            .on_click(move |_, window, cx| {
                                if let Some(request) = &request {
                                    app.update(cx, |app, cx| {
                                        app.dispatch_activity(request, window, cx)
                                    });
                                }
                            });
                            menu = menu.item(entry);
                        }
                        menu
                    }),
            );
        }
        let pending_push = self.pending_push_status();
        let branch_tooltip = pending_push.as_ref().map_or_else(
            || conduit::i18n::text_static("plugin_ui.git_branches_and_log").to_owned(),
            |(_, tooltip)| {
                conduit::i18n::format(
                    "plugin_ui.git_branches_and_log_27c010",
                    &[("tooltip", tooltip.to_string())],
                )
            },
        );
        let rail = rail.child(div().flex_1()).child(
            div()
                .relative()
                .w(px(36.))
                .h(px(34.))
                .flex_shrink_0()
                .child(
                    Button::new("nav-branches")
                        .debug_selector(|| "git-branch-trigger".into())
                        .ghost()
                        .small()
                        .compact()
                        .w(px(36.))
                        .h(px(34.))
                        .flex_shrink_0()
                        .icon(FullIconName::GitBranch)
                        .selected(self.plugins.is_selected(builtins::BRANCHES))
                        .tooltip(branch_tooltip.clone())
                        .accessibility_label(branch_tooltip)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_branch_navigation(window, cx)
                        })),
                )
                .when_some(pending_push, |row, (count, _)| {
                    row.child(
                        h_flex()
                            .id("nav-branches-badge")
                            .test_support()
                            .debug_selector(|| "nav-branches-badge".into())
                            .absolute()
                            .top_0()
                            .right_0()
                            .min_w(px(14.))
                            .h(px(14.))
                            .px(px(3.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(palette().text))
                            .text_color(rgb(palette().on_text))
                            .text_size(ui_text_size(10.))
                            .line_height(relative(1.))
                            .child(if count > 99 {
                                "99+".to_owned()
                            } else {
                                count.to_string()
                            }),
                    )
                }),
        );
        let rail = rail.child(
            Button::new("nav-collapse")
                .debug_selector(|| "nav-collapse".into())
                .ghost()
                .small()
                .compact()
                .w(px(36.))
                .h(px(32.))
                .flex_shrink_0()
                .icon(if self.plugins.collapsed() {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronLeft
                })
                .tooltip(conduit::i18n::text_static(
                    "plugin_ui.expand_collapse_sidebar",
                ))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.plugins.set_collapsed(!this.plugins.collapsed());
                    cx.notify();
                })),
        );

        if self.plugins.collapsed() {
            return rail.into_any_element();
        }
        let renderer = self
            .plugins
            .adapter(self.plugins.selected())
            .map(|adapter| adapter.render);
        let pane = renderer.map(|render| render(self, cx)).unwrap_or_else(|| {
            div()
                .p_3()
                .child(conduit::i18n::text_static(
                    "plugin_ui.navigation_view_unavailable",
                ))
                .into_any_element()
        });
        h_flex()
            .id("navigation-panel")
            .test_support()
            .debug_selector(|| "navigation-panel".into())
            .size_full()
            .child(rail)
            .child(div().flex_1().min_w_0().h_full().child(pane))
            .into_any_element()
    }
}

fn contribution_title(id: &str, title: &str) -> String {
    if id.starts_with("conduit.") {
        conduit::i18n::retained(title).to_string()
    } else {
        title.to_string()
    }
}
