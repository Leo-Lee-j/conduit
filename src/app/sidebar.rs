//! Workspace navigation and session list.

use super::*;
use gpui_kit::component::status_bar::StatusBar;

/// Shared frame, toolbar and horizontal gutter for the left navigation pages.
/// Scrolling and bottom borders remain page-specific rather than being nested
/// inside a padded frame, so they can extend to the pane edges.
pub(super) fn navigation_pane() -> Div {
    v_flex().size_full().min_w_0().bg(rgb(palette().sidebar_bg))
}

pub(super) fn navigation_header() -> Div {
    h_flex()
        .h(px(34.))
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
            .text_size(px(13.))
            .text_color(rgb(palette().text))
            .child(
                Icon::new(IconName::Folder)
                    .small()
                    .text_color(rgb(palette().highlight_text)),
            )
            .child(div().flex_1().min_w_0().truncate().child(self.name.clone()))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child("移动"),
            )
    }
}

/// Cache session navigation independently of wheel-driven transcript frames.
pub(super) struct SidebarView {
    app: WeakEntity<ConduitApp>,
    _subscription: Subscription,
    #[cfg(test)]
    pub(super) render_count: usize,
}

impl SidebarView {
    pub(super) fn new(app: WeakEntity<ConduitApp>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&app.upgrade().expect("app is alive"), |_, _, cx| {
            cx.notify()
        });
        Self {
            app,
            _subscription: subscription,
            #[cfg(test)]
            render_count: 0,
        }
    }
}

