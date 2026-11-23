use super::*;

impl ConduitApp {
    pub(in crate::app) fn render_chat_approval(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let run = self.session_control.runs.get(&id)?;
        let approval = run.approvals.front()?.clone();
        if approval.is_user_input() {
            return Some(self.render_user_input(id, &approval, window, cx));
        }
        let approval_count = run.approvals.len();
        let index: usize = 0;
        let submitted = run.approvals.is_submitted(&approval.id);
        let preview = notifications::approval_preview(&approval);
        let detail_copy = approval.detail.clone();
        let origin = self
            .session_control
            .locate(&self.store.workspaces, id)
            .map(|(index, _)| display_path(&self.store.workspaces[index].path))
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
            conduit::i18n::format(
                "shell.item",
                &[
                    ("arg0", "1".to_string()),
                    ("arg1", format!("{}", approval_count)),
                ],
            )
        } else {
            String::new()
        };
        Some(
            div()
                .id(("chat-approval", id))
                .debug_selector(move || format!("chat-approval-{id}"))
                .w_full()
                .flex_shrink_0()
                .px_3()
                .mb_3()
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .when(!self.store.ui.full_width_content, |card| {
                            card.max_w(px(840.)).mx_auto()
                        })
                        .px_3()
                        .py_2()
                        .gap_2()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(palette().warning_border))
                        .bg(rgb(palette().warning_bg))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::TriangleAlert)
                                        .small()
                                        .text_color(rgb(palette().warning_text)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(ui_text_size(13.))
                                        .font_semibold()
                                        .text_color(rgb(palette().highlight_text))
                                        .child(first_line(&approval.title)),
                                )
                                .when(!queue_label.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(palette().warning_muted))
                                            .child(queue_label),
                                    )
                                })
                                .child(
                                    Button::new(("approval-toggle", index))
                                        .label(if expanded {
                                            conduit::i18n::text_static("chat.collapse")
                                        } else {
                                            conduit::i18n::text_static("shell.show_details")
                                        })
                                        .small()
                                        .ghost()
                                        .flex_shrink_0()
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_approval_detail(id, toggle_id.clone(), cx)
                                        })),
                                ),
                        )
                        .when_some(preview, |column, preview| {
                            column.child(
                                div()
                                    .id(("approval-summary", index))
                                    .min_w_0()
                                    .max_h(px(66.))
                                    .overflow_y_scroll()
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .bg(rgb(palette().warning_code_bg))
                                    .text_size(ui_text_size(12.))
                                    .line_height(relative(1.35))
                                    .text_color(rgb(palette().warning_code_text))
                                    .child(preview),
                            )
                        })
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
                        .when(!origin.is_empty(), |column| {
                            column.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette().warning_muted))
                                    .child(conduit::i18n::format(
                                        "shell.workspace",
                                        &[("origin", origin)],
                                    )),
                            )
                        })
                        .child(
                            h_flex()
                                .items_center()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new(("approval-copy", index))
                                        .label(conduit::i18n::text_static(
                                            "shell.copy_full_request",
                                        ))
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
                                            .text_size(ui_text_size(11.))
                                            .text_color(rgb(palette().warning_accent))
                                            .child(conduit::i18n::text_static(
                                                "shell.the_cli_did_not_offer_allow_once",
                                            )),
                                    )
                                })
                                .when(approval.allow_once, |row| row.child(div().flex_1()))
                                .child(
                                    Button::new(("deny", index))
                                        .label(conduit::i18n::text_static("notifications.deny"))
                                        .small()
                                        .ghost()
                                        .disabled(self.closing || submitted)
                                        .when(!self.closing && !submitted, |button| {
                                            button.cursor_pointer()
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.submit_approval(id, &deny_id, false, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("allow", index))
                                        .label(conduit::i18n::text_static(
                                            "notifications.allow_once",
                                        ))
                                        .small()
                                        .primary()
                                        .disabled(self.closing || submitted || !approval.allow_once)
                                        .when(
                                            !self.closing && !submitted && approval.allow_once,
                                            |button| button.cursor_pointer(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.submit_approval(id, &allow_id, true, cx);
                                        })),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}
