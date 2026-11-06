//! Window layout, sidebar, conversation, composer, and toolbar.

use super::*;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

// All navigation modes share one range and one retained resize state.
pub(super) const NAVIGATION_MIN_WIDTH: f32 = 200.;
pub(super) const NAVIGATION_MAX_WIDTH: f32 = 420.;
pub(super) const NAVIGATION_DEFAULT_WIDTH: f32 = NAVIGATION_MAX_WIDTH;

/// Keep wheel-driven invalidation inside the transcript. The list notifies its
/// owning view on every wheel event, even when no surrounding controls change.
pub(super) struct ChatTimeline {
    app: WeakEntity<ConduitApp>,
    scroll: ListState,
}

impl ChatTimeline {
    pub(super) fn new(app: WeakEntity<ConduitApp>, scroll: ListState) -> Self {
        Self { app, scroll }
    }

    fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        origin: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(20.)).y;
        self.scroll
            .set_offset_from_scrollbar(point(px(0.), origin.y + delta));
        let top = self.scroll.logical_scroll_top();
        let at_bottom = self
            .scroll
            .is_scrolled_to_end()
            .unwrap_or(top.item_ix >= self.scroll.item_count());
        let _ = self.app.update(cx, |app, cx| {
            let navigating = app.pending_message_jump.take().is_some();
            let outline = app
                .outline
                .active_for_item(app.chat_rows.message(top.item_ix));
            // Let message_outline update its marker and scroll the rail when
            // the active section changes. Wheel packets within a section only
            // invalidate this timeline.
            if navigating || app.at_bottom != at_bottom || app.outline.active != outline {
                app.at_bottom = at_bottom;
                cx.notify();
            }
        });
        cx.notify();
    }
}

