//! Workspace navigation and session list.

use super::*;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tag::Tag;

mod cache;
mod keyboard;
#[cfg(all(test, feature = "ui-tests"))]
mod pin_tests;
mod projects;
mod sticky;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;
pub(super) use cache::SidebarState;

// Keep the virtual-list estimates and rendered navigation rows in agreement.
const ROW_HEIGHT: f32 = 30.;
// Folder/CLI centers share an axis; the 16px marks leave an 8px title gap.
const TREE_AXIS: f32 = 14.;
const TREE_INDENT: f32 = 26.;
const TREE_MARK_SIZE: f32 = 16.;
const TREE_TOGGLE_SIZE: f32 = 24.;
const TREE_TITLE_LEFT: f32 = TREE_AXIS + TREE_MARK_SIZE / 2. + 8.;

fn tree_guide(id: String, axis: f32, height: f32) -> Div {
    div()
        .debug_selector(move || id.clone())
        .absolute()
        .left(px(axis))
        .top_0()
        .h(px(height))
        .w(px(1.))
        .bg(rgb(palette().border))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskState {
    Running,
    Approval,
    Answer,
    Stopping,
    Cleanup,
}

impl TaskState {
    fn new(run: &ActiveRun) -> Self {
        if run.cleanup_error.is_some() {
            Self::Cleanup
        } else if run.runtime.is_cancelled() {
            Self::Stopping
        } else if let Some(approval) = run.approvals.front() {
            if approval.is_user_input() {
                Self::Answer
            } else {
                Self::Approval
            }
        } else {
            Self::Running
        }
    }

    fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Running => "session_data.running",
            Self::Approval => "sidebar.awaiting_confirmation",
            Self::Answer => "sidebar.awaiting_answer",
            Self::Stopping => "session_data.stopping",
            Self::Cleanup => "sidebar.cleanup_pending",
        })
    }
}

/// Shared frame, toolbar and horizontal gutter for the left navigation pages.
/// Scrolling and bottom borders remain page-specific rather than being nested
/// inside a padded frame, so they can extend to the pane edges.
pub(super) fn navigation_pane() -> Div {
    v_flex()
        .size_full()
        .min_w_0()
        .bg(chrome_background(palette().sidebar_bg))
}

pub(super) fn navigation_header() -> Div {
    h_flex()
        .h(px(36.))
        .flex_shrink_0()
        .min_w_0()
        .px_2()
        .items_center()
}

pub(super) fn navigation_content() -> Div {
    v_flex().min_w_0().px_2()
}

#[derive(Clone)]
struct WorkspaceDrag {
    path: PathBuf,
    index: usize,
    pinned: bool,
}
struct WorkspaceDragPreview {
    name: String,
}
impl Render for WorkspaceDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(px(248.))
            .h(px(42.))
            .gap_2()
            .items_center()
            .px_3()
            .rounded(px(10.))
            .bg(rgb(palette().surface_strong))
            .border_1()
            .border_color(rgb(palette().border))
            .shadow_sm()
            .text_size(ui_text_size(13.))
            .text_color(rgb(palette().text))
            .child(
                Icon::new(IconName::Folder)
                    .small()
                    .text_color(rgb(palette().highlight_text)),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.name.clone()))
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().faint))
                    .child(conduit::i18n::text_static("sidebar.move")),
            )
    }
}

/// Cache session navigation independently of wheel-driven transcript frames.
pub(super) struct SidebarView {
    app: WeakEntity<ConduitApp>,
    _subscription: Subscription,
    _color_subscription: Subscription,
    _bounds_subscription: Subscription,
    was_workspace_navigation: bool,
    navigation_revision: Option<super::plugin_ui::NavigationRevision>,
    pending_push: Option<(usize, String)>,
    pending_tasks: usize,
    #[cfg(test)]
    pub(super) render_count: usize,
}

impl SidebarView {
    pub(super) fn new(
        app: WeakEntity<ConduitApp>,
        color_picker: &Entity<ColorPickerState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&app.upgrade().expect("app is alive"), |view, app, cx| {
            #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
            let _span = chat::profiling::host_span("sidebar_observe");
            let (workspaces, changed, revision, pending_push, pending_tasks, refresh) =
                app.update(cx, |app, cx| {
                    app.sync_plugin_context();
                    app.refresh_plugin_views(cx);
                    let workspaces = app.workspace_navigation_active();
                    (
                        workspaces,
                        workspaces && app.refresh_sidebar(),
                        app.plugins.revision(),
                        app.pending_push_status(),
                        app.session_control.runs.len() + app.session_control.unread_completed.len(),
                        app.plugins.navigation_refresh_on_notify(),
                    )
                });
            if refresh
                || workspaces != view.was_workspace_navigation
                || changed
                || view.navigation_revision.as_ref() != Some(&revision)
                || view.pending_push != pending_push
                || view.pending_tasks != pending_tasks
            {
                cx.notify();
            }
            view.was_workspace_navigation = workspaces;
            view.navigation_revision = Some(revision);
            view.pending_push = pending_push;
            view.pending_tasks = pending_tasks;
        });
        Self {
            _color_subscription: cx.observe(color_picker, |_, _, cx| cx.notify()),
            app,
            _subscription: subscription,
            _bounds_subscription: cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
            was_workspace_navigation: false,
            navigation_revision: None,
            pending_push: None,
            pending_tasks: 0,
            #[cfg(test)]
            render_count: 0,
        }
    }
}

impl Render for SidebarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        if let Some(app) = self.app.upgrade() {
            self.was_workspace_navigation = app.read(cx).workspace_navigation_active();
        }
        let rendered = self
            .app
            .update(cx, |app, cx| app.render_navigation(window, cx));
        if let Some(app) = self.app.upgrade() {
            let app = app.read(cx);
            self.navigation_revision = Some(app.plugins.revision());
            self.pending_push = app.pending_push_status();
            self.pending_tasks =
                app.session_control.runs.len() + app.session_control.unread_completed.len();
        }
        rendered.unwrap_or_else(|_| div().into_any_element())
    }
}

