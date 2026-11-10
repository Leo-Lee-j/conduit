//! Saved feedback remains reachable after a file is renamed, committed or removed.
use super::*;
mod remote;

struct ReviewInbox {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    error: Option<String>,
    shown: usize,
    reports_shown: usize,
    remote_shown: usize,
    _subscription: Subscription,
}

impl ConduitApp {
    pub(in crate::app) fn review_inbox_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("review-inbox")
            .small()
            .ghost()
            .compact()
            .icon(FullIconName::MessageSquare)
            .accessibility_label(label("review.inbox"))
            .tooltip(label("review.inbox"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| this.open_review_inbox(window, cx)))
    }

    pub(in crate::app) fn open_review_inbox(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        let host = cx.entity();
        let view = cx.new(|cx| ReviewInbox {
            host: host.downgrade(),
            workspace,
            error: None,
            shown: 30,
            reports_shown: 10,
            remote_shown: 10,
            _subscription: cx.observe(&host, |_, _, cx| cx.notify()),
        });
        window.open_dialog(cx, move |dialog, window, _| {
            let size = window.viewport_size();
            let view = view.clone();
            dialog
                .title(label("review.inbox"))
                .w(px((f32::from(size.width) - 48.).clamp(280., 850.)))
                .content(move |content, _, _| content.child(view.clone()))
        });
    }
}