impl Render for ChatTimeline {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(app) = self.app.upgrade() {
            let app = app.read(cx);
            let monitor = app
                .store
                .ui
                .performance_monitor
                .then(|| app.performance.clone());
            if let Some(monitor) = monitor {
                monitor.update(cx, |monitor, _| monitor.begin_message_frame());
            }
        }
        let app = self.app.clone();
        let list = gpui_kit::list(self.scroll.clone(), move |index, _, cx| {
            app.update(cx, |app, cx| app.render_message(index, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        let origin = std::rc::Rc::new(std::cell::Cell::new(Point::<Pixels>::default()));
        let capture_origin = origin.clone();
        let scroll = self.scroll.clone();
        div()
            .relative()
            .size_full()
            .pt(px(5.))
            .on_scroll_wheel(cx.listener(move |this, event, _, cx| {
                this.handle_scroll(event, origin.get(), cx);
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        // gpui-pre 0.3.9 coalesces wheel packets against the
                        // last painted anchor, losing movement on reversals.
                        // Capture the live offset before its listener, then
                        // replace that result in our bubble handler. Nested
                        // scrollers get first refusal and can stop bubbling.
                        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, _| {
                            if phase == DispatchPhase::Capture && bounds.contains(&event.position) {
                                let offset = scroll.scroll_px_offset_for_scrollbar();
                                // scroll_to_end can temporarily use an item-count
                                // sentinel; normalize it to the viewport's top.
                                capture_origin.set(point(
                                    px(0.),
                                    offset.y.max(-scroll.max_offset_for_scrollbar().y),
                                ));
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(list)
    }
}

impl ConduitApp {
    /// The window's first frame: shown until the initial CLI history scan
    /// merges, so the main layout never renders a half-imported workspace
    /// list. The title bar stays live so the window can still be dragged,
    /// moved, and closed while loading.
    fn startup_page(&self) -> AnyElement {
        let title_bar = TitleBar::new()
            .bg(rgb(palette().canvas))
            .border_color(rgb(palette().border))
            .child(
                h_flex().items_center().gap_2().child(app_icon(20.)).child(
                    div()
                        .text_size(px(13.))
                        .font_semibold()
                        .text_color(rgb(palette().text))
                        .child("Conduit"),
                ),
            );
        let workspaces = self.store.workspaces.len();
        v_flex()
            .id("startup-loading")
            .debug_selector(|| "startup-loading".into())
            .relative()
            .size_full()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .child(title_bar)
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(app_icon(64.))
                    .child(div().text_size(px(20.)).font_semibold().child("Conduit"))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Spinner::new().small().color(rgb(palette().loading).into()))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!("正在加载 {workspaces} 个工作区的 CLI 历史…")),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ConduitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The startup page owns the window until the initial history scan
        // merges, so the workspace sidebar never renders a half-imported
        // list.
        if self.startup.is_some() {
            return self.startup_page();
        }
        self.sync_plugin_context();
        self.refresh_plugin_views(cx);
        if self.settings_ui.open {
            return self.render_settings_workspace(window, cx);
        }
        self.ensure_selected_transcript(cx);
        let render_started = Instant::now();
        let app_entity = cx.entity();
        let selected = self
            .selected_session
            .and_then(|id| self.session(id))
            .map(SessionSummary::from);
        let message_count = selected.as_ref().map_or(0, |session| session.message_count);
        let outline_session = self.selected_session.and_then(|id| {
            self.store
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.sessions)
                .find(|session| session.id == id)
        });
        self.outline.refresh(outline_session);
        self.sync_chat_rows();
        if self.at_bottom && self.pending_message_jump.is_none() {
            self.scroll.scroll_to_end();
        }
        // Parent changes include streaming content, settings and row expansion.
        self.timeline.update(cx, |_, cx| cx.notify());
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.begin_render(
                    message_count,
                    self.session_control.runs.len(),
                    self.store.workspaces.iter().map(|w| w.sessions.len()).sum(),
                );
            });
        }
        let has_workspace = self.selected_workspace.is_some();
        let directory = self
            .selected_workspace
            .map(|i| self.store.workspaces[i].name())
            .unwrap_or_else(|| "未选择工作区".into());
        let sidebar = self
            .sidebar_view
            .clone()
            .cached(StyleRefinement::default().size_full());
        let mut conversation = if self.branch_navigation_open() {
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .h_full()
                .children(self.branch_manager.clone())
        } else if self.active_diff.is_some() {
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(self.render_diff_document(cx))
        } else if self.active_file.is_some() {
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(
                    self.file_view
                        .clone()
                        .cached(StyleRefinement::default().w_full().flex_1().min_h_0()),
                )
        } else {
            let mut messages = div()
                .v_flex()
                .id("messages")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .min_w_0()
                .px_2()
                .py_1()
                .gap_5()
                .on_scroll_wheel(cx.listener(|this, _: &ScrollWheelEvent, _, cx| {
                    if this.pending_message_jump.take().is_some() {
                        this.at_bottom = false;
                        cx.notify();
                    }
                }));
            if let Some(session) = &selected {
                let unloaded = self
                    .session(session.id)
                    .is_some_and(|record| !record.transcript.loaded);
                if unloaded {
                    let id = session.id;
                    let error = self.transcripts.errors.get(&id).cloned();
                    let mut loading = v_flex()
                        .id("transcript-loading")
                        .debug_selector(|| "transcript-loading".into())
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .px_4()
                        .child(div().text_size(px(14.)).child(if error.is_some() {
                            "会话正文加载失败"
                        } else {
                            "正在加载会话…"
                        }));
                    if let Some(error) = error {
                        loading = loading
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(error),
                            )
                            .child(
                                Button::new("retry-transcript")
                                    .label("重试加载")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.retry_transcript(id, cx)
                                    })),
                            );
                    } else {
                        loading = loading.child(Spinner::new());
                    }
                    messages = messages.child(loading);
                } else if session.message_count == 0 {
                    messages = messages.child(
                        v_flex()
                            .flex_1()
                            .justify_center()
                            .items_center()
                            .gap_3()
                            .py_20()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(provider_icon(session.provider))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(palette().faint))
                                            .child(format!(
                                                "{} · 可以开始输入",
                                                session.provider.label()
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(30.))
                                    .font_bold()
                                    .text_color(rgb(palette().text))
                                    .child("今天想完成什么？"),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "在 {} 中开始你的任务；CLI 登录与模型权限将在执行时确认。",
                                        directory
                                    )),
                            ),
                    );
                }
                if session.message_count > 0 {
                    messages = div()
                        .id("messages")
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .track_focus(&self.chat_focus)
                        .on_mouse_up(MouseButton::Left, {
                            let focus = self.chat_focus.clone();
                            move |_, window, cx| {
                                // The composer holds keyboard focus, so a finished chat
                                // selection would never see Ctrl+C. Move focus onto the
                                // conversation, inside the window root's key context.
                                if !TextSelection::selected_text(window, cx).trim().is_empty() {
                                    focus.focus(window, cx);
                                }
                            }
                        })
                        .child(self.timeline.clone());
                }
            } else if has_workspace {
                messages = messages.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .py_20()
                    .child(
                        h_flex()
                            .w(px(44.))
                            .h(px(44.))
                            .items_center()
                            .justify_center()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(rgb(palette().border))
                            .bg(rgb(palette().surface))
                            .child(
                                div()
                                    .text_base()
                                    .font_bold()
                                    .text_color(rgb(palette().muted))
                                    .child("C"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("今天想完成什么？"),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "会话将在 {directory} 中运行，输入任务后会按上次使用的引擎新建会话。"
                            )),
                    ),
            );
            } else {
                messages = messages.child(self.session_heatmap(13, cx));
            }

            let mut conversation = v_flex().flex_1().min_w_0().h_full();
            conversation = conversation.child(self.session_tab_bar(cx));
            // Keep the outline attached to the centered message column instead of
            // the window edge. It stays outside the scrolling list, while the
            // rail itself is vertically centered in the chat viewport.
            let mut message_area = h_flex().flex_1().min_h_0().min_w_0().items_stretch();
            if !self.outline.entries.is_empty() {
                message_area = message_area.child(
                    h_flex()
                        .w_full()
                        .when(!self.store.ui.full_width_content, |row| {
                            row.max_w(px(952.)).mx_auto()
                        })
                        .h_full()
                        .min_h_0()
                        .min_w_0()
                        .items_stretch()
                        .child(self.message_outline(cx))
                        .child(messages)
                        // Balance the rail so messages stay aligned with the composer.
                        .child(div().w(px(32.)).flex_shrink_0()),
                );
            } else {
                message_area = message_area.child(messages);
            }
            conversation = conversation.child(message_area);
            let approval_count = self
                .selected_session
                .and_then(|id| self.session_control.runs.get(&id))
                .map(|r| r.approvals.len())
                .unwrap_or(0);
            let approvals = self
                .selected_session
                .and_then(|id| self.session_control.runs.get(&id))
                .and_then(|r| r.approvals.front().cloned());
            for (index, approval) in approvals.into_iter().enumerate() {
                if approval.is_user_input() {
                    conversation =
                        conversation.child(self.render_user_input(&approval, window, cx));
                    continue;
                }
                let submitted = self
                    .selected_session
                    .and_then(|id| self.session_control.runs.get(&id))
                    .is_some_and(|r| r.approvals.is_submitted(&approval.id));
                let id = self.selected_session.unwrap();
                let summary = notifications::approval_message(&approval);
                let detail_copy = approval.detail.clone();
                let origin = self
                    .selected_workspace_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default();
                let allow_id = approval.id.clone();
                let deny_id = approval.id;
                let toggle_id = deny_id.clone();
                let expanded = self
                    .session_control
                    .runs
                    .get(&id)
                    .is_some_and(|run| run.expanded_approvals.contains(&deny_id));
                let queue_label = if approval_count > 1 {
                    format!("第 {} / {} 项", index + 1, approval_count)
                } else {
                    String::new()
                };
                conversation = conversation.child(
                    div().w_full().px_6().mb_4().child(
                        v_flex()
                            .w_full()
                            .when(!self.store.ui.full_width_content, |card| {
                                card.max_w(px(840.)).mx_auto()
                            })
                            .p_4()
                            .gap_3()
                            .rounded(px(14.))
                            .border_1()
                            .border_color(rgb(palette().warning_border))
                            .bg(rgb(palette().warning_bg))
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .w(px(28.))
                                            .h(px(28.))
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(8.))
                                            .bg(rgb(palette().warning_icon_bg))
                                            .child(
                                                Icon::new(IconName::TriangleAlert)
                                                    .small()
                                                    .text_color(rgb(palette().warning_text)),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                h_flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_size(px(13.))
                                                            .font_semibold()
                                                            .text_color(rgb(
                                                                palette().highlight_text
                                                            ))
                                                            .child(approval.title.clone()),
                                                    )
                                                    .when(!queue_label.is_empty(), |row| {
                                                        row.child(
                                                            h_flex()
                                                                .h(px(18.))
                                                                .px_2()
                                                                .items_center()
                                                                .rounded_full()
                                                                .bg(rgb(palette().warning_chip_bg))
                                                                .text_size(px(10.))
                                                                .text_color(rgb(
                                                                    palette().warning_chip_text
                                                                ))
                                                                .child(queue_label.clone()),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .line_height(relative(1.4))
                                                    .text_color(rgb(palette().warning_muted))
                                                    .child("CLI 请求授权，仅对本次操作生效。"),
                                            ),
                                    )
                                    .child(
                                        Button::new(("approval-toggle", index))
                                            .label(if expanded { "收起" } else { "展开详情" })
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle_approval_detail(
                                                    id,
                                                    toggle_id.clone(),
                                                    cx,
                                                )
                                            })),
                                    ),
                            )
                            .child(
                                div()
                                    .id(("approval-summary", index))
                                    .max_h(px(120.))
                                    .overflow_y_scroll()
                                    .text_sm()
                                    .text_color(rgb(palette().warning_text))
                                    .child(summary),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().warning_muted))
                                    .child(format!("工作区：{origin}")),
                            )
                            .when(expanded, |column| {
                                column.child(
                                    div()
                                        .id(("approval-detail", index))
                                        .max_h(px((f32::from(window.viewport_size().height) * 0.4)
                                            .clamp(120., 360.)))
                                        .overflow_y_scroll()
                                        .p_3()
                                        .rounded(px(10.))
                                        .border_1()
                                        .border_color(rgb(palette().warning_inner_border))
                                        .bg(rgb(palette().warning_code_bg))
                                        .text_xs()
                                        .line_height(relative(1.5))
                                        .text_color(rgb(palette().warning_code_text))
                                        .child(approval.detail.clone()),
                                )
                            })
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        Button::new(("approval-copy", index))
                                            .label("复制完整请求")
                                            .small()
                                            .ghost()
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    detail_copy.clone(),
                                                ))
                                            }),
                                    )
                                    .when(!approval.allow_once, |row| {
                                        row.child(
                                            div()
                                                .flex_1()
                                                .text_size(px(11.))
                                                .text_color(rgb(palette().warning_accent))
                                                .child(
                                                    "CLI 未提供「允许本次」选项，此请求只能拒绝。",
                                                ),
                                        )
                                    })
                                    .when(approval.allow_once, |row| row.child(div().flex_1()))
                                    .child(
                                        Button::new(("deny", index))
                                            .label("拒绝")
                                            .small()
                                            .ghost()
                                            .disabled(self.closing || submitted)
                                            .when(!self.closing && !submitted, |button| {
                                                button.cursor_pointer()
                                            })
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.decide(id, deny_id.clone(), false, window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new(("allow", index))
                                            .label("允许本次")
                                            .small()
                                            .primary()
                                            .disabled(
                                                self.closing || submitted || !approval.allow_once,
                                            )
                                            .when(
                                                !self.closing && !submitted && approval.allow_once,
                                                |button| button.cursor_pointer(),
                                            )
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.decide(id, allow_id.clone(), true, window, cx)
                                            })),
                                    ),
                            ),
                    ),
                );
            }
            if let Some(message) = &self.message
                && self.writable
                && self.persistence.error.is_none()
            {
                conversation = conversation.child(
                    div()
                        .mx_6()
                        .mb_3()
                        .child(
                            gpui_kit::component::alert::Alert::error(
                                "application-message",
                                message.clone(),
                            )
                            .small()
                            .on_close(cx.listener(|app, _, _, cx| {
                                app.message = None;
                                cx.notify();
                            })),
                        )
                        .when(message.contains("AI 引擎"), |body| {
                            body.child(
                                Button::new("message-engine-settings")
                                    .label("打开 AI 引擎设置")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|app, _, window, cx| {
                                        app.open_engine_settings(window, cx)
                                    })),
                            )
                        }),
                );
            }
            conversation = conversation.child(self.render_composer(selected.as_ref(), window, cx));
            conversation
        };
        conversation = conversation
            .child(self.notification_banner(cx))
            .child(self.update_notice(cx));
        let toolbar = self.render_toolbar(cx);
        let title_bar = TitleBar::new()
            .bg(rgb(palette().canvas))
            .border_color(rgb(palette().border))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(app_icon(20.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("Conduit"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child("Chat M0 · Preview"),
                    ),
            );
        let title_bar = title_bar.child(div().flex_1()).child(
            h_flex()
                .id("title-bar-actions")
                .debug_selector(|| "title-bar-actions".into())
                .items_center()
                .gap_1()
                .mr_1()
                .flex_shrink_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new("title-settings")
                        .debug_selector(|| "title-settings".into())
                        .accessibility_label("设置")
                        .ghost()
                        .small()
                        .compact()
                        .w(px(36.))
                        .h(px(28.))
                        .flex_shrink_0()
                        .icon(IconName::Settings)
                        .tooltip("设置（Ctrl+,）")
                        .cursor_pointer()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                )
                .child(
                    Button::new("title-about")
                        .debug_selector(|| "title-about".into())
                        .accessibility_label("关于")
                        .ghost()
                        .small()
                        .compact()
                        .w(px(36.))
                        .h(px(28.))
                        .flex_shrink_0()
                        .icon(IconName::Info)
                        .tooltip("关于 Conduit")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_about_dialog(window, cx);
                        })),
                ),
        );
        let conversation = self.with_terminal_panel(conversation.into_any_element(), window, cx);
        let conversation = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(div().flex_1().min_h_0().child(conversation))
            .child(toolbar);
        // The left workspace rail stays a resizable group. The right sidebar is
        // laid out by hand so dragging its edge follows the pointer 1:1 and its
        // width can be capped at half of the remaining content area.
        let viewport_width = f32::from(window.viewport_size().width);
        let sidebar_state =
            window.use_keyed_state("body-resizable", cx, |_, _| ResizableState::default());
        let measured_sidebar = sidebar_state
            .read(cx)
            .sizes()
            .first()
            .map(|width| f32::from(*width))
            .filter(|width| *width > 0.)
            .unwrap_or(NAVIGATION_DEFAULT_WIDTH);
        let measured_sidebar = if self.plugins.collapsed() {
            38.
        } else {
            measured_sidebar
        };
        self.sidebar_width = measured_sidebar;
        let content_width = (viewport_width - measured_sidebar - 6.).max(320.);
        let max_detail_width = (content_width * 0.5).max(260.);
        self.plugins.dock.width = self.plugins.dock.width.clamp(260., max_detail_width);
        let detail_width = self.plugins.dock.width;
        let (detail_panel, overlay) = self.render_detail_dock(window, cx);

        let body = h_resizable("body").with_state(&sidebar_state).child(
            resizable_panel()
                .size(px(NAVIGATION_DEFAULT_WIDTH))
                .size_range(px(NAVIGATION_MIN_WIDTH)..px(NAVIGATION_MAX_WIDTH))
                .fixed()
                .child(sidebar),
        );
        let conversation_area: AnyElement = match detail_panel {
            Some(panel) => {
                let drag_app = app_entity.clone();
                let band = div()
                    .id("detail-resize")
                    .test_support()
                    .debug_selector(|| "detail-resize".into())
                    .relative()
                    .w(px(6.))
                    .h_full()
                    .flex_shrink_0()
                    .cursor_col_resize()
                    .hover(|style| style.bg(rgb(palette().surface_hover)))
                    .on_drag(DetailResize, move |_, position, window, cx| {
                        // `position` is relative to the band; the panel's right
                        // edge is the window edge, so recover the pointer's
                        // window x to keep the grab offset constant while dragging.
                        let viewport = f32::from(window.viewport_size().width);
                        let local = f32::from(position.x);
                        drag_app.update(cx, |this, cx| {
                            let start_x = viewport - this.plugins.dock.width - 6. + local;
                            this.plugins.dock.drag = Some((start_x, this.plugins.dock.width));
                            cx.notify();
                        });
                        cx.new(|_| DetailResize)
                    });
                h_flex()
                    .h_full()
                    .w_full()
                    .child(conversation)
                    .child(band)
                    .child(
                        div()
                            .w(px(detail_width))
                            .h_full()
                            .flex_shrink_0()
                            .child(panel),
                    )
                    .into_any_element()
            }
            None => conversation.into_any_element(),
        };
        let body = if self.plugins.collapsed() {
            h_flex()
                .size_full()
                .child(
                    div().w(px(38.)).h_full().flex_shrink_0().child(
                        self.sidebar_view
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    ),
                )
                .child(conversation_area)
                .into_any_element()
        } else {
            body.child(resizable_panel().child(conversation_area))
                .into_any_element()
        };

        let drag_app = app_entity.clone();
        let mut main_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .on_drag_move(move |event: &DragMoveEvent<DetailResize>, window, cx| {
                let x = f32::from(event.event.position.x);
                let viewport = f32::from(window.viewport_size().width);
                drag_app.update(cx, |this, cx| {
                    let Some((start_x, start_width)) = this.plugins.dock.drag else {
                        return;
                    };
                    let content = (viewport - this.sidebar_width - 6.).max(320.);
                    let max = (content * 0.5).max(260.);
                    let width = (start_width + (start_x - x)).clamp(260., max);
                    if (width - this.plugins.dock.width).abs() > 0.5 {
                        this.plugins.dock.width = width;
                        cx.notify();
                    }
                });
            })
            .child(body);
        if let Some(panel) = overlay {
            main_area = main_area.child(panel);
        }
        let view = v_flex()
            .relative()
            .size_full()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .child(title_bar)
            .child(main_area);
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.record_render(render_started.elapsed());
            });
        }
        view.capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
            if event.keystroke.key != "shift" {
                this.double_shift.cancel();
            }
        }))
        .capture_any_mouse_down(cx.listener(|this, _, _, _| {
            this.double_shift.cancel();
        }))
        .on_modifiers_changed(
            cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                if !this.terminal_focused(window, cx)
                    && this.double_shift.update(event.modifiers, Instant::now())
                {
                    this.open_file_search(window, cx);
                }
            }),
        )
        .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
            this.terminal_command(false, window, cx)
        }))
        .on_action(
            cx.listener(|this, _: &NewTerminal, window, cx| {
                this.terminal_command(true, window, cx)
            }),
        )
        .on_action(
            cx.listener(|this, _: &LeaveTerminal, window, cx| this.leave_terminal(window, cx)),
        )
        .on_action(
            cx.listener(|this, _: &NextTerminal, window, cx| {
                this.cycle_terminal(false, window, cx)
            }),
        )
        .on_action(cx.listener(|this, _: &PreviousTerminal, window, cx| {
            this.cycle_terminal(true, window, cx)
        }))
        .on_action(cx.listener(|this, _: &RecentSessions, window, cx| {
            this.open_recent_sessions(window, cx);
        }))
        .on_action(cx.listener(|this, _: &OpenSessionSearch, window, cx| {
            this.open_search_dialog(window, cx);
        }))
        .on_action(cx.listener(|this, _: &OpenSettingsShortcut, window, cx| {
            this.open_settings(window, cx);
        }))
        .on_action(cx.listener(|this, _: &ToggleGitNavigation, _, cx| {
            this.toggle_git_navigation(cx);
        }))
        .into_any_element()
    }
}
