//! A workspace-bound PR/CI snapshot. Reading or attaching never publishes comments.
use super::*;
mod checks;
mod threads;
pub(in crate::app) use threads::thread_body;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

struct PullRequestPanel {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    snapshot: Option<crate::github::PullRequest>,
    error: Option<String>,
    sync_result: Option<String>,
    loading: bool,
    shown: usize,
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
    _subscription: Option<Subscription>,
}

impl ConduitApp {
    pub(super) fn github_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("github-pr")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::GitPullRequest)
            .tooltip(label("github.title"))
            .accessibility_label(label("github.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                let Some(workspace) = this.selected_workspace_path() else {
                    return;
                };
                let host = cx.weak_entity();
                let panel = cx.new(|cx| {
                    let subscription = host
                        .upgrade()
                        .map(|host| cx.observe(&host, |_, _, cx| cx.notify()));
                    let mut panel = PullRequestPanel {
                        host,
                        workspace,
                        snapshot: None,
                        error: None,
                        sync_result: None,
                        loading: false,
                        shown: 20,
                        cancel: Arc::new(AtomicBool::new(false)),
                        _task: Task::ready(()),
                        _subscription: subscription,
                    };
                    panel.reload(cx);
                    panel
                });
                window.open_dialog(cx, move |dialog, window, _| {
                    let panel = panel.clone();
                    dialog
                        .title(label("github.title"))
                        .w(px(
                            (f32::from(window.viewport_size().width) - 48.).clamp(280., 1000.)
                        ))
                        .content(move |content, _, _| content.child(panel.clone()))
                });
            }))
    }
}

impl PullRequestPanel {
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.loading = true;
        self.error = None;
        self.sync_result = None;
        self.snapshot = None;
        let cancel = self.cancel.clone();
        let workspace = self.workspace.clone();
        self._task = cx.spawn(async move |this, cx| {
            let ticket = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::github::load(&workspace, &ticket).map_err(|e| format!("{e:#}"))
                })
                .await;
            if cancel.load(Ordering::Acquire) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(snapshot) => this.snapshot = Some(snapshot),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn attach(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        if text.len() > crate::review::MAX_FEEDBACK_BYTES {
            self.error = Some(label("github.output_limit").into());
            cx.notify();
            return;
        }
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let result = host.update(cx, |app, cx| {
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
}

impl Drop for PullRequestPanel {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Render for PullRequestPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writable = self
            .host
            .upgrade()
            .is_some_and(|host| host.read(cx).writable && !host.read(cx).closing);
        let mut body = v_flex()
            .id("github-pr-panel")
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 180.).max(160.)
            ))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .child(self.workspace.display().to_string()),
            )
            .child(label("github.snapshot_hint"))
            .child(
                Button::new("github-refresh")
                    .small()
                    .label(label(if self.loading {
                        "github.loading"
                    } else {
                        "github.refresh"
                    }))
                    .disabled(self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
            );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(pr) = &self.snapshot {
            let url = pr.url.clone();
            body = body
                .child(
                    div()
                        .text_size(ui_text_size(14.))
                        .child(format!("#{} · {}", pr.number, pr.title)),
                )
                .child(format!(
                    "{} → {} · {} · {}",
                    pr.branch, pr.base, pr.state, pr.decision
                ))
                .child(div().text_size(ui_text_size(11.)).child(format!(
                    "{} · {}",
                    message_timestamp(pr.fetched_at),
                    pr.head
                )))
                .child(
                    Button::new("github-open")
                        .small()
                        .ghost()
                        .label(label("github.open_pr"))
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            if pr.head != pr.local_head {
                body = body.child(
                    div()
                        .text_color(rgb(palette().warning_text))
                        .child(label("github.head_mismatch")),
                );
            }
            if pr.limited {
                body = body.child(label("github.limited"));
            }
            body = body.child(self.review_threads(writable, cx));
            body = body.child(label("github.checks"));
            if pr.checks.is_empty() {
                body = body.child(label("github.no_checks"));
            }
            for (index, check) in pr.checks.iter().enumerate().take(self.shown) {
                let feedback = pr.feedback(index).unwrap_or_default();
                let mut row = h_flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(format!("{} · {}", check.name, check.state)),
                    )
                    .child(
                        Button::new(("github-check-attach", index))
                            .small()
                            .ghost()
                            .label(label("github.attach"))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.attach(feedback.clone(), window, cx)
                            })),
                    );
                if let Some(source) = pr.check_source(index) {
                    let existing = self.host.upgrade().and_then(|host| {
                        host.read(cx)
                            .store
                            .workflow
                            .check_task_id(&self.workspace, &source)
                    });
                    if existing.is_some() || check.needs_attention() {
                        row = row.child(
                            Button::new(("github-check-task", index))
                                .debug_selector(move || format!("github-check-task-{index}"))
                                .small()
                                .ghost()
                                .label(label(if existing.is_some() {
                                    "github.open_check_task"
                                } else {
                                    "github.create_check_task"
                                }))
                                .disabled(self.loading || (!writable && existing.is_none()))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_check_task(&source, window, cx)
                                })),
                        );
                    }
                }
                if let Some(url) = &check.url {
                    let url = url.clone();
                    row = row.child(
                        Button::new(("github-check-open", index))
                            .small()
                            .ghost()
                            .label(label("github.details"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                body = body.child(row);
            }
            body = body.child(label("github.comments"));
            for (index, comment) in pr.comments.iter().enumerate().take(self.shown) {
                let text = conduit::i18n::format(
                    "github.comment_feedback",
                    &[
                        ("url", pr.url.clone()),
                        ("head", pr.head.clone()),
                        ("author", comment.author.clone()),
                        (
                            "body",
                            comment
                                .body
                                .lines()
                                .map(|s| format!("    {s}"))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                    ],
                );
                let mut row = v_flex()
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .rounded_md()
                    .child(comment.author.clone())
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(comment.body.clone()),
                    )
                    .child(
                        Button::new(("github-comment-attach", index))
                            .small()
                            .ghost()
                            .label(label("github.attach"))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.attach(text.clone(), window, cx)
                            })),
                    );
                if let Some(url) = &comment.url {
                    let url = url.clone();
                    row = row.child(
                        Button::new(("github-comment-open", index))
                            .small()
                            .ghost()
                            .label(label("github.details"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    );
                }
                body = body.child(row);
            }
            if pr.checks.len().max(pr.comments.len()) > self.shown {
                body = body.child(
                    Button::new("github-more")
                        .small()
                        .label(label("github.more"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shown += 20;
                            cx.notify();
                        })),
                );
            }
        }
        body.into_any_element()
    }
}
