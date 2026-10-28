//! Window layout, sidebar, conversation, composer, and toolbar.

use super::*;

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
        gpui_kit::list(self.scroll.clone(), move |index, _, cx| {
            app.update(cx, |app, cx| app.render_message(index, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full()
        .pt(px(5.))
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
        if self.timeline_session != self.selected_session {
            self.scroll.reset(message_count);
            self.timeline_session = self.selected_session;
        } else {
            let previous_count = self.scroll.item_count();
            if message_count > previous_count {
                self.scroll.splice(
                    previous_count..previous_count,
                    message_count - previous_count,
                );
            } else if message_count < previous_count {
                self.scroll.splice(message_count..previous_count, 0);
            }
        }
        if self.at_bottom {
            self.scroll.scroll_to_end();
        }
        self.scroll
            .set_scroll_handler(cx.listener(|this, event: &ListScrollEvent, _, cx| {
                let at_bottom = !event.is_scrolled;
                let outline = this.outline.active_for_item(event.visible_range.start);
                // The list redraws itself. Rebuild the surrounding UI only when
                // the jump-to-latest control or active outline marker changes.
                let changed = this.at_bottom != at_bottom || this.outline.active != outline;
                this.at_bottom = at_bottom;
                if changed {
                    cx.notify();
                }
            }));
        // Parent changes include streaming content, settings and row expansion.
        self.timeline.update(cx, |_, cx| cx.notify());
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.begin_render(
                    message_count,
                    self.runs.len(),
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
        let mut messages = div()
            .v_flex()
            .id("messages")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .min_w_0()
            .px_2()
            .py_1()
            .gap_5();
        if let Some(session) = &selected {
            if session.message_count == 0 {
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
                                        .child(format!("{} · 已就绪", session.provider.label())),
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
                                .child(format!("在 {} 中开始你的任务", directory)),
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
            messages = messages.child(self.session_heatmap(13));
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
            .and_then(|id| self.runs.get(&id))
            .map(|r| r.approvals.len())
            .unwrap_or(0);
        let approvals = self
            .selected_session
            .and_then(|id| self.runs.get(&id))
            .and_then(|r| r.approvals.front().cloned());
        for (index, approval) in approvals.into_iter().enumerate() {
            let submitted = self
                .selected_session
                .and_then(|id| self.runs.get(&id))
                .is_some_and(|r| r.approvals.is_submitted(&approval.id));
            let id = self.selected_session.unwrap();
            let allow_id = approval.id.clone();
            let deny_id = approval.id;
            let toggle_id = deny_id.clone();
            let expanded = self
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
                                                        .text_color(rgb(palette().highlight_text))
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
                                            this.toggle_approval_detail(id, toggle_id.clone(), cx)
                                        })),
                                ),
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
                                .when(!approval.allow_once, |row| {
                                    row.child(
                                        div()
                                            .flex_1()
                                            .text_size(px(11.))
                                            .text_color(rgb(palette().warning_accent))
                                            .child("CLI 未提供「允许本次」选项，此请求只能拒绝。"),
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
                                        .disabled(self.closing || submitted || !approval.allow_once)
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
        if let Some(message) = &self.message {
            conversation = conversation.child(
                div()
                    .mx_6()
                    .mb_3()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(palette().danger_border))
                    .bg(rgb(palette().danger_bg))
                    .text_sm()
                    .text_color(rgb(palette().danger_text))
                    .child(message.clone()),
            );
        }
        conversation = conversation.child(self.render_composer(selected.as_ref(), window, cx));
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
        conversation = conversation.child(toolbar);
        let (detail_panel, overlay) = self.render_detail_dock(selected.as_ref(), window, cx);
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
            .unwrap_or(248.);
        self.sidebar_width = measured_sidebar;
        let content_width = (viewport_width - measured_sidebar - 6.).max(320.);
        let max_detail_width = (content_width * 0.5).max(260.);
        self.detail_width = self.detail_width.clamp(260., max_detail_width);
        let detail_width = self.detail_width;

        let mut body = h_resizable("body").with_state(&sidebar_state).child(
            resizable_panel()
                .size(px(248.))
                .size_range(px(200.)..px(420.))
                .flex_none()
                .child(sidebar),
        );
        let conversation_area: AnyElement = match detail_panel {
            Some(panel) => {
                let drag_app = app_entity.clone();
                let band = div()
                    .id("detail-resize")
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
                            let start_x = viewport - this.detail_width - 6. + local;
                            this.detail_drag = Some((start_x, this.detail_width));
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
        body = body.child(resizable_panel().child(conversation_area));

        let drag_app = app_entity.clone();
        let mut main_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .on_drag_move(move |event: &DragMoveEvent<DetailResize>, window, cx| {
                let x = f32::from(event.event.position.x);
                let viewport = f32::from(window.viewport_size().width);
                drag_app.update(cx, |this, cx| {
                    let Some((start_x, start_width)) = this.detail_drag else {
                        return;
                    };
                    let content = (viewport - this.sidebar_width - 6.).max(320.);
                    let max = (content * 0.5).max(260.);
                    let width = (start_width + (start_x - x)).clamp(260., max);
                    if (width - this.detail_width).abs() > 0.5 {
                        this.detail_width = width;
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
        view.into_any_element()
    }
}