impl ConduitApp {
    fn workspace_collapsed(&self, path: &Path) -> bool {
        self.collapsed_workspaces.contains(path) || self.sidebar.auto_collapsed.contains(path)
    }

    fn set_workspace_collapsed(&mut self, path: &Path, collapsed: bool) {
        if collapsed {
            self.collapsed_workspaces.insert(path.to_owned());
            self.sidebar.auto_expanded.remove(path);
        } else {
            self.collapsed_workspaces.remove(path);
            self.sidebar.auto_expanded.insert(path.to_owned());
        }
    }

    pub(super) fn refresh_sidebar(&mut self) -> bool {
        let inputs = cache::Inputs::new(self);
        self.sidebar
            .refresh(&self.store, &self.session_control, &self.search_ui, inputs)
    }

    fn set_sidebar_sessions_expanded(
        &mut self,
        path: &Path,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.sidebar.workspace_index(path) else {
            return;
        };
        if expanded {
            self.expanded_workspaces.insert(index);
        } else {
            self.expanded_workspaces.remove(&index);
        }
        self.refresh_sidebar();
        if !expanded {
            self.sidebar.reveal_workspace(path);
        }
        self.focus_sidebar(window, cx);
    }

    fn toggle_sidebar_workspace(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let collapsed = !self.workspace_collapsed(path);
        self.set_workspace_collapsed(path, collapsed);
        self.refresh_sidebar();
        if collapsed {
            self.sidebar.reveal_workspace(path);
        }
        self.focus_sidebar(window, cx);
    }

    fn activate_sidebar_workspace(
        &mut self,
        index: usize,
        session: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.store.workspaces.get(index) else {
            return;
        };
        let path = workspace.path.clone();
        if !self.workspace_collapsed(&path) {
            self.toggle_sidebar_workspace(&path, window, cx);
            return;
        }
        self.switch(index, session, window, cx);
        if self.selected_workspace == Some(index) && self.selected_session == session {
            self.set_workspace_collapsed(&path, false);
            self.refresh_sidebar();
            self.sidebar_view.update(cx, |_, cx| cx.notify());
            cx.notify();
        }
    }

    fn focus_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = &self.sidebar.focus {
            focus.focus(window, cx);
        }
        self.sidebar_view.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn sidebar_compact_list_tooltip(&self) -> String {
        conduit::i18n::format(
            "sidebar.collapse_session_list_hint",
            &[(
                "count",
                self.store
                    .ui
                    .sessions_per_workspace
                    .clamp(1, 30)
                    .to_string(),
            )],
        )
    }

