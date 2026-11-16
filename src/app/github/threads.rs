use super::*;
use crate::github::threads::{CommentState, Thread};

pub(super) fn excerpt(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut text: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

pub(in crate::app) fn thread_body(thread: &Thread) -> impl IntoElement {
    let mut body = v_flex()
        .gap_1()
        .child(
            div()
                .text_size(ui_text_size(12.))
                .child(thread.path.clone()),
        )
        .child(div().text_size(ui_text_size(11.)).child(thread.location()))
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().muted))
                .child(thread.remote_status()),
        );
    for (index, comment) in thread
        .comments
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || *i >= thread.comments.len().saturating_sub(2))
    {
        let url = comment.url.clone();
        let author = comment
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("?");
        let mut row = v_flex()
            .gap_1()
            .p_1()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .child(format!("{author} · {}", comment.updated_at)),
            )
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .child(excerpt(&comment.body, 4000)),
            );
        if comment.state == CommentState::Pending {
            row = row.child(label("github.thread_pending"));
        }
        if crate::github::web_url(&url).is_some() {
            row = row.child(
                Button::new(("thread-comment-open", index))
                    .small()
                    .ghost()
                    .label(label("github.details"))
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }
        body = body.child(row);
    }
    if let Some(comment) = thread.comments.first() {
        body = body
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "github.thread_evidence",
                        &[(
                            "commit",
                            comment
                                .original_commit
                                .as_ref()
                                .map(|c| c.oid.clone())
                                .unwrap_or_else(|| "?".into()),
                        )],
                    )),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .child(excerpt(&comment.diff_hunk, 2000)),
            );
    }
    body.child(
        div()
            .text_size(ui_text_size(11.))
            .text_color(rgb(palette().muted))
            .child(conduit::i18n::format(
                "github.thread_excerpt",
                &[("count", thread.total_comments.to_string())],
            )),
    )
}

impl PullRequestPanel {
    pub(super) fn sync_threads(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let Some(snapshot) = self.snapshot.as_ref().and_then(|pr| pr.threads.as_ref()) else {
            return;
        };
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let result = host.update(cx, |app, cx| {
            if !app.writable
                || app.closing
                || !app
                    .store
                    .workspaces
                    .iter()
                    .any(|w| w.path == self.workspace)
            {
                return Err(label("review.cannot_attach").to_owned());
            }
            let result = app.store.reviews.sync_remote(&self.workspace, snapshot)?;
            app.persist();
            cx.notify();
            Ok(result)
        });
        match result {
            Ok(result) => {
                self.error = None;
                self.sync_result = Some(conduit::i18n::format(
                    "github.threads_synced",
                    &[
                        ("added", result.added.to_string()),
                        ("updated", result.updated.to_string()),
                        ("skipped", result.skipped.to_string()),
                    ],
                ));
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn attach_thread(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref().and_then(|pr| pr.threads.as_ref()) else {
            return;
        };
        let Some(thread) = snapshot.threads.iter().find(|t| t.id == id) else {
            return;
        };
        match thread.feedback(&snapshot.pr, &snapshot.head) {
            Ok(text) => self.attach(text, window, cx),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    pub(super) fn review_threads(
        &self,
        writable: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut body = v_flex().gap_2().child(label("github.threads")).child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().muted))
                .child(label("github.threads_hint")),
        );
        let Some(pr) = &self.snapshot else {
            return body;
        };
        if let Some(error) = &pr.threads_error {
            body = body.child(
                div()
                    .text_color(rgb(palette().warning_text))
                    .child(format!("{}: {error}", label("github.threads_failed"))),
            );
        }
        if let Some(error) = self
            .host
            .upgrade()
            .and_then(|host| host.read(cx).persistence.error.clone())
        {
            body = body.child(div().text_color(rgb(palette().danger_text)).child(
                conduit::i18n::format("review.save_failed", &[("error", error)]),
            ));
        }
        let Some(snapshot) = &pr.threads else {
            return body;
        };
        body = body.child(
            Button::new("github-sync-threads")
                .small()
                .label(label("github.threads_sync"))
                .disabled(!writable || self.loading || snapshot.threads.is_empty())
                .on_click(cx.listener(|this, _, _, cx| this.sync_threads(cx))),
        );
        if let Some(result) = &self.sync_result {
            body = body.child(result.clone());
        }
        if snapshot.limited {
            body = body.child(label("github.threads_limited"));
        }
        if snapshot.threads.is_empty() {
            body = body.child(label("github.threads_empty"));
        }
        let mut threads: Vec<_> = snapshot.threads.iter().collect();
        threads.sort_by_key(|thread| thread.is_resolved);
        for thread in threads.into_iter().take(self.shown) {
            let id = thread.id.clone();
            let resolve_thread = thread.clone();
            let reply_thread = thread.clone();
            let can_write = writable
                && !self.loading
                && pr.state == "OPEN"
                && !thread
                    .comments
                    .iter()
                    .any(|comment| comment.state == CommentState::Pending);
            let can_resolve = !thread.is_resolved && can_write;
            body = body.child(
                v_flex()
                    .id(SharedString::from(format!("pr-thread-{id}")))
                    .p_2()
                    .gap_1()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .rounded_md()
                    .child({
                        let expanded = self.expanded_threads.contains(&thread.id);
                        let key = thread.id.clone();
                        let mut summary = v_flex().gap_2();
                        if expanded {
                            summary = summary.child(thread_body(thread));
                        } else {
                            summary = summary.child(thread.path.clone()).child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "{} · {}",
                                        thread.location(),
                                        thread.remote_status()
                                    )),
                            );
                            if let Some(comment) = thread.comments.last() {
                                summary = summary.child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .child(excerpt(&comment.body, 350)),
                                );
                            }
                        }
                        summary.child(
                            Button::new("thread-expand")
                                .small()
                                .ghost()
                                .label(label(if expanded {
                                    "github.collapse_context"
                                } else {
                                    "github.expand_context"
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.expanded_threads.remove(&key) {
                                        this.expanded_threads.insert(key.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("thread-attach")
                                    .small()
                                    .ghost()
                                    .label(label("github.attach"))
                                    .disabled(!writable || self.loading)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.attach_thread(&id, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("thread-resolve")
                                    .small()
                                    .ghost()
                                    .label(label("github.resolve_thread"))
                                    .disabled(!can_resolve)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_write(
                                            crate::github::write::Action::Resolve(
                                                resolve_thread.clone(),
                                            ),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new("thread-reply")
                                    .small()
                                    .ghost()
                                    .label(label("github.reply_thread"))
                                    .disabled(!can_write)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_write(
                                            crate::github::write::Action::Reply {
                                                thread: reply_thread.clone(),
                                                body: String::new(),
                                            },
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    ),
            );
        }
        if snapshot.threads.len() > self.shown {
            body = body.child(
                Button::new("github-threads-more")
                    .small()
                    .label(label("github.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shown += 20;
                        cx.notify();
                    })),
            );
        }
        body
    }
}
