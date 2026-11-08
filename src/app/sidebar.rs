//! Workspace navigation and session list.

use super::*;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::status_bar::StatusBar;

mod cache;
mod keyboard;
pub(super) use cache::SidebarState;

// Keep the virtual-list estimates and rendered navigation rows in agreement.
const ROW_HEIGHT: f32 = 36.;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

/// Shared frame, toolbar and horizontal gutter for the left navigation pages.
/// Scrolling and bottom borders remain page-specific rather than being nested
/// inside a padded frame, so they can extend to the pane edges.
pub(super) fn navigation_pane() -> Div {
    v_flex().size_full().min_w_0().bg(rgb(palette().sidebar_bg))
}

pub(super) fn navigation_header() -> Div {
    h_flex()
        .h(px(ROW_HEIGHT))
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
    fn refresh_sidebar(&mut self) -> bool {
        let inputs = cache::Inputs::new(self);
        self.sidebar
            .refresh(&self.store, &self.session_control, &self.search_ui, inputs)
    }

    /// Reorder by stable paths and remap all index-based navigation state.
    pub(super) fn reorder_workspace(
        &mut self,
        source: &Path,
        target: &Path,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
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
        let can_edit = self.writable && !self.closing;
        let mut sidebar =
            navigation_pane()
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
                .focus_visible(|style| style.border_1().border_color(rgb(palette().focus)))
                .on_key_down(cx.listener(Self::sidebar_key))
                .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                    // A row-owned confirmation should not reopen when its row is recycled.
                    if this.delete_target.take().is_some() {
                        cx.notify();
                    }
                }))
                // No bottom padding: the bottom bar aligns with the conversation toolbar.
                .gap_0()
                .child(
                    navigation_header()
                        .gap_1()
                        .child(
                            Button::new("sidebar-new-session")
                                .ghost()
                                .small()
                                .compact()
                                .icon(IconName::Plus)
                                .tooltip(conduit::i18n::format(
                                    "sidebar.new_session",
                                    &[("arg0", self.new_session_provider.label().to_string())],
                                ))
                                .disabled(!can_edit || self.selected_workspace.is_none())
                                .when(can_edit && self.selected_workspace.is_some(), |button| {
                                    button.cursor_pointer()
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(index) = this.selected_workspace {
                                        this.new_session_here(index, window, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new("sidebar-search")
                                .ghost()
                                .small()
                                .compact()
                                .icon(IconName::Search)
                                .tooltip(conduit::i18n::text_static("localization.search_sessions"))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_search_dialog(window, cx)
                                })),
                        )
                        .child(
                            Button::new("sidebar-file-search")
                                .icon(FullIconName::FileSearch)
                                .small()
                                .ghost()
                                .tooltip(conduit::i18n::text_static(
                                    "sidebar.search_files_double_shift",
                                ))
                                .accessibility_label(conduit::i18n::text_static(
                                    "file_picker.search_files",
                                ))
                                .disabled(self.selected_workspace.is_none())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_file_search(window, cx)
                                })),
                        )
                        .child(div().flex_1()),
                );
        let rows = self.sidebar.rows.clone();
        let list = gpui_kit::component::v_virtual_list(
            cx.entity(),
            "workspace-list",
            self.sidebar.sizes.clone(),
            move |app, range, _, cx| {
                range
                    .map(|ix| app.render_sidebar_row(&rows[ix], cx))
                    .collect()
            },
        )
        .track_scroll(&self.sidebar.scroll)
        .flex_1()
        .min_h_0()
        .px_2()
        .gap(px(4.));
        // Mirrors the conversation toolbar height so the two bottom bars read
        // as one strip.
        sidebar = sidebar.child(list).child(
            StatusBar::new()
                .h(px(34.))
                .min_w_0()
                .flex_shrink_0()
                .py_0()
                .bg(rgb(palette().sidebar_bg))
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

    fn render_sidebar_row(&self, row: &cache::Row, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(test)]
        self.sidebar
            .rendered_rows
            .set(self.sidebar.rendered_rows.get() + 1);
        let app_entity = cx.entity();
        let element = match row {
            cache::Row::Controls => self.sidebar_controls(cx),
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
                let Some(index) = row
                    .path()
                    .and_then(|path| self.sidebar.workspace_index(path))
                else {
                    return div().h(px(row.height())).into_any_element();
                };
                match row {
                    cache::Row::Workspace { count, last, .. } => {
                        self.sidebar_workspace(index, *count, *last, cx)
                    }
                    cache::Row::Loading(_) => {
                        let workspace = &self.store.workspaces[index];
                        h_flex()
                            .h(px(ROW_HEIGHT))
                            .flex_shrink_0()
                            .pl(px(24.))
                            .pr(px(8.))
                            .gap_2()
                            .items_center()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(Spinner::new().small().color(rgb(palette().faint).into()))
                            .child(
                                self.session_control
                                    .scan_progress
                                    .get(&workspace.path)
                                    .map(|(completed, total)| {
                                        conduit::i18n::format(
                                            "sidebar.loading_session_history",
                                            &[
                                                ("completed", format!("{}", completed)),
                                                ("total", format!("{}", total)),
                                            ],
                                        )
                                    })
                                    .unwrap_or_else(|| {
                                        conduit::i18n::text_static(
                                            "sidebar.discovering_session_history",
                                        )
                                        .into()
                                    }),
                            )
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
                            .relative()
                            .h(px(ROW_HEIGHT))
                            .flex_shrink_0()
                            .pl(px(24.))
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
                                palette().muted
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
                            .child(provider_icon(session.provider))
                            .child(
                                div()
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
                            .when(self.delete_target == Some(id), |row| {
                                row.child(
                                    div()
                                        .absolute()
                                        .right_0()
                                        .top_0()
                                        .child(self.delete_session_popover(id, cx)),
                                )
                            })
                            .when_some(session_age_label(session.last_activity_at()), |row, age| {
                                row.child(
                                    div()
                                        .flex_shrink_0()
                                        .px(px(5.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(rgb(palette().surface_strong))
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().faint))
                                        .child(age),
                                )
                            })
                            .into_any_element()
                    }
                    cache::Row::More {
                        total, expanded, ..
                    } => {
                        let total = *total;
                        let expanded = *expanded;
                        let limit = self.store.ui.sessions_per_workspace.clamp(1, 30);
                        let toggle_app = app_entity.clone();
                        let more_label = if expanded {
                            conduit::i18n::text_static("sidebar.collapse_session_list").to_string()
                        } else {
                            conduit::i18n::format(
                                "sidebar.show_more_remaining",
                                &[("arg0", format!("{}", total - limit))],
                            )
                        };

                        h_flex()
                            .h(px(28.))
                            // Center the glyph under the 14px session icons at a 24px inset.
                            .pl(px(17.))
                            .items_center()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "workspace-more-{}",
                                    self.store.workspaces[index].path.display()
                                )))
                                .debug_selector(move || format!("workspace-more-{index}"))
                                .icon(if expanded {
                                    IconName::ChevronUp
                                } else {
                                    IconName::Ellipsis
                                })
                                .tooltip(more_label.clone())
                                .accessibility_label(more_label)
                                .small()
                                .compact()
                                .ghost()
                                .h(px(28.))
                                .w(px(28.))
                                .text_color(rgb(palette().muted))
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    toggle_app.update(cx, |this, cx| {
                                        if !this.expanded_workspaces.insert(index) {
                                            this.expanded_workspaces.remove(&index);
                                        }
                                        cx.notify();
                                    });
                                }),
                            )
                            .into_any_element()
                    }
                    _ => unreachable!(),
                }
            }
        };
        div()
            .w_full()
            .h(px(row.height()))
            .when(
                self.sidebar
                    .keyboard_row
                    .as_ref()
                    .is_some_and(|focused| focused.same_identity(row)),
                |row| row.border_1().border_color(rgb(palette().focus)),
            )
            .child(element)
            .into_any_element()
    }

    fn switch_sidebar_session(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((workspace, _)) = self.session_control.locate(&self.store.workspaces, id) {
            self.switch(workspace, Some(id), window, cx);
        }
    }

    fn sidebar_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_edit = self.writable && !self.closing;
        h_flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_2()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().faint))
                    .child(conduit::i18n::text_static("git_compare.workspace")),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("toggle-archived")
                            .label(conduit::i18n::text_static("search.archived"))
                            .icon(IconName::Inbox)
                            .small()
                            .ghost()
                            .h(px(28.))
                            .selected(self.show_archived)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_archived = !this.show_archived;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("add-workspace")
                            .icon(IconName::Plus)
                            .small()
                            .ghost()
                            .w(px(28.))
                            .h(px(28.))
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
        let collapsed = self.collapsed_workspaces.contains(&workspace.path);
        let importing = self.history_loading.contains(&workspace.path);
        let toggle_path = workspace.path.clone();
        let drag_path = workspace.path.clone();
        let drop_path = workspace.path.clone();
        let hover_path = workspace.path.clone();
        let drag_name = workspace.name();
        h_flex()
            .id(SharedString::from(format!(
                "workspace-{}",
                workspace.path.display()
            )))
            .debug_selector(move || format!("workspace-item-{index}"))
            .role(Role::TreeItem)
            .aria_label(workspace.name())
            .aria_expanded(!collapsed)
            .aria_selected(workspace_selected)
            .when(self.sidebar.keyboard_row.as_ref().is_some_and(|row| matches!(row, cache::Row::Workspace { path, .. } if *path == workspace.path)), |row| row.aria_active_descendant())
            .group("workspace-row")
            .relative()
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(8.))
            .text_size(ui_text_size(13.))
            .text_color(rgb(if workspace_selected {
                palette().text
            } else {
                palette().muted
            }))
            .when(workspace_selected, |row| row.bg(rgb(palette().surface)))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(palette().surface_hover)))
            .when(can_edit, |row| {
                row.on_drag(
                    WorkspaceDrag {
                        path: drag_path,
                        index,
                    },
                    move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| WorkspaceDragPreview {
                            name: drag_name.clone(),
                        })
                    },
                )
                .drag_over::<WorkspaceDrag>(move |row, dragged, _, _| {
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
            .on_click(cx.listener(move |this, _, window, cx| this.switch(index, last, window, cx)))
            .child(
                Button::new(("workspace-collapse", index))
                    .debug_selector(move || format!("workspace-collapse-{index}"))
                    .ghost()
                    .small()
                    .compact()
                    .w(px(24.))
                    .h(px(24.))
                    .p_0()
                    .cursor_pointer()
                    .icon(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .tooltip(if collapsed {
                        conduit::i18n::text_static("sidebar.expand_workspace")
                    } else {
                        conduit::i18n::text_static("sidebar.collapse_workspace")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if !this.collapsed_workspaces.insert(toggle_path.clone()) {
                            this.collapsed_workspaces.remove(&toggle_path);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Icon::new(IconName::Folder)
                    .small()
                    .text_color(rgb(palette().faint)),
            )
            .child(div().flex_1().truncate().child(workspace.name()))
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
                                    .px(px(5.))
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(4.))
                                    .bg(rgb(palette().surface_strong))
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
                    .child(self.workspace_menu(index, app_entity.clone())),
            )
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