    fn sidebar_focus_indicator(&self) -> AnyElement {
        let focus = self.sidebar.focus.clone();
        // Paint independently of layout and use the live input modality, including
        // when the virtual list reuses a cached row after a mouse/keyboard change.
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                if window.last_input_was_keyboard()
                    && focus.as_ref().is_some_and(|focus| focus.is_focused(window))
                {
                    let mut ring =
                        gpui_kit::outline(bounds, rgb(palette().focus), Default::default());
                    ring.corner_radii = px(8.).into();
                    window.paint_quad(ring);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    fn workspace_reorderable(&self, path: &Path) -> bool {
        self.writable
            && !self.closing
            && (!self.store.ui.auto_sort_workspaces
                || self.store.ui.pinned_workspaces.contains(path))
    }

    fn can_reorder_workspace(&self, source: &Path, target: &Path) -> bool {
        self.workspace_reorderable(source)
            && self.store.ui.pinned_workspaces.contains(source)
                == self.store.ui.pinned_workspaces.contains(target)
    }

    /// Reorder by stable paths and remap all index-based navigation state.
    pub(super) fn reorder_workspace(
        &mut self,
        source: &Path,
        target: &Path,
        cx: &mut Context<Self>,
    ) {
        if !self.can_reorder_workspace(source, target) {
            return;
        }
        let Some(from) = self.store.workspaces.iter().position(|w| w.path == source) else {
            return;
        };
        let Some(to) = self.store.workspaces.iter().position(|w| w.path == target) else {
            return;
        };
        if from == to {
            return;
        }
        let remap = |index: usize| {
            if index == from {
                to
            } else if from < to && index > from && index <= to {
                index - 1
            } else if to < from && index >= to && index < from {
                index + 1
            } else {
                index
            }
        };
        let moved = self.store.workspaces.remove(from);
        self.store.workspaces.insert(to, moved);
        self.selected_workspace = self.selected_workspace.map(remap);
        self.remove_workspace_target = self.remove_workspace_target.map(remap);
        self.expanded_workspaces = self
            .expanded_workspaces
            .iter()
            .copied()
            .map(remap)
            .collect();
        // Sessions, tabs, running tasks and Git caches retain their path/id identity.
        self.persist();
        cx.notify();
    }

    pub(super) fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.refresh_sidebar();
        let focus = self
            .sidebar
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let mut sidebar = navigation_pane()
            .id("sidebar")
            .debug_selector(|| "sidebar".into())
            .relative()
            .track_focus(&focus)
            .role(Role::Tree)
            .aria_label(conduit::i18n::text_static(
                "sidebar.workspaces_and_sessions",
            ))
            .aria_description(conduit::i18n::text_static(
                "sidebar.arrow_keys_to_move_left_right_to",
            ))
            .tab_index(0)
            .focus_visible(|style| {
                style.shadow(vec![BoxShadow {
                    color: rgb(palette().focus).into(),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(1.),
                    inset: true,
                }])
            })
            .on_key_down(cx.listener(Self::sidebar_key))
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                // A row-owned confirmation should not reopen when its row is recycled.
                if this.delete_target.take().is_some() {
                    cx.notify();
                }
            }))
            // No bottom padding: the bottom bar aligns with the conversation toolbar.
            .gap_0()
            .child(self.sidebar_controls(cx))
            .child(self.sidebar_active_tasks(cx));
        let rows = self.sidebar.rows.clone();
        let list = gpui_kit::component::v_virtual_list(
            cx.entity(),
            "workspace-list",
            self.sidebar.sizes.clone(),
            move |app, range, _, cx| {
                range
                    .map(|ix| {
                        let parent = rows[ix]
                            .path()
                            .and_then(|path| app.sidebar.projects.parent(path));
                        let last_project_child = rows.get(ix + 1).is_none_or(|next| {
                            matches!(next, cache::Row::Project { .. })
                                || next
                                    .path()
                                    .and_then(|path| app.sidebar.projects.parent(path))
                                    != parent
                        });
                        app.render_sidebar_row(&rows[ix], rows.get(ix + 1), last_project_child, cx)
                    })
                    .collect()
            },
        )
        .track_scroll(&self.sidebar.scroll)
        .flex_1()
        .min_h_0()
        .px_2()
        .gap_0();
        // Mirrors the conversation toolbar height so the two bottom bars read
        // as one strip.
        sidebar = sidebar
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(list)
                    .child(self.sticky_workspace_header(cx)),
            )
            .child(
                StatusBar::new()
                    .h(px(STATUS_BAR_HEIGHT))
                    .min_w_0()
                    .flex_shrink_0()
                    .py_0()
                    .bg(chrome_background(palette().sidebar_bg))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border)),
            );
        sidebar
            .when_some(self.sidebar.menu.clone(), |sidebar, menu| {
                sidebar.child(deferred(div().absolute().top(px(36.)).left_2().child(menu)))
            })
            .into_any_element()
    }

    fn render_sidebar_row(
        &self,
        row: &cache::Row,
        next: Option<&cache::Row>,
        last_project_child: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        self.sidebar
            .rendered_rows
            .set(self.sidebar.rendered_rows.get() + 1);
        let last_child = row.is_last_child(next);
        let list_height = row.list_height(next);
        let app_entity = cx.entity();
        let workspace_index = row
            .path()
            .and_then(|path| self.sidebar.workspace_index(path));
        let nested = !matches!(row, cache::Row::Project { .. })
            && row
                .path()
                .and_then(|path| self.sidebar.projects.parent(path))
                .is_some();
        let indent = if nested { TREE_INDENT } else { 0. };
        let element = match row {
            cache::Row::Project { .. } => self.sidebar_project(row, cx),
            cache::Row::Empty => v_flex()
                .flex_shrink_0()
                .px_2()
                .py_3()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(gpui_kit::assets::IconName::FolderPlus)
                                .small()
                                .text_color(rgb(palette().faint)),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::text_static("sidebar.no_workspaces_yet")),
                        ),
                )
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::text_static(
                            "sidebar.click_above_to_add_a_local_directory",
                        )),
                )
                .into_any_element(),
            _ => {
                let Some(index) = workspace_index else {
                    return div().h(px(row.height())).into_any_element();
                };
                match row {
                    cache::Row::Workspace { count, last, .. } => {
                        self.sidebar_workspace(index, *count, *last, cx)
                    }
                    cache::Row::Group { group, .. } => h_flex()
                        .h(px(row.height()))
                        .pl(px(TREE_TITLE_LEFT + indent))
                        .items_center()
                        .child(
                            Tag::secondary()
                                .small()
                                .text_size(ui_text_size(11.))
                                .child(group.label()),
                        )
                        .into_any_element(),
                    cache::Row::Loading(_) => {
                        let workspace = &self.store.workspaces[index];
                        let loaded = workspace.visible_sessions().count();
                        h_flex()
                            .h(px(ROW_HEIGHT))
                            .flex_shrink_0()
                            .pl(px(TREE_TITLE_LEFT + indent))
                            .pr(px(8.))
                            .gap_2()
                            .items_center()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(Spinner::new().small().color(rgb(palette().faint).into()))
                            .child(if loaded > 0 {
                                conduit::i18n::format(
                                    "sidebar.loading_session_history",
                                    &[("completed", loaded.to_string())],
                                )
                            } else {
                                conduit::i18n::text_static("sidebar.discovering_session_history")
                                    .into()
                            })
                            .into_any_element()
                    }
                    cache::Row::Session { id, .. } => {
                        let Some(session) = self.session(*id) else {
                            return div().h(px(row.height())).into_any_element();
                        };
                        let id = session.id;
                        let active = self.selected_session == Some(id);
                        let running = self.session_control.runs.contains_key(&id);
                        let unread = self.session_control.unread_completed.contains(&id);
                        let archived = session.archived;
                        let pinned = session.pinned;

                        h_flex()
                            .id(("session", id))
                            .role(Role::TreeItem)
                            .aria_selected(active)
                            .aria_label(format!("{} · {}{}{}", single_line_title(&session.title), session.provider.label(), if running { conduit::i18n::text_static("sidebar.running") } else { "" }, if archived { conduit::i18n::text_static("plugin_ui.example.archived") } else { "" }))
                            .when(self.sidebar.keyboard_row.as_ref().is_some_and(|row| matches!(row, cache::Row::Session { id: focused, .. } if *focused == id)), |row| row.aria_active_descendant())
                            .debug_selector(move || format!("session-row-{id}"))
                            .group("session-row")
                            .relative()
                            .h(px(row.height()))
                            .flex_shrink_0()
                            .pl(px(TREE_AXIS + indent - TREE_MARK_SIZE / 2.))
                            .pr(px(8.))
                            .gap_2()
                            .items_center()
                            .rounded(px(8.))
                            .text_size(ui_text_size(13.))
                            .text_color(rgb(if archived {
                                palette().faint
                            } else if active {
                                palette().text
                            } else {
                                theme::sidebar_session_text_color()
                            }))
                            .when(active, |row| row.bg(rgb(palette().surface_hover)))
                            .cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(rgb(palette().surface_hover))
                                    .text_color(rgb(palette().text))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_sidebar_session(id, window, cx)
                            }))
                            .context_menu({
                                let menu_app = app_entity.clone();
                                move |menu, _, cx| {
                                    session_menu(menu, menu_app.clone(), id, menu_app.read(cx))
                                }
                            })
                            .child(tree_guide(
                                format!("session-guide-{id}"),
                                TREE_AXIS + indent,
                                if last_child { row.height() / 2. } else { list_height },
                            ))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .debug_selector(move || format!("session-cli-{id}"))
                                            .size(px(TREE_MARK_SIZE))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .bg(rgb(if active { palette().surface_hover } else { palette().sidebar_bg }))
                                            .group_hover("session-row", |style| style.bg(rgb(palette().surface_hover)))
                                            .child(provider_icon(session.provider).size_full()),
                                    )
                                    .child(
                                        div()
                                            .debug_selector(move || format!("session-title-{id}"))
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .line_height(px(16.))
                                            .child(if session.title.is_empty() {
                                                conduit::i18n::text_static("files.new_session").to_string()
                                            } else {
                                                single_line_title(&session.title)
                                            }),
                                    )
                                    .when(self.session_control.namings.contains_key(&id), |row| {
                                        row.child(Spinner::new().small().color(rgb(palette().muted).into()))
                                    })
                                    .when(pinned, |row| {
                                        row.child(
                                            Icon::new(FullIconName::Pin)
                                                .size(px(12.))
                                                .text_color(rgb(palette().faint)),
                                        )
                                    })
                                    .when(archived, |row| {
                                        row.child(
                                            div()
                                                .text_size(ui_text_size(12.))
                                                .text_color(rgb(palette().faint))
                                                .child(conduit::i18n::text_static("search.archived")),
                                        )
                                    })
                                    .when(running || unread, |row| {
                                        row.child(session_activity_dot("session-activity", id, running))
                                    })
                                    .when_some(session_age_label(session.last_activity_at()), |row, age| {
                                        row.child(
                                            div()
                                                .debug_selector(move || format!("session-age-{id}"))
                                                .flex_shrink_0()
                                                .whitespace_nowrap()
                                                .line_height(px(16.))
                                                .text_size(ui_text_size(12.))
                                                .text_color(rgb(palette().faint))
                                                .child(age),
                                        )
                                    }),
                            )
                            .when(self.delete_target == Some(id), |row| {
                                row.child(
                                    div()
                                        .absolute()
                                        .right_0()
                                        .top_0()
                                        .child(self.delete_session_popover(id, cx)),
                                )
                            })
                            .into_any_element()
                    }
                    cache::Row::More {
                        path,
                        total,
                        expanded,
                    } => {
                        let total = *total;
                        let expanded = *expanded;
                        let limit = self.store.ui.sessions_per_workspace.clamp(1, 30);
                        let toggle_app = app_entity.clone();
                        let toggle_path = path.clone();
                        let more_label = if expanded {
                            conduit::i18n::text_static("sidebar.collapse_session_list").to_string()
                        } else {
                            conduit::i18n::format(
                                "sidebar.show_more_remaining",
                                &[("arg0", format!("{}", total - limit))],
                            )
                        };
                        let tooltip = if expanded {
                            self.sidebar_compact_list_tooltip()
                        } else {
                            more_label.clone()
                        };

                        h_flex()
                            .h(px(row.height()))
                            .w_full()
                            .items_center()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "workspace-more-{}",
                                    self.store.workspaces[index].path.display()
                                )))
                                .debug_selector(move || format!("workspace-more-{index}"))
                                .tooltip(tooltip.clone())
                                .accessibility_label(tooltip)
                                .small()
                                .compact()
                                .ghost()
                                .h(px(row.height()))
                                .w_full()
                                .pl(px(TREE_AXIS + indent - TREE_MARK_SIZE / 2.))
                                .pr_2()
                                .text_color(rgb(palette().muted))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            h_flex()
                                                .debug_selector(move || {
                                                    format!("workspace-more-node-{index}")
                                                })
                                                .size(px(TREE_MARK_SIZE))
                                                .flex_shrink_0()
                                                .items_center()
                                                .justify_center()
                                                .rounded_full()
                                                .border_1()
                                                .border_color(rgb(palette().faint))
                                                .child(
                                                    Icon::new(if expanded {
                                                        FullIconName::ChevronUp
                                                    } else {
                                                        FullIconName::ChevronDown
                                                    })
                                                    .size(px(12.)),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .debug_selector(move || {
                                                    format!("workspace-more-label-{index}")
                                                })
                                                .min_w_0()
                                                .truncate()
                                                .child(more_label),
                                        ),
                                )
                                .on_click(
                                    move |_, window, cx| {
                                        cx.stop_propagation();
                                        toggle_app.update(cx, |this, cx| {
                                            this.set_sidebar_sessions_expanded(
                                                &toggle_path,
                                                !expanded,
                                                window,
                                                cx,
                                            );
                                        });
                                    },
                                ),
                            )
                            .into_any_element()
                    }
                    _ => unreachable!(),
                }
            }
        };
        let wrapper = div()
            .relative()
            .w_full()
            .h(px(list_height))
            .child(element)
            .when(
                matches!(row, cache::Row::Workspace { .. }) && !last_child
                    || matches!(row, cache::Row::Project { key, .. }
                        if self.sidebar.expanded_projects.contains(key) || !self.search_query.trim().is_empty()),
                |wrapper| {
                    let index = workspace_index.expect("tree header workspace");
                    let prefix = if matches!(row, cache::Row::Project { .. }) { "project" } else { "workspace" };
                    wrapper.child(
                        tree_guide(
                            format!("{prefix}-guide-{index}-header"),
                            TREE_AXIS + indent,
                            list_height - (ROW_HEIGHT + TREE_MARK_SIZE) / 2.,
                        )
                        .top(px((ROW_HEIGHT + TREE_MARK_SIZE) / 2.)),
                    )
                },
            )
            .when(
                row.path().is_some()
                    && !row.is_header()
                    && !matches!(row, cache::Row::Session { .. }),
                |wrapper| {
                    let index = workspace_index.expect("child workspace rendered");
                    let guide_id = match row {
                        cache::Row::Group { group, .. } => {
                            format!("workspace-guide-{index}-{group:?}")
                        }
                        cache::Row::More { .. } => format!("workspace-guide-{index}-more"),
                        cache::Row::Loading(_) => format!("workspace-guide-{index}-loading"),
                        _ => unreachable!(),
                    };
                    wrapper.child(tree_guide(
                        guide_id,
                        TREE_AXIS + indent,
                        if matches!(row, cache::Row::More { .. }) {
                            // Stop at the hollow node's rim, leaving its center empty.
                            (row.height() - TREE_MARK_SIZE) / 2.
                        } else if last_child {
                            row.height()
                        } else {
                            list_height
                        },
                    ))
                },
            )
            .when(nested, |wrapper| {
                let index = workspace_index.expect("nested workspace");
                let guide_id = match row {
                    cache::Row::Session { id, .. } => format!("project-guide-session-{id}"),
                    cache::Row::Workspace { .. } => format!("project-guide-workspace-{index}"),
                    cache::Row::Group { group, .. } => format!("project-guide-{index}-{group:?}"),
                    cache::Row::More { .. } => format!("project-guide-{index}-more"),
                    _ => format!("project-guide-{index}-loading"),
                };
                wrapper.child(
                    div()
                        .debug_selector(move || guide_id.clone())
                        .absolute()
                        .left(px(TREE_AXIS))
                        .top_0()
                        .bottom(px(if last_project_child { list_height - row.height() } else { 0. }))
                        .w(px(1.))
                        .bg(rgb(palette().border)),
                )
            })
            .when(
                !row.is_header()
                    && self
                        .sidebar
                        .keyboard_row
                        .as_ref()
                        .is_some_and(|focused| focused.same_identity(row)),
                |row| row.child(self.sidebar_focus_indicator()),
            );
        if let Some(offset) = row
            .path()
            .and_then(|path| self.sidebar.motion_start_offset(path))
            && let Some(started) = self.sidebar.motion_started
            && started.elapsed() < cache::WORKSPACE_MOVE_DURATION
        {
            return wrapper
                .relative()
                .with_animation(
                    SharedString::from(format!(
                        "workspace-move-{}-{row:?}",
                        self.sidebar.motion_generation
                    )),
                    Animation::new(cache::WORKSPACE_MOVE_DURATION),
                    move |row, progress| row.top(px(offset * (1. - progress).powi(3))),
                )
                .into_any_element();
        }
        wrapper.into_any_element()
    }

    fn switch_sidebar_session(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((workspace, _)) = self.session_control.locate(&self.store.workspaces, id) {
            self.switch(workspace, Some(id), window, cx);
            if self.selected_session == Some(id) {
                let path = self.store.workspaces[workspace].path.clone();
                self.set_workspace_collapsed(&path, false);
                self.refresh_sidebar();
                if let Some(key) = self.sidebar.projects.parent(&path).cloned() {
                    self.sidebar.expanded_projects.insert(key);
                    self.refresh_sidebar();
                }
            }
        }
    }

    fn sidebar_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_edit = self.writable && !self.closing;
        h_flex()
            .id("workspace-controls")
            .debug_selector(|| "workspace-controls".into())
            .w_full()
            .h(px(44.))
            .flex_shrink_0()
            .px_2()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(ui_text_size(16.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child(conduit::i18n::text_static("git_compare.workspace")),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("sort-projects-by-activity")
                            .debug_selector(|| "sort-projects-by-activity".into())
                            .icon(FullIconName::ListFilter)
                            .small()
                            .compact()
                            .ghost()
                            .w(px(32.))
                            .h(px(32.))
                            .cursor_pointer()
                            .selected(self.store.ui.auto_sort_workspaces)
                            .tooltip(conduit::i18n::text_static(
                                "sidebar.sort_projects_by_activity",
                            ))
                            .accessibility_label(conduit::i18n::text_static(
                                "sidebar.sort_projects_by_activity",
                            ))
                            .disabled(!self.writable || self.closing)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.writable || this.closing {
                                    return;
                                }
                                this.store.ui.auto_sort_workspaces =
                                    !this.store.ui.auto_sort_workspaces;
                                this.persist();
                                this.refresh_sidebar();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("toggle-archived")
                            .cursor_pointer()
                            .debug_selector(|| "toggle-archived".into())
                            .icon(IconName::Inbox)
                            .small()
                            .ghost()
                            .compact()
                            .w(px(32.))
                            .h(px(32.))
                            .tooltip(conduit::i18n::text_static("search.archived"))
                            .accessibility_label(conduit::i18n::text_static("search.archived"))
                            .selected(self.show_archived)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_archived = !this.show_archived;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("sidebar-search")
                            .cursor_pointer()
                            .debug_selector(|| "sidebar-search".into())
                            .icon(IconName::Search)
                            .small()
                            .compact()
                            .ghost()
                            .w(px(32.))
                            .h(px(32.))
                            .tooltip(conduit::i18n::text_static("search.everything"))
                            .accessibility_label(conduit::i18n::text_static("search.everything"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_search_dialog(window, cx)
                            })),
                    )
                    .child(
                        Button::new("add-workspace")
                            .cursor_pointer()
                            .debug_selector(|| "add-workspace".into())
                            .icon(IconName::Plus)
                            .small()
                            .primary()
                            .w(px(32.))
                            .h(px(32.))
                            .p_0()
                            .rounded(px(8.))
                            .tooltip(conduit::i18n::text_static("sessions.add_workspace"))
                            .accessibility_label(conduit::i18n::text_static(
                                "sessions.add_workspace",
                            ))
                            .disabled(!can_edit)
                            .on_click(cx.listener(Self::add_workspace)),
                    ),
            )
            .into_any_element()
    }

    fn sidebar_active_tasks(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut tasks: Vec<_> = self
            .session_control
            .runs
            .iter()
            .filter_map(|(id, run)| {
                self.session(*id)?;
                Some((*id, run.turn_started, TaskState::new(run)))
            })
            .collect();
        tasks.sort_by(|(a_id, a_started, _), (b_id, b_started, _)| {
            b_started.cmp(a_started).then_with(|| a_id.cmp(b_id))
        });
        if tasks.is_empty() {
            return div().into_any_element();
        }

        let count = tasks.len();
        let panel = v_flex()
            .id("sidebar-active-tasks")
            .debug_selector(|| "sidebar-active-tasks".into())
            .mx_2()
            .mb_2()
            .flex_shrink_0()
            .p_1()
            .rounded(px(8.))
            .bg(rgb(palette().canvas))
            .border_1()
            .border_color(rgb(palette().border))
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .font_semibold()
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static("sidebar.active_tasks")),
                    )
                    .child(
                        h_flex()
                            .id("sidebar-active-task-count")
                            .debug_selector(|| "sidebar-active-task-count".into())
                            .min_w(px(18.))
                            .h(px(18.))
                            .px(px(5.))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(palette().activity_running))
                            .text_color(rgb(0xffffff))
                            .text_size(ui_text_size(10.))
                            .line_height(relative(1.))
                            .child(if count > 99 {
                                "99+".into()
                            } else {
                                count.to_string()
                            }),
                    ),
            );

        let mut list = v_flex()
            .id("sidebar-active-task-list")
            .max_h(px(208.))
            .overflow_y_scroll();
        let row_radius = px(6.);
        let outline_color = rgb(palette().focus);
        for (id, _, state) in tasks {
            let Some(session) = self.session(id) else {
                continue;
            };
            let Some((workspace_index, _)) =
                self.session_control.locate(&self.store.workspaces, id)
            else {
                continue;
            };
            let workspace_name = self.store.workspaces[workspace_index].name();
            let status = state.label();
            let status_color = if matches!(
                state,
                TaskState::Approval | TaskState::Answer | TaskState::Cleanup
            ) {
                palette().highlight_text
            } else {
                palette().activity_running
            };
            let title = if session.title.is_empty() {
                conduit::i18n::text_static("files.new_session").to_string()
            } else {
                single_line_title(&session.title)
            };
            let age = session_age_label(session.last_activity_at());
            let active = self.selected_session == Some(id);
            list = list.child(
                Button::new(("workspace-active-task", id))
                    .debug_selector(move || format!("workspace-active-task-{id}"))
                    .w_full()
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .ghost()
                    .selected(active)
                    .cursor_pointer()
                    .rounded(row_radius)
                    .accessibility_label(format!("{title} · {workspace_name} · {status}"))
                    .when(active, |row| row.bg(rgb(palette().surface)))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .text_left()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .size(px(10.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(status_color)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .min_w_0()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(ui_text_size(13.))
                                                    .text_color(rgb(palette().text))
                                                    .child(title),
                                            )
                                            .when_some(age, |row, age| {
                                                row.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .whitespace_nowrap()
                                                        .text_size(ui_text_size(11.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(age),
                                                )
                                            }),
                                    )
                                    .child(
                                        h_flex()
                                            .min_w_0()
                                            .gap_1()
                                            .text_size(ui_text_size(11.))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_color(rgb(palette().muted))
                                                    .child(workspace_name),
                                            )
                                            .child(
                                                div().text_color(rgb(palette().muted)).child("·"),
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_color(rgb(status_color))
                                                    .child(status),
                                            ),
                                    ),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_sidebar_session(id, window, cx)
                    }))
                    .map(|button| {
                        div()
                            .relative()
                            .flex_shrink_0()
                            .group("active-task-row")
                            .child(button)
                            // Paint the outline separately so it never affects the row layout.
                            .child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .rounded(row_radius)
                                    .border_1()
                                    .border_color(outline_color.opacity(if active {
                                        1.
                                    } else {
                                        0.
                                    }))
                                    .when(!active, |outline| {
                                        outline.group_hover("active-task-row", |style| {
                                            style.border_color(outline_color.opacity(0.4))
                                        })
                                    }),
                            )
                    }),
            );
        }
        panel.child(list).into_any_element()
    }

    fn sidebar_workspace(
        &self,
        index: usize,
        count: usize,
        last: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let workspace = &self.store.workspaces[index];
        let app_entity = cx.entity();
        let can_edit = self.writable && !self.closing;
        let workspace_selected = self.selected_workspace == Some(index);
        let nested = self.sidebar.projects.parent(&workspace.path).is_some();
        let collapsed = self.workspace_collapsed(&workspace.path);
        let importing = self.history_loading.contains(&workspace.path);
        let workspace_color = self.store.ui.workspace_colors.get(&workspace.path).copied();
        let pinned = self.store.ui.pinned_workspaces.contains(&workspace.path);
        let can_reorder = self.workspace_reorderable(&workspace.path);
        let workspace_hover_color = workspace_color.map(|color| {
            let mut tint = gpui_kit::rgba(color);
            tint.a *= 0.24;
            tint
        });
        let compact_path = workspace.path.clone();
        let drag_path = workspace.path.clone();
        let drop_path = workspace.path.clone();
        let hover_path = workspace.path.clone();
        let drag_name = workspace.name();
        // Attention sets are small; do not scan every session in a virtualized header.
        let activity = self
            .session_control
            .runs
            .keys()
            .copied()
            .filter(|id| {
                self.session_control
                    .locate(&self.store.workspaces, *id)
                    .is_some_and(|(workspace, _)| workspace == index)
            })
            .min()
            .map(|id| (id, true))
            .or_else(|| {
                self.session_control
                    .unread_completed
                    .iter()
                    .copied()
                    .filter(|id| {
                        self.session_control
                            .locate(&self.store.workspaces, *id)
                            .is_some_and(|(workspace, _)| workspace == index)
                    })
                    .min()
                    .map(|id| (id, false))
            });
        h_flex()
            .id(SharedString::from(format!(
                "workspace-{}",
                workspace.path.display()
            )))
            .debug_selector(move || format!("workspace-item-{index}"))
            .role(Role::TreeItem)
            .aria_label(if pinned {
                conduit::i18n::format(
                    "sidebar.pinned_workspace_label",
                    &[("name", workspace.name())],
                )
            } else {
                workspace.name()
            })
            .aria_expanded(!collapsed)
            .aria_selected(workspace_selected)
            .when(self.sidebar.keyboard_row.as_ref().is_some_and(|row| matches!(row, cache::Row::Workspace { path, .. } if *path == workspace.path)), |row| row.aria_active_descendant())
            .group("workspace-row")
            .relative()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .pl(px(TREE_AXIS + if nested { TREE_INDENT } else { 0. } - TREE_TOGGLE_SIZE / 2.))
            .pr_2()
            .gap(px(4.))
            .items_center()
            .rounded(px(8.))
            .text_size(ui_text_size(13.))
            .text_color(rgb(palette().text))
            .cursor_pointer()
            .hover(move |style| match workspace_hover_color {
                Some(color) => style.bg(color),
                None => style.bg(rgb(palette().surface_hover)),
            })
            .when_some(workspace_color, |row, color| {
                let mut tint = gpui_kit::rgba(color);
                tint.a *= 0.18;
                row.bg(tint)
            })
            .when(can_reorder, |row| {
                row.on_drag(
                    WorkspaceDrag {
                        path: drag_path,
                        index,
                        pinned,
                    },
                    move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| WorkspaceDragPreview {
                            name: drag_name.clone(),
                        })
                    },
                )
                .drag_over::<WorkspaceDrag>(move |row, dragged, _, _| {
                    if dragged.pinned != pinned {
                        return row;
                    }
                    if dragged.path == hover_path {
                        return row.opacity(0.5);
                    }
                    // Moving down inserts after the target; moving
                    // up inserts before it. Match reorder_workspace.
                    let moving_down = dragged.index < index;
                    // A zero-blur edge shadow is an insertion
                    // marker without changing the row's box layout.
                    row.bg(rgb(palette().highlight_bg)).shadow(vec![BoxShadow {
                        color: rgb(palette().highlight_text).into(),
                        offset: point(px(0.), px(if moving_down { -2. } else { 2. })),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                        inset: true,
                    }])
                })
                .on_drop(cx.listener(
                    move |this, dragged: &WorkspaceDrag, _, cx| {
                        this.reorder_workspace(&dragged.path, &drop_path, cx);
                    },
                ))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.activate_sidebar_workspace(index, last, window, cx)))
            .child(self.sidebar_workspace_icon(index, collapsed, can_edit, cx))
            .child(div().debug_selector(move || format!("workspace-name-{index}")).flex_1().min_w_0().font_semibold().truncate().child(workspace.name()))
            .when(pinned, |row| {
                row.child(
                    div()
                        .debug_selector(move || format!("workspace-pinned-{index}"))
                        .flex_shrink_0()
                        .child(Icon::new(FullIconName::Pin).size(px(12.)).text_color(rgb(palette().muted))),
                )
            })
            .child(
                h_flex()
                    // Share one slot so the count and actions swap
                    // without moving the workspace title.
                    .relative()
                    .w(px(48.))
                    .h(px(24.))
                    .flex_shrink_0()
                    .gap_0()
                    .items_center()
                    .child(
                        h_flex()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h(px(24.))
                            .items_center()
                            .opacity(1.)
                            .group_hover("workspace-row", |style| style.opacity(0.))
                            .child(if importing {
                                Spinner::new()
                                    .small()
                                    .color(rgb(palette().faint).into())
                                    .into_any_element()
                            } else {
                                h_flex()
                                    .min_w(px(22.))
                                    .h(px(18.))
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .when_some(
                                        activity,
                                        |row, (id, running)| {
                                            row.child(session_activity_dot("workspace-activity", id, running))
                                        },
                                    )
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().faint))
                                    .child(count.to_string())
                                    .into_any_element()
                            }),
                    )
                    .child(
                        Button::new(("workspace-new-session", index))
                            .ghost()
                            .small()
                            .compact()
                            .w(px(24.))
                            .h(px(24.))
                            .p_0()
                            .opacity(0.)
                            .group_hover("workspace-row", |style| style.opacity(1.))
                            .focus_visible(|style| style.opacity(1.))
                            .icon(IconName::Plus)
                            .tooltip(conduit::i18n::text_static("sidebar.new_session_in_this_workspace"))
                            .disabled(!can_edit)
                            .when(can_edit, |button| button.cursor_pointer())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // Keep the workspace row's own click
                                // handler from reselecting the previous
                                // session right after creation.
                                cx.stop_propagation();
                                this.new_session_here(index, window, cx)
                            })),
                    )
                    .child(self.workspace_menu(index, app_entity.clone(), can_edit)),
            )
            .when(!collapsed && self.expanded_workspaces.contains(&index), |row| {
                row.child(
                    Button::new(("workspace-show-fewer", index))
                        .debug_selector(move || format!("workspace-show-fewer-{index}"))
                        .ghost()
                        .small()
                        .compact()
                        .w(px(24.))
                        .h(px(24.))
                        .p_0()
                        .flex_shrink_0()
                        .rounded(px(5.))
                        .bg(rgb(palette().surface_strong))
                        .text_color(rgb(palette().muted))
                        // Button installs its own hover style while rendering.
                        // Adding InteractiveElement::hover here would set it twice.
                        .icon(FullIconName::ListChevronsDownUp)
                        .tooltip(self.sidebar_compact_list_tooltip())
                        .accessibility_label(self.sidebar_compact_list_tooltip())
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.set_sidebar_sessions_expanded(&compact_path, false, window, cx);
                        })),
                )
            })
            .when(self.sidebar.keyboard_row.as_ref().is_some_and(|row| matches!(row, cache::Row::Workspace { path, .. } if *path == workspace.path)), |row| row.child(self.sidebar_focus_indicator()))
            .into_any_element()
    }

    fn sidebar_workspace_icon(
        &self,
        index: usize,
        collapsed: bool,
        can_edit: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let path = self.store.workspaces[index].path.clone();
        let picker_open = self.workspace_color_target.as_ref() == Some(&path)
            && self.workspace_color_picker.read(cx).is_open();
        let label = conduit::i18n::text_static("sessions.workspace_color");
        let folder = div()
            .debug_selector(move || format!("workspace-folder-{index}"))
            .size(px(TREE_MARK_SIZE))
            .child(
                Icon::new(if collapsed {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .size(px(TREE_MARK_SIZE))
                .text_color(rgb(palette().muted)),
            );
        if !can_edit {
            return Button::new(("workspace-collapse", index))
                .debug_selector(move || format!("workspace-collapse-{index}"))
                .ghost()
                .small()
                .compact()
                .size(px(TREE_TOGGLE_SIZE))
                .p_0()
                .flex_shrink_0()
                .cursor_pointer()
                .child(folder)
                .tooltip(if collapsed {
                    conduit::i18n::text_static("sidebar.expand_workspace")
                } else {
                    conduit::i18n::text_static("sidebar.collapse_workspace")
                })
                .accessibility_label(if collapsed {
                    conduit::i18n::text_static("sidebar.expand_workspace")
                } else {
                    conduit::i18n::text_static("sidebar.collapse_workspace")
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.toggle_sidebar_workspace(&path, window, cx);
                }))
                .into_any_element();
        }
        let palette_icon = Icon::new(FullIconName::Palette)
            .size(px(TREE_MARK_SIZE))
            .text_color(rgb(palette().muted));
        let control = if picker_open {
            gpui_kit::component::color_picker::ColorPicker::new(&self.workspace_color_picker)
                .icon(palette_icon)
                .with_size(Size::Small)
                .accessibility_label(label)
                .size(px(TREE_TOGGLE_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .into_any_element()
        } else {
            Button::new(("workspace-color-trigger", index))
                .ghost()
                .small()
                .compact()
                .size(px(TREE_TOGGLE_SIZE))
                .p_0()
                .opacity(0.)
                .group_hover("workspace-row", |style| style.opacity(1.))
                .focus_visible(|style| style.opacity(1.).bg(rgb(palette().surface_hover)))
                .child(palette_icon)
                .tooltip(label)
                .accessibility_label(label)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    if !this.writable || this.closing {
                        return;
                    }
                    this.workspace_color_target = Some(path.clone());
                    let color = this.store.ui.workspace_colors.get(&path).copied();
                    this.workspace_color_picker.update(cx, |picker, cx| {
                        match color {
                            Some(color) => picker.set_value(
                                gpui_kit::Hsla::from(gpui_kit::rgba(color)),
                                window,
                                cx,
                            ),
                            None => picker.clear_value(window, cx),
                        }
                        picker.set_open(true, cx);
                        picker.focus_handle(cx).focus(window, cx);
                    });
                    this.sidebar_view.update(cx, |_, cx| cx.notify());
                    cx.notify();
                }))
                .into_any_element()
        };
        div()
            .id(("workspace-color-slot", index))
            .debug_selector(move || format!("workspace-color-trigger-{index}"))
            .relative()
            .size(px(TREE_TOGGLE_SIZE))
            .flex_shrink_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                folder
                    .absolute()
                    .top(px(4.))
                    .left(px(4.))
                    .when(picker_open, |folder| folder.opacity(0.))
                    .group_hover("workspace-row", |style| style.opacity(0.)),
            )
            .child(control)
            .into_any_element()
    }
}