impl Render for SidebarView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        self.app
            .update(cx, |app, cx| app.render_navigation(cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl ConduitApp {
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

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_edit = self.writable && !self.closing;
        let app_entity = cx.entity();
        let mut sidebar = navigation_pane()
            .id("sidebar")
            .debug_selector(|| "sidebar".into())
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
                            .tooltip(format!("新建 {} 会话", self.new_session_provider.label()))
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
                            .tooltip("搜索会话")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_search_dialog(window, cx)
                            })),
                    )
                    .child(div().flex_1()),
            );
        let mut list = navigation_content()
            .id("workspace-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt(px(0.))
            .gap_1();
        let query = self.search_query.trim().to_lowercase();
        list = list.child(
            h_flex()
                .flex_shrink_0()
                .px_2()
                .pb_1()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("工作区"),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .id("toggle-archived")
                                .h(px(22.))
                                .px_2()
                                .gap_1()
                                .items_center()
                                .rounded(px(6.))
                                .text_size(px(10.))
                                .text_color(rgb(if self.show_archived {
                                    palette().text
                                } else {
                                    palette().faint
                                }))
                                .when(self.show_archived, |row| {
                                    row.bg(rgb(palette().surface_hover))
                                })
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(rgb(palette().surface_hover))
                                        .text_color(rgb(palette().text))
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_archived = !this.show_archived;
                                    cx.notify();
                                }))
                                .child(Icon::new(IconName::Inbox).small())
                                .child("已归档"),
                        )
                        .child(
                            h_flex()
                                .id("add-workspace")
                                .w(px(22.))
                                .h(px(22.))
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .when(can_edit, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgb(palette().surface_hover)))
                                        .on_click(cx.listener(Self::add_workspace))
                                })
                                .child(Icon::new(IconName::Plus).small().text_color(rgb(
                                    if can_edit {
                                        palette().muted
                                    } else {
                                        palette().faint
                                    },
                                ))),
                        ),
                ),
        );
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            let sessions: Vec<&SessionRecord> = workspace
                .sessions
                .iter()
                .filter(|session| {
                    (self.show_archived || !session.archived) && session_matches(session, &query)
                })
                .collect();
            if !query.is_empty() && sessions.is_empty() {
                continue;
            }
            let last = sessions
                .first()
                .map(|session| session.id)
                .or_else(|| workspace.sessions.first().map(|session| session.id));
            let workspace_selected = self.selected_workspace == Some(index);
            let count = sessions.len();
            let limit = self.store.ui.sessions_per_workspace.clamp(1, 30);
            let expanded = self.expanded_workspaces.contains(&index);
            let collapsed = self.collapsed_workspaces.contains(&workspace.path);
            let toggle_path = workspace.path.clone();
            let importing = self.history_loading.contains(&workspace.path);
            let drag_path = workspace.path.clone();
            let drop_path = workspace.path.clone();
            let hover_path = workspace.path.clone();
            let drag_name = workspace
                .path
                .file_name()
                .unwrap_or(workspace.path.as_os_str())
                .to_string_lossy()
                .into_owned();
            let total = sessions.len();
            list =
                list.child(
                    h_flex()
                        .id(("workspace", index))
                        .debug_selector(move || format!("workspace-item-{index}"))
                        .group("workspace-row")
                        .relative()
                        .h(px(30.))
                        .flex_shrink_0()
                        .px_2()
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .text_size(px(13.))
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
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch(index, last, window, cx)
                        }))
                        .child(
                            Button::new(("workspace-collapse", index))
                                .debug_selector(move || format!("workspace-collapse-{index}"))
                                .ghost()
                                .small()
                                .compact()
                                .w(px(18.))
                                .h(px(22.))
                                .p_0()
                                .cursor_pointer()
                                .icon(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .tooltip(if collapsed {
                                    "展开工作区"
                                } else {
                                    "折叠工作区"
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
                                .w(px(44.))
                                .h(px(22.))
                                .flex_shrink_0()
                                .gap_0()
                                .items_center()
                                .child(
                                    h_flex()
                                        .absolute()
                                        .top_0()
                                        .right_0()
                                        .h(px(22.))
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
                                                .text_size(px(10.))
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
                                        .w(px(22.))
                                        .h(px(22.))
                                        .p_0()
                                        .opacity(0.)
                                        .group_hover("workspace-row", |style| style.opacity(1.))
                                        .focus_visible(|style| style.opacity(1.))
                                        .icon(IconName::Plus)
                                        .tooltip("在此工作区新建会话")
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
                        ),
                );
            if collapsed {
                continue;
            }
            if importing {
                list = list.child(
                    h_flex()
                        .h(px(30.))
                        .flex_shrink_0()
                        .pl(px(24.))
                        .pr(px(8.))
                        .gap_2()
                        .items_center()
                        .text_size(px(12.))
                        .text_color(rgb(palette().faint))
                        .child(Spinner::new().small().color(rgb(palette().faint).into()))
                        .child(
                            self.session_control
                                .scan_progress
                                .get(&workspace.path)
                                .map(|(completed, total)| {
                                    format!("正在加载会话历史：{completed}/{total}")
                                })
                                .unwrap_or_else(|| "正在发现会话历史…".into()),
                        ),
                );
            }
            for session in sessions.iter().take(if expanded { total } else { limit }) {
                let id = session.id;
                let native_id = session.native_id.clone();
                let active = self.selected_session == Some(id);
                let running = self.session_control.runs.contains_key(&id);
                let unread = self.session_control.unread_completed.contains(&id);
                let archived = session.archived;
                let can_ai_rename = can_edit
                    && session
                        .messages
                        .iter()
                        .any(|item| item.kind == ItemKind::User);
                list = list.child(
                    h_flex()
                        .id(("session", id))
                        .debug_selector(move || format!("session-row-{id}"))
                        .relative()
                        .h(px(30.))
                        .flex_shrink_0()
                        .pl(px(24.))
                        .pr(px(8.))
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .text_size(px(13.))
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
                            this.switch(index, Some(id), window, cx)
                        }))
                        .context_menu({
                            let menu_app = app_entity.clone();
                            move |menu, _, _| {
                                let rename_app = menu_app.clone();
                                let ai_rename_app = menu_app.clone();
                                let archive_app = menu_app.clone();
                                let delete_app = menu_app.clone();
                                let has_native_id = native_id.is_some();
                                let copy_native_id = native_id.clone();
                                menu.min_w(px(150.))
                                    .item(
                                        PopupMenuItem::new("重命名")
                                            .icon(IconName::Replace)
                                            .on_click(move |_, window, cx| {
                                                rename_app.update(cx, |this, cx| {
                                                    this.open_rename_dialog(id, window, cx)
                                                });
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new("AI 重命名")
                                            .icon(gpui_kit::assets::IconName::Sparkles)
                                            .disabled(!can_ai_rename)
                                            .on_click(move |_, window, cx| {
                                                ai_rename_app.update(cx, |this, cx| {
                                                    this.switch(index, Some(id), window, cx);
                                                    this.ai_rename_session(id, cx);
                                                });
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new("复制会话 ID")
                                            .icon(IconName::Copy)
                                            .disabled(!has_native_id)
                                            .on_click(move |_, _, cx| {
                                                if let Some(native_id) = copy_native_id.clone() {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(native_id),
                                                    );
                                                }
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new(if archived {
                                            "取消归档"
                                        } else {
                                            "归档"
                                        })
                                        .icon(IconName::Inbox)
                                        .on_click(
                                            move |_, _, cx| {
                                                archive_app.update(cx, |this, cx| {
                                                    this.toggle_archive(id, cx)
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        PopupMenuItem::new("删除").icon(IconName::Delete).on_click(
                                            move |_, _, cx| {
                                                delete_app.update(cx, |this, cx| {
                                                    this.request_delete_session(id, cx)
                                                });
                                            },
                                        ),
                                    )
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
                                    "新会话".to_string()
                                } else {
                                    single_line_title(&session.title)
                                }),
                        )
                        .when(self.session_control.namings.contains_key(&id), |row| {
                            row.child(Spinner::new().small().color(rgb(palette().muted).into()))
                        })
                        .when(archived, |row| {
                            row.child(
                                div()
                                    .text_size(px(9.))
                                    .text_color(rgb(palette().faint))
                                    .child("已归档"),
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
                        .when_some(session_age_label(session.created_at), |row, age| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .px(px(5.))
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .bg(rgb(palette().surface_strong))
                                    .text_size(px(9.))
                                    .text_color(rgb(palette().faint))
                                    .child(age),
                            )
                        }),
                );
            }
            if total > limit {
                let toggle_app = app_entity.clone();
                let more_label = if expanded {
                    "收起".to_string()
                } else {
                    format!("显示更多（还有 {} 条）", total - limit)
                };
                list = list.child(
                    h_flex()
                        .id(("workspace-more", index))
                        .h(px(24.))
                        .flex_shrink_0()
                        .pl(px(24.))
                        .pr(px(4.))
                        .items_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(palette().surface_hover)))
                        .on_click(move |_, _, cx| {
                            toggle_app.update(cx, |this, cx| {
                                if !this.expanded_workspaces.insert(index) {
                                    this.expanded_workspaces.remove(&index);
                                }
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette().faint))
                                .child(more_label),
                        ),
                );
            }
        }
        if self.store.workspaces.is_empty() {
            list = list.child(
                v_flex()
                    .flex_shrink_0()
                    .mt_2()
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
                                    .text_size(px(13.))
                                    .text_color(rgb(palette().muted))
                                    .child("还没有工作区"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
                            .child("点击上方 + 添加一个本地目录"),
                    ),
            );
        }
        // Mirrors the conversation toolbar height so the two bottom bars read
        // as one strip; icons are packed right-to-left like the right side.
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
                .border_color(rgb(palette().border))
                .right(
                    Button::new("sidebar-about")
                        .ghost()
                        .small()
                        .compact()
                        .icon(IconName::Info)
                        .tooltip("关于 Conduit")
                        .accessibility_label("关于")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_about_dialog(window, cx);
                        })),
                ),
        );
        sidebar.into_any_element()
    }
}
