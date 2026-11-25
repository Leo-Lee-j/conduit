//! Contextual GitHub write form. Closing an unsent draft requires an explicit discard.
use super::*;

impl Render for GithubCommentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self
            .editor
            .as_ref()
            .map(|editor| editor.read(cx).value().to_string());
        let bytes = text.as_ref().map_or(0, |text| text.len());
        let invalid = text.as_ref().is_some_and(|text| {
            text.trim().is_empty() || bytes > crate::github::write::MAX_BODY_BYTES
        });
        let writable = self.writable(cx);
        let resolve = matches!(self.action, crate::github::write::Action::Resolve(_));
        let mut content = v_flex().gap_3().when(!self.inline, |content| {
            content.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(palette().surface_strong))
                    .child(div().text_size(ui_text_size(13.)).child(format!(
                        "{} · {}",
                        self.expected.forge.request_name(self.expected.number),
                        self.expected.title
                    )))
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(self.expected.url.clone()),
                    ),
            )
        });
        if let crate::github::write::Action::Reply { thread, .. }
        | crate::github::write::Action::Resolve(thread) = &self.action
        {
            let mut context = v_flex()
                .gap_1()
                .p_3()
                .border_1()
                .border_color(rgb(palette().border))
                .rounded_md()
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(label("github.discussion_context")),
                )
                .child(thread.path.clone())
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(thread.location()),
                );
            if let Some(comment) = thread.comments.last() {
                context = context.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .child(threads::excerpt(&comment.body, 700)),
                );
                let url = comment.url.clone();
                if crate::github::web_url(&url).is_some() {
                    context = context.child(
                        Button::new("github-context-open")
                            .small()
                            .ghost()
                            .label(label("github.details"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
            }
            content = content.child(context);
        }
        if self.finished {
            content = content.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(palette().surface_strong))
                    .child(
                        div()
                            .text_color(rgb(palette().activity_completed))
                            .child(label("github.write_done")),
                    )
                    .child(label("github.write_success")),
            );
        } else if !self.confirm_discard
            && let Some(editor) = &self.editor
        {
            content = content.child(
                v_flex()
                    .gap_2()
                    .child(label(
                        if matches!(self.action, crate::github::write::Action::Reply { .. }) {
                            "github.reply_label"
                        } else {
                            "github.comment_label"
                        },
                    ))
                    .child(
                        Textarea::new(editor)
                            .h(px(if self.inline { 90. } else { 180. }))
                            .disabled(self.busy || self.uncertain),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(label("github.markdown_hint")),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(
                                        if bytes > crate::github::write::MAX_BODY_BYTES {
                                            palette().danger_text
                                        } else {
                                            palette().muted
                                        },
                                    ))
                                    .child(conduit::i18n::format(
                                        "github.body_bytes",
                                        &[
                                            ("count", bytes.to_string()),
                                            (
                                                "limit",
                                                crate::github::write::MAX_BODY_BYTES.to_string(),
                                            ),
                                        ],
                                    )),
                            ),
                    ),
            );
        } else if resolve {
            content = content.child(label("github.resolve_hint"));
        }
        if let Some(error) = &self.error {
            content = content.child(
                v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(palette().danger_border))
                    .bg(rgb(palette().danger_bg))
                    .child(div().text_color(rgb(palette().danger_text)).child(label(
                        if self.uncertain {
                            "github.write_unknown"
                        } else {
                            "github.write_failed"
                        },
                    )))
                    .child(div().text_size(ui_text_size(11.)).child(error.clone())),
            );
        }
        if self.uncertain {
            content = content.child(label("github.write_uncertain"));
            if let Some(text) = text.clone() {
                content = content.child(
                    Button::new("github-copy-draft")
                        .small()
                        .label(label("github.copy_draft"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                        }),
                );
            }
        } else if !writable && !self.finished {
            content = content.child(
                div()
                    .text_color(rgb(palette().warning_text))
                    .child(label("github.write_unavailable")),
            );
        }
        if self.confirm_discard && !self.finished {
            content = content.child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(palette().warning_border))
                    .bg(rgb(palette().warning_bg))
                    .child(label("github.discard_hint"))
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("github-keep-editing")
                                    .small()
                                    .label(label("github.keep_editing"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_discard = false;
                                        if let Some(editor) = &this.editor {
                                            editor.read(cx).focus_handle(cx).focus(window, cx);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("github-discard")
                                    .small()
                                    .ghost()
                                    .label(label("github.discard"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_form(window, cx)
                                    })),
                            ),
                    ),
            );
        }
        let url = self.expected.url.clone();
        if self.inline {
            content = content.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(label("github.write_hint")),
            );
        }
        let mut actions = h_flex()
            .gap_2()
            .when(!self.inline, |actions| actions.flex_wrap())
            .justify_end()
            .items_center()
            .child(
                Button::new("github-write-open")
                    .small()
                    .ghost()
                    .when(self.inline, |button| {
                        button
                            .icon(FullIconName::ExternalLink)
                            .compact()
                            .size(px(28.))
                            .accessibility_label(label("github.open_pr"))
                    })
                    .when(!self.inline, |button| button.label(label("github.open_pr")))
                    .tooltip(label("github.open_pr"))
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(
                Button::new("github-comment-cancel")
                    .small()
                    .when(self.inline, |button| button.h(px(28.)))
                    .when(!self.inline, |button| button.ghost())
                    .label(label(if self.finished {
                        "github.back_to_pr"
                    } else {
                        "github.cancel"
                    }))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.can_close(cx) {
                            this.close_form(window, cx);
                        }
                    })),
            );
        if !self.finished {
            actions = actions.child(
                Button::new("github-comment-submit")
                    .small()
                    .primary()
                    .when(self.inline, |button| button.h(px(28.)))
                    .label(label(if self.busy {
                        "github.sending"
                    } else if resolve {
                        "github.resolve_thread"
                    } else {
                        "github.send_comment"
                    }))
                    .disabled(self.busy || self.uncertain || !writable || invalid)
                    .on_click(cx.listener(|this, _, _, cx| this.submit(cx))),
            );
        }
        v_flex()
            .when(!self.inline, |panel| panel.gap_3())
            .w_full()
            .min_h_0()
            .child(
                content
                    .id("github-comment-panel")
                    .when(self.inline, |content| {
                        content
                            .p_3()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                    })
                    .max_h(px(if self.inline {
                        170.
                    } else {
                        (f32::from(window.viewport_size().height) - 280.).max(100.)
                    }))
                    .overflow_y_scroll(),
            )
            .child(if self.inline {
                document_footer()
                    .id("github-discussion-footer")
                    .debug_selector(|| "github-discussion-footer".into())
                    .child(actions.w_full())
                    .into_any_element()
            } else {
                v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(label("github.write_hint")),
                    )
                    .child(actions)
                    .into_any_element()
            })
    }
}

impl GithubCommentPanel {
    fn close_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.inline {
            let url = self.expected.url.clone();
            let _ = self.parent.update(cx, |parent, cx| {
                parent.details.composers.remove(&url);
                cx.notify();
            });
        } else {
            window.close_dialog(cx);
        }
    }
}
