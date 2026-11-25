//! Shared transcript frame for every chat column.
use super::*;

impl ConduitApp {
    pub(in crate::app) fn render_chat_messages(
        &mut self,
        selected: Option<&SessionSummary>,
        timeline: Entity<ChatTimeline>,
        source: TimelineSource,
        focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        source.refresh_outline(self);
        let workspace = selected
            .and_then(|s| self.session_control.locate(&self.store.workspaces, s.id))
            .map(|(w, _)| w)
            .or(self.selected_workspace);
        let has_workspace = workspace.is_some();
        let directory = workspace
            .map(|i| self.store.workspaces[i].name())
            .unwrap_or_else(|| conduit::i18n::text_static("git_ui.no_workspace_selected").into());
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
        if let Some(session) = selected {
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
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .child(if error.is_some() {
                                conduit::i18n::text_static("shell.failed_to_load_session_messages")
                            } else {
                                conduit::i18n::text_static("shell.loading_session")
                            }),
                    );
                if let Some(error) = error {
                    loading =
                        loading
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(error),
                            )
                            .child(
                                Button::new("retry-transcript")
                                    .label(conduit::i18n::text_static("shell.retry_loading"))
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
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().faint))
                                        .child(conduit::i18n::format(
                                            "shell.ready_for_your_input",
                                            &[("arg0", session.provider.label().to_string())],
                                        )),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(30.))
                                .font_bold()
                                .text_color(rgb(palette().text))
                                .child(conduit::i18n::text_static(
                                    "shell.what_would_you_like_to_work_on",
                                )),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(14.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::format(
                                    "shell.start_your_task_in_cli_sign_in",
                                    &[("arg0", directory.to_string())],
                                )),
                        ),
                );
            }
            if !unloaded && session.message_count > 0 {
                messages = div()
                    .id("messages")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .track_focus(&focus)
                    .on_mouse_up(MouseButton::Left, {
                        let focus = focus.clone();
                        move |_, window, cx| {
                            // The composer holds keyboard focus, so a finished chat
                            // selection would never see Ctrl+C. Move focus onto the
                            // conversation, inside the window root's key context.
                            if !TextSelection::selected_text(window, cx).trim().is_empty() {
                                focus.focus(window, cx);
                            }
                        }
                    })
                    .child(
                        timeline
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    )
                    .when(!source.following(self), |messages| {
                        messages.child(
                            div()
                                .absolute()
                                .bottom(px(12.))
                                .right(px(24.))
                                .child(self.timeline_latest_button(source.clone(), cx)),
                        )
                    });
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
                            .text_size(ui_text_size(30.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(conduit::i18n::text_static(
                                "shell.what_would_you_like_to_work_on",
                            )),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::format(
                                "shell.the_session_will_run_in_enter_a",
                                &[("directory", directory.to_string())],
                            )),
                    ),
            );
        } else {
            messages = messages.child(self.session_heatmap(13, cx));
        }
        // Both columns use the same rail, outside the scrolling transcript and
        // balanced on the other side so it stays aligned with the composer.
        let mut message_area = h_flex().flex_1().min_h_0().min_w_0().items_stretch();
        if source.with_outline(self, |outline| !outline.entries.is_empty()) {
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
                    .child(self.message_outline(source, cx))
                    .child(messages)
                    .child(div().w(px(32.)).flex_shrink_0()),
            );
        } else {
            message_area = message_area.child(messages);
        }
        message_area.into_any_element()
    }

    fn timeline_latest_button(
        &self,
        source: TimelineSource,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Button::new("scroll-to-latest")
            .debug_selector(|| "scroll-to-latest".into())
            .ghost()
            .tooltip(conduit::i18n::text_static(
                "composer.jump_to_latest_message",
            ))
            .accessibility_label(conduit::i18n::text_static(
                "composer.jump_to_latest_message",
            ))
            .size(px(28.))
            .p_0()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| source.jump_to_end(this, cx)))
            .child(
                Icon::new(IconName::ArrowDown)
                    .size(px(14.))
                    .text_color(rgb(palette().muted)),
            )
    }
}
