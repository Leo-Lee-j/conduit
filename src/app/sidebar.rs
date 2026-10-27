//! Workspace navigation and session list.

use super::*;
use gpui_kit::component::status_bar::StatusBar;

impl ConduitApp {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_edit = self.writable && !self.closing;
        let app_entity = cx.entity();
        let mut sidebar = v_flex()
            .id("sidebar")
            .h_full()
            .w_full()
            .bg(rgb(palette().sidebar_bg))
            // No bottom padding: the bottom bar must sit on the window edge so
            // its 34px strip lines up with the conversation toolbar.
            .px_2()
            .gap_0()
            .child(
                h_flex()
                    .h(px(34.))
                    .flex_shrink_0()
                    .px_1()
                    .gap_1()
                    .items_center()
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
        let mut list = v_flex()
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
            let importing = self.history_loading.contains(&workspace.path);
            let total = sessions.len();
            list =
                list.child(
                    h_flex()
                        .id(("workspace", index))
                        .group("workspace-row")
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
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch(index, last, window, cx)
                        }))
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
            if importing && sessions.is_empty() {
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
                        .child("正在加载会话历史…"),
                );
            }
            for session in sessions.iter().take(if expanded { total } else { limit }) {
                let id = session.id;
                let native_id = session.native_id.clone();
                let active = self.selected_session == Some(id);
                let running = self.runs.contains_key(&id);
                let unread = self.unread_completed.contains(&id);
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
                        .when(self.session_namings.contains_key(&id), |row| {
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
                )
                .right(
                    Button::new("sidebar-settings")
                        .ghost()
                        .small()
                        .compact()
                        .icon(IconName::Settings)
                        .tooltip("设置")
                        .accessibility_label("设置")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_settings(window, cx);
                        })),
                ),
        );
        sidebar.into_any_element()
    }
}
