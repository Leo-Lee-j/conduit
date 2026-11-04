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

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

#[derive(Clone, Copy)]
struct NavigationAdapter {
    render: fn(&mut ConduitApp, &mut Context<ConduitApp>) -> AnyElement,
    activate: fn(&mut ConduitApp, &mut Context<ConduitApp>),
    element_id: &'static str,
    task_badge: bool,
}
pub(super) struct PluginUi {
    registry: Registry,
    context: ContextTracker,
    snapshot: PluginContext,
    state: WorkbenchState,
    renderers: BTreeMap<String, NavigationAdapter>,
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
}
impl PluginUi {
    fn from_registry(registry: Registry) -> Self {
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
                },
            ),
            (
                builtins::FILES,
                NavigationAdapter {
                    render: ConduitApp::render_file_tree,
                    activate: ConduitApp::activate_file_navigation,
                    element_id: "nav-files",
                    task_badge: false,
                },
            ),
            (
                builtins::GIT,
                NavigationAdapter {
                    render: ConduitApp::render_git_navigation,
                    activate: ConduitApp::refresh_git,
                    element_id: "nav-git",
                    task_badge: false,
                },
            ),
            (
                builtins::BRANCHES,
                NavigationAdapter {
                    render: ConduitApp::render_branch_navigation,
                    activate: |app, _| app.refresh_branches(),
                    element_id: "nav-branches",
                    task_badge: false,
                },
            ),
        ]
        .into_iter()
        .map(|(key, adapter)| (key.into(), adapter))
        .collect();
        Self {
            registry,
            context,
            snapshot,
            state,
            renderers,
            reconciled_revision: None,
        }
    }
    pub(super) fn new() -> Self {
        Self::from_registry(
            builtins::register(false)
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
    }
    fn update_context(&mut self, input: ContextInput) {
        if self.snapshot.input() == &input && self.reconciled_revision == Some(self.registry.revision()) {
            return;
        }
        self.snapshot = self
            .context
            .update(input)
            .expect("context version available")
            .clone();
        self.state
            .reconcile(&self.registry, &self.snapshot)
            .expect("core fallbacks remain registered");
        self.reconciled_revision = Some(self.registry.revision());
    }
    pub(super) fn revision(&self) -> NavigationRevision {
        NavigationRevision {
            registry: self.registry.revision(),
            context: self.snapshot.version(),
            selected: self.selected().clone(),
            collapsed: self.collapsed(),
            running: self.snapshot.input().session_running,
        }
    }
    fn adapter(&self, view: &ViewId) -> Option<NavigationAdapter> {
        let descriptor = self.registry.view(view)?;
        self.renderers.get(&descriptor.renderer).copied()
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
            // Generic command handlers arrive with P1-4. Unbound actions are visibly disabled.
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
        Ok(adapter)
    }
}

// Rail rows are 36px + 4px gap. Reserve the four fixed bottom controls,
// their gaps, the spacer gap and 4px top padding. An overflow button needs one slot.
fn visible_activity_count(count: usize, height: f32) -> usize {
    let slots = ((height - 154.).max(0.) / 40.).floor() as usize;
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
            Err(error) => self.message = Some(format!("无法打开导航：{error}")),
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
            format!("{} · {pending} 个运行中或未读任务", item.title)
        } else {
            item.title.clone()
        };
        let button = Button::new(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .ghost()
            .small()
            .compact()
            .w(px(36.))
            .h(px(36.))
            .flex_shrink_0()
            .icon(activity_icon(&item.icon).unwrap_or(FullIconName::CirclePile))
            .selected(self.activity_selected(item))
            .tooltip(tooltip)
            .accessibility_label(item.title.clone())
            .disabled(request.is_none())
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(request) = &request {
                    this.dispatch_navigation(request, cx);
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
                        .text_size(px(10.))
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
        let height = f32::from(window.viewport_size().height - gpui_kit::component::TITLE_BAR_HEIGHT);
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
                    .tooltip("更多功能")
                    .accessibility_label("更多功能")
                    .dropdown_menu(move |mut menu, _, _| {
                        for (item, request, selected) in &hidden {
                            let app = app.clone();
                            let request = request.clone();
                            let label = item.title.clone();
                            let debug = format!("nav-more-{}", item.id);
                            let entry = PopupMenuItem::element(move |_, _| {
                                div()
                                    .debug_selector({
                                        let debug = debug.clone();
                                        move || debug.clone()
                                    })
                                    .child(label.clone())
                            })
                            .checked(*selected)
                            .disabled(request.is_none())
                            .icon(activity_icon(&item.icon).unwrap_or(FullIconName::CirclePile))
                            .on_click(move |_, _, cx| {
                                if let Some(request) = &request {
                                    app.update(cx, |app, cx| app.dispatch_navigation(request, cx));
                                }
                            });
                            menu = menu.item(entry);
                        }
                        menu
                    }),
            );
        }
        let rail = rail
            .child(div().flex_1())
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
                    .tooltip("Git 分支与日志")
                    .accessibility_label("Git 分支与日志")
                    .cursor_pointer()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.show_branch_navigation(window, cx)),
                    ),
            )
            .child(
                Button::new("nav-settings")
                    .debug_selector(|| "nav-settings".into())
                    .accessibility_label("设置")
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(34.))
                    .flex_shrink_0()
                    .icon(IconName::Settings)
                    .tooltip("设置")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            )
            .child(
                Button::new("nav-about")
                    .debug_selector(|| "nav-about".into())
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(34.))
                    .flex_shrink_0()
                    .icon(IconName::Info)
                    .tooltip("关于 Conduit")
                    .accessibility_label("关于")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_about_dialog(window, cx);
                    })),
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
                .tooltip("展开 / 收起侧栏")
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
        let pane = renderer
            .map(|render| render(self, cx))
            .unwrap_or_else(|| div().p_3().child("导航视图不可用").into_any_element());
        h_flex()
            .id("navigation-panel")
            .debug_selector(|| "navigation-panel".into())
            .size_full()
            .child(rail)
            .child(div().flex_1().min_w_0().h_full().child(pane))
            .into_any_element()
    }
}