impl ReviewInbox {
    fn open_source(
        &mut self,
        id: u64,
        created_at: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        match host.update(cx, |app, cx| {
            app.open_ai_review_session(&self.workspace, id, created_at, window, cx)
        }) {
            Ok(()) => {
                window.close_dialog(cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn remove_report(&mut self, key: &str, cx: &mut Context<Self>) {
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
                .ai
                .iter()
                .position(|r| r.key == key && r.target.workspace == self.workspace)
            else {
                return;
            };
            if app.session_busy(app.store.reviews.ai[index].session) {
                return;
            }
            app.store.reviews.ai.remove(index);
            app.persist();
            cx.notify();
        });
    }

    fn attach(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let result = host.update(cx, |app, cx| {
            let comment = app
                .store
                .reviews
                .comments
                .iter()
                .find(|c| c.id == id && c.target.workspace == self.workspace)
                .ok_or_else(|| label("review.target_missing").to_owned())?;
            let feedback = app.store.reviews.feedback(&comment.target, &[id], "")?;
            // Use the selected conversation only when it belongs to this workspace.
            let session = (app.selected_workspace_path().as_ref() == Some(&self.workspace))
                .then_some(app.selected_session)
                .flatten();
            app.append_review_draft(&self.workspace, session, None, &feedback, window, cx)
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

    fn set_status(&mut self, id: u64, status: Status, cx: &mut Context<Self>) {
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |app, cx| {
                if !app.writable || app.closing {
                    return;
                }
                if let Some(note) = app
                    .store
                    .reviews
                    .comments
                    .iter_mut()
                    .find(|c| c.id == id && c.target.workspace == self.workspace)
                {
                    note.status = status;
                    app.persist();
                    cx.notify();
                }
            });
        }
    }
}

impl Render for ReviewInbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let notes: Vec<_> = app
            .store
            .reviews
            .comments
            .iter()
            .filter(|c| c.target.workspace == self.workspace)
            .rev()
            .collect();
        let mut body = v_flex()
            .id("review-inbox-body")
            .w_full()
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 200.).max(160.)
            ))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .child(self.workspace.display().to_string()),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(label("review.inbox_hint")),
            );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(error) = &app.persistence.error {
            body = body.child(div().text_color(rgb(palette().danger_text)).child(
                conduit::i18n::format("review.save_failed", &[("error", error.clone())]),
            ));
        }
        let reports: Vec<_> = app
            .store
            .reviews
            .ai
            .iter()
            .filter(|r| r.target.workspace == self.workspace)
            .rev()
            .collect();
        if !reports.is_empty() {
            body = body.child(
                div()
                    .text_size(ui_text_size(12.))
                    .child(label("review.ai_reports")),
            );
        }
        for report in reports.iter().take(self.reports_shown) {
            let key = report.key.clone();
            let id = report.session;
            let created_at = report.session_created_at;
            body = body.child(
                v_flex()
                    .id(SharedString::from(format!("ai-review-{key}")))
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .rounded_md()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(format!("{} · {}", report.target.path, report.target.source)),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .child(report.state.label()),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!("ai-review-source-{key}")))
                                    .small()
                                    .ghost()
                                    .label(label("review.ai_open_source"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_source(id, created_at, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("ai-review-remove-{key}")))
                                    .small()
                                    .ghost()
                                    .label(label("review.ai_remove"))
                                    .tooltip(label("review.ai_remove_hint"))
                                    .disabled(!writable || app.session_busy(id))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_report(&key, cx)
                                    })),
                            ),
                    ),
            );
        }
        if reports.len() > self.reports_shown {
            body = body.child(
                Button::new("ai-review-more")
                    .small()
                    .label(label("review.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reports_shown += 10;
                        cx.notify();
                    })),
            );
        }
        if notes.is_empty() {
            body = body.child(label("review.no_comments"));
        }
        for note in notes.iter().take(self.shown) {
            let id = note.id;
            let owner = cx.weak_entity();
            body = body.child(
                v_flex()
                    .id(("saved-review", id))
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .rounded_md()
                    .child(div().text_size(ui_text_size(12.)).child(format!(
                        "#{} · {} · {}",
                        id,
                        note.target.path,
                        location(&note.anchor)
                    )))
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(note.target.source.clone()),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(note.display_text()),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(note.anchor.snippet.clone()),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .children(note.ai.as_ref().map(|origin| {
                                let session = origin.session;
                                let created_at = origin.session_created_at;
                                Button::new(("saved-review-source", id))
                                    .small()
                                    .ghost()
                                    .label(label("review.ai_open_source"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_source(session, created_at, window, cx)
                                    }))
                            }))
                            .child(
                                Button::new(("saved-review-status", id))
                                    .small()
                                    .ghost()
                                    .label(note.status.label())
                                    .disabled(!writable)
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for status in Status::ALL {
                                            let owner = owner.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(status.label()).on_click(
                                                    move |_, _, cx| {
                                                        let _ = owner.update(cx, |this, cx| {
                                                            this.set_status(id, status, cx)
                                                        });
                                                    },
                                                ),
                                            );
                                        }
                                        menu
                                    }),
                            )
                            .child(
                                Button::new(("saved-review-attach", id))
                                    .small()
                                    .ghost()
                                    .label(label("review.attach_note"))
                                    .disabled(!writable)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.attach(id, window, cx)
                                    })),
                            ),
                    ),
            );
        }
        if notes.len() > self.shown {
            body = body.child(
                Button::new("saved-review-more")
                    .small()
                    .label(label("review.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shown += 30;
                        cx.notify();
                    })),
            );
        }
        body.child(self.remote_threads(cx)).into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ElementId, Event, SharedString, Status, Target, px, review};
    use crate::app::queue::tests::{fixture, running};
    use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext};

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    fn click(cx: &mut VisualTestContext, id: impl Into<ElementId>) {
        let id = id.into();
        let bounds = cx.update(|window, _| {
            gpui_kit::base::test_support::find(window, &[], &id)
                .expect("review button is rendered")
                .bounds()
        });
        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
        draw(cx);
    }

    #[gpui_kit::test]
    fn review_inbox_removes_only_idle_reports_and_keeps_comment_source_navigation(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = fixture(cx, "review-inbox-ai");
        cx.simulate_resize(gpui_kit::size(px(1000.), px(850.)));
        let (key, sender) = cx.update(|window, cx| app.update(cx, |app, cx| {
            app.persistence = crate::persistence::Persistence::new(root.join("inbox-state.json"));
            let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
            let mut request = review::ai::Review::capture(Target { workspace:root.clone(), path:"a.rs".into(), source:"unstaged".into() }, patch).unwrap();
            request.session = 1;
            request.session_created_at = app.session(1).unwrap().created_at;
            let text = serde_json::json!({"version":1,"review_id":request.key,"fingerprint":request.fingerprint,"findings":[
                {"path":"a.rs","side":"new","start_line":1,"end_line":1,"priority":"P1","title":"Check input","body":"Verify empty input"}
            ]}).to_string();
            let key = request.key.clone();
            app.store.reviews.ai.push(request);
            app.store.reviews.accept_ai(&key, Some("turn-reply".into()), Ok(&text));
            let sender = running(app, 1);
            app.open_review_inbox(window, cx);
            (key, sender)
        }));
        draw(cx);
        let remove = SharedString::from(format!("ai-review-remove-{key}"));
        click(cx, remove.clone());
        cx.update(|_, cx| assert_eq!(app.read(cx).store.reviews.ai.len(), 1));
        sender
            .send(Event::Finished {
                error: None,
                cancelled: false,
            })
            .unwrap();
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        draw(cx);
        click(cx, remove);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(app.store.reviews.ai.is_empty());
            assert_eq!(app.store.reviews.comments.len(), 1);
            assert_eq!(app.store.reviews.comments[0].status, Status::NeedsReview);
        });
        // A saved AI note retains navigation even after its patch record is removed.
        cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
        draw(cx);
        click(cx, ("saved-review-source", 1_u64));
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                assert_eq!(app.selected_session, Some(1));
                app.persistence.flush();
                app.writable = false;
            })
        });
    }
}