fn session_menu(
    menu: PopupMenu,
    menu_app: Entity<ConduitApp>,
    id: u64,
    app: &ConduitApp,
) -> PopupMenu {
    let Some(session) = app.session(id) else {
        return menu;
    };
    let native_id = session.native_id.clone();
    let pinned = session.pinned;
    let archived = session.archived;
    let can_edit = app.writable && !app.closing;
    let can_ai_rename = can_edit && session.has_user_messages();
    let pin_app = menu_app.clone();
    let rename_app = menu_app.clone();
    let ai_rename_app = menu_app.clone();
    let archive_app = menu_app.clone();
    let delete_app = menu_app.clone();
    let has_native_id = native_id.is_some();
    let copy_native_id = native_id.clone();
    menu.min_w(px(150.))
        .item(
            PopupMenuItem::new(if pinned {
                conduit::i18n::text_static("sidebar.unpin")
            } else {
                conduit::i18n::text_static("sidebar.pin_session")
            })
            .icon(if pinned {
                FullIconName::PinOff
            } else {
                FullIconName::Pin
            })
            .disabled(!can_edit)
            .on_click(move |_, _, cx| {
                pin_app.update(cx, |this, cx| this.toggle_pin(id, cx));
            }),
        )
        .item(
            PopupMenuItem::new(conduit::i18n::text_static("sidebar.rename"))
                .icon(IconName::Replace)
                .on_click(move |_, window, cx| {
                    rename_app.update(cx, |this, cx| this.open_rename_dialog(id, window, cx));
                }),
        )
        .item(
            PopupMenuItem::new(conduit::i18n::text_static("sidebar.rename_with_ai"))
                .icon(FullIconName::Pencil)
                .disabled(!can_ai_rename)
                .on_click(move |_, window, cx| {
                    ai_rename_app.update(cx, |this, cx| {
                        this.switch_sidebar_session(id, window, cx);
                        this.ai_rename_session(id, cx);
                    });
                }),
        )
        .item(
            PopupMenuItem::new(conduit::i18n::text_static("sidebar.copy_session_id"))
                .icon(IconName::Copy)
                .disabled(!has_native_id)
                .on_click(move |_, _, cx| {
                    if let Some(native_id) = copy_native_id.clone() {
                        cx.write_to_clipboard(ClipboardItem::new_string(native_id));
                    }
                }),
        )
        .item(
            PopupMenuItem::new(if archived {
                conduit::i18n::text_static("sidebar.unarchive")
            } else {
                conduit::i18n::text_static("sidebar.archive")
            })
            .icon(IconName::Inbox)
            .on_click(move |_, _, cx| {
                archive_app.update(cx, |this, cx| this.toggle_archive(id, cx));
            }),
        )
        .item(
            PopupMenuItem::new(conduit::i18n::text_static("git_state.deleted"))
                .icon(IconName::Delete)
                .on_click(move |_, _, cx| {
                    delete_app.update(cx, |this, cx| this.request_delete_session(id, cx));
                }),
        )
}
