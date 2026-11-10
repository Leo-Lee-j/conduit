use super::*;

impl ReviewInbox {
    fn remote_status(&mut self, key: &str, status: Option<Status>, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        host.update(cx, |app, cx| {
            if !app.writable || app.closing {
                return;
            }
            let Some(index) = app
                .store
                .reviews
                .remote
                .iter()
                .position(|r| r.key == key && r.workspace == self.workspace)
            else {
                return;
            };
            if let Some(status) = status {
                app.store.reviews.remote[index].set_status(status);
            } else {
                app.store.reviews.remote.remove(index);
            }
            app.persist();
            cx.notify();
        });
    }

    fn attach_remote(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let result = host.update(cx, |app, cx| {
            let saved = app
                .store
                .reviews
                .remote
                .iter()
                .find(|r| r.key == key && r.workspace == self.workspace)
                .ok_or_else(|| label("review.target_missing").to_owned())?;
            let text = saved.feedback()?;
            let session = (app.selected_workspace_path().as_ref() == Some(&self.workspace))
                .then_some(app.selected_session)
                .flatten();
            app.append_review_draft(&self.workspace, session, None, &text, window, cx)
        });
        match result {
            Ok(()) => {
                window.close_dialog(cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    pub(super) fn remote_threads(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex().gap_2();
        let Some(host) = self.host.upgrade() else {
            return body;
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let saved: Vec<_> = app
            .store
            .reviews
            .remote
            .iter()
            .filter(|r| r.workspace == self.workspace)
            .rev()
            .collect();
        if saved.is_empty() {
            return body;
        }
        body = body.child(label("github.saved_threads")).child(
            div()
                .text_size(ui_text_size(11.))
                .child(label("github.threads_hint")),
        );
        for saved in saved.iter().take(self.remote_shown) {
            let key = saved.key.clone();
            let remove = key.clone();
            let attach = key.clone();
            let owner = cx.weak_entity();
            let mut card = v_flex()
                .id(SharedString::from(format!("saved-thread-{key}")))
                .p_2()
                .gap_1()
                .border_1()
                .border_color(rgb(palette().border))
                .rounded_md()
                .child(div().text_size(ui_text_size(11.)).child(saved.pr.clone()))
                .child(div().text_size(ui_text_size(11.)).child(format!(
                    "{} · {}",
                    message_timestamp(saved.fetched_at),
                    saved.head
                )))
                .child(crate::app::github::thread_body(&saved.thread));
            if saved.changed() {
                card = card.child(
                    div()
                        .text_color(rgb(palette().warning_text))
                        .child(label("github.thread_updated")),
                );
            }
            card = card.child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("remote-status")
                            .small()
                            .ghost()
                            .label(conduit::i18n::format(
                                "github.thread_local_status",
                                &[("status", saved.status.label().into())],
                            ))
                            .disabled(!writable)
                            .dropdown_menu(move |mut menu, _, _| {
                                for status in Status::ALL {
                                    let key = key.clone();
                                    let owner = owner.clone();
                                    menu = menu.item(PopupMenuItem::new(status.label()).on_click(
                                        move |_, _, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.remote_status(&key, Some(status), cx)
                                            });
                                        },
                                    ));
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("remote-attach")
                            .small()
                            .ghost()
                            .label(label("github.attach"))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.attach_remote(&attach, window, cx)
                            })),
                    )
                    .child(
                        Button::new("remote-remove")
                            .small()
                            .ghost()
                            .label(label("github.thread_remove"))
                            .tooltip(label("github.thread_remove_hint"))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remote_status(&remove, None, cx)
                            })),
                    ),
            );
            body = body.child(card);
        }
        if saved.len() > self.remote_shown {
            body = body.child(
                Button::new("saved-threads-more")
                    .small()
                    .label(label("review.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.remote_shown += 10;
                        cx.notify();
                    })),
            );
        }
        body
    }
}
