//! A workspace-bound PR/CI snapshot. Remote writes require explicit user actions.
use super::*;
mod checks;
mod document;
mod presentation;
pub(super) use document::GithubUi;
mod composer;
mod threads;
pub(in crate::app) use threads::thread_body;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

struct GithubCommentPanel {
    host: WeakEntity<ConduitApp>,
    parent: WeakEntity<PullRequestPanel>,
    workspace: PathBuf,
    expected: crate::github::PullRequest,
    action: crate::github::write::Action,
    editor: Option<Entity<TextareaState>>,
    error: Option<String>,
    busy: bool,
    finished: bool,
    uncertain: bool,
    confirm_discard: bool,
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
    _subscription: Subscription,
    _editor_subscription: Option<Subscription>,
}

impl GithubCommentPanel {
    fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        if !self.finished
            && self
                .editor
                .as_ref()
                .is_some_and(|editor| !editor.read(cx).value().is_empty())
        {
            self.confirm_discard = true;
            cx.notify();
            return false;
        }
        true
    }

    fn writable(&self, cx: &App) -> bool {
        self.host.upgrade().is_some_and(|host| {
            let app = host.read(cx);
            app.writable
                && !app.closing
                && app
                    .store
                    .workspaces
                    .iter()
                    .any(|w| w.path == self.workspace)
        })
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.finished || self.uncertain || !self.writable(cx) {
            return;
        }
        let mut action = self.action.clone();
        if let Some(editor) = &self.editor {
            let body = editor.read(cx).value().to_string();
            match &mut action {
                crate::github::write::Action::Comment(text)
                | crate::github::write::Action::Reply { body: text, .. } => *text = body,
                _ => {}
            }
        }
        if let Err(error) = crate::github::write::validate(&self.expected, &self.expected, &action)
        {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.busy = true;
        self.confirm_discard = false;
        self.error = None;
        let workspace = self.workspace.clone();
        let expected = self.expected.clone();
        let cancel = self.cancel.clone();
        self._task = cx.spawn(async move |this, cx| {
            let preflight_cancel = cancel.clone();
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    crate::github::write::prepare(&workspace, &expected, &action, &preflight_cancel)
                })
                .await;
            let result = match prepared {
                Ok(prepared) => {
                    let allowed = this
                        .update(cx, |this, cx| this.writable(cx))
                        .unwrap_or(false);
                    if allowed {
                        cx.background_executor()
                            .spawn(async move { crate::github::write::perform(prepared, &cancel) })
                            .await
                    } else {
                        Err(crate::github::write::WriteError {
                            message: label("review.cannot_attach").into(),
                            uncertain: false,
                        })
                    }
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.finished = true;
                        let _ = this.parent.update(cx, |parent, cx| parent.reload(cx));
                    }
                    Err(error) => {
                        this.uncertain = error.uncertain;
                        this.error = Some(error.message);
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }
}

impl Drop for GithubCommentPanel {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(super) struct PullRequestPanel {
    host: WeakEntity<ConduitApp>,
    pub(super) workspace: PathBuf,
    pub(super) focus: FocusHandle,
    document: bool,
    scrolls: [ScrollHandle; 3],
    snapshot: Option<crate::github::PullRequest>,
    error: Option<String>,
    sync_result: Option<String>,
    loading: bool,
    shown: usize,
    tab: usize,
    expanded_threads: std::collections::HashSet<String>,
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
    _subscription: Option<Subscription>,
}

impl ConduitApp {
    pub(super) fn github_button(
        &self,
        cx: &mut Context<Self>,
        popover: Option<WeakEntity<gpui_kit::component::popover::PopoverState>>,
    ) -> Button {
        Button::new("github-pr")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::GitPullRequest)
            .tooltip(label("github.title"))
            .accessibility_label(label("github.title"))
            .selected(self.github_ui.active.is_some())
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(popover) = &popover {
                    let _ = popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                }
                this.open_github(window, cx);
            }))
    }
}

impl PullRequestPanel {
    pub(super) fn tab_title(&self) -> String {
        let project = self
            .workspace
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let title = self.snapshot.as_ref().map_or_else(
            || label("github.short_title").to_owned(),
            |pr| format!("PR #{}", pr.number),
        );
        format!("{title} · {project}")
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.loading = true;
        self.error = None;
        self.sync_result = None;
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
                    Ok(snapshot) => {
                        let changed = this
                            .snapshot
                            .as_ref()
                            .is_none_or(|old| old.url != snapshot.url || old.head != snapshot.head);
                        if changed {
                            this.expanded_threads.clear();
                            this.shown = 20;
                            for scroll in &this.scrolls {
                                scroll.set_offset(point(px(0.), px(0.)));
                            }
                        }
                        this.snapshot = Some(snapshot);
                    }
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
                if !self.document {
                    window.close_dialog(cx);
                }
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn open_comment(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_write(
            crate::github::write::Action::Comment(String::new()),
            window,
            cx,
        );
    }

    fn open_write(
        &self,
        action: crate::github::write::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let app = host.read(cx);
        if self.loading
            || self.error.is_some()
            || !app.writable
            || app.closing
            || !app
                .store
                .workspaces
                .iter()
                .any(|w| w.path == self.workspace)
        {
            return;
        }
        let Some(expected) = self.snapshot.clone() else {
            return;
        };
        if expected.state != "OPEN" {
            return;
        }
        let with_editor = !matches!(action, crate::github::write::Action::Resolve(_));
        let title = label(match &action {
            crate::github::write::Action::Comment(_) => "github.comment_title",
            crate::github::write::Action::Reply { .. } => "github.reply_title",
            crate::github::write::Action::Resolve(_) => "github.resolve_title",
        });
        let parent = cx.weak_entity();
        let editor = with_editor.then(|| {
            cx.new(|cx| {
                TextareaState::new(window, cx).placeholder(label("github.comment_placeholder"))
            })
        });
        let panel = cx.new(|cx| GithubCommentPanel {
            host: host.downgrade(),
            parent,
            workspace: self.workspace.clone(),
            expected,
            action,
            _editor_subscription: editor
                .as_ref()
                .map(|editor| cx.observe(editor, |_, _, cx| cx.notify())),
            editor,
            error: None,
            busy: false,
            finished: false,
            uncertain: false,
            confirm_discard: false,
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: cx.observe(&host, |_, _, cx| cx.notify()),
        });
        window.open_dialog(cx, move |dialog, window, cx| {
            let panel = panel.clone();
            let dismiss = panel.clone();
            let busy = panel.read(cx).busy;
            dialog
                .title(title)
                .overlay_closable(false)
                .keyboard(!busy)
                .close_button(!busy)
                .on_cancel(move |_, _, cx| dismiss.update(cx, |panel, cx| panel.can_close(cx)))
                .w(px(
                    (f32::from(window.viewport_size().width) - 48.).clamp(0., 720.)
                ))
                .content(move |body, _, _| body.child(panel.clone()))
        });
    }
}

impl Drop for PullRequestPanel {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Render for PullRequestPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writable = self.host.upgrade().is_some_and(|host| {
            let app = host.read(cx);
            app.writable
                && !app.closing
                && app
                    .store
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.path == self.workspace)
        });
        let mut header = v_flex().gap_3().child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(self.workspace.display().to_string()),
                )
                .when_some(self.snapshot.as_ref(), |row, pr| {
                    let url = pr.url.clone();
                    row.child(
                        Button::new("github-open")
                            .small()
                            .ghost()
                            .label(label("github.open_pr"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                })
                .child(
                    Button::new("github-refresh")
                        .small()
                        .ghost()
                        .label(label(if self.loading {
                            "github.loading"
                        } else {
                            "github.refresh"
                        }))
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                ),
        );
        let mut body = v_flex()
            .id(("github-pr-panel", self.tab))
            .debug_selector(|| "github-content".into())
            .gap_3()
            .min_h_0()
            .when(!self.document, |body| {
                body.max_h(px(
                    (f32::from(window.viewport_size().height) - 360.).max(100.)
                ))
            })
            .when(self.document, |body| body.flex_1())
            .track_scroll(&self.scrolls[self.tab])
            .overflow_y_scroll();
        if let Some(error) = &self.error {
            header = header.child(
                div()
                    .p_2()
                    .rounded_md()
                    .bg(rgb(palette().danger_bg))
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(pr) = &self.snapshot {
            header = header.child(self.overview(pr));
            let mut tabs = h_flex()
                .gap_1()
                .flex_wrap()
                .pb_2()
                .border_b_1()
                .border_color(rgb(palette().border));
            for (index, (key, count)) in [
                ("github.checks", pr.checks.len()),
                (
                    "github.threads",
                    pr.threads
                        .as_ref()
                        .map_or(0, |snapshot| snapshot.threads.len()),
                ),
                ("github.comments", pr.comments.len()),
            ]
            .into_iter()
            .enumerate()
            {
                let button = Button::new(("github-tab", index))
                    .small()
                    .selected(self.tab == index)
                    .label(format!("{} · {}", label(key), count))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = index;
                        cx.notify();
                    }));
                tabs = tabs.child(if self.tab == index {
                    button.primary()
                } else {
                    button.ghost()
                });
            }
            header = header.child(tabs);
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
            if self.tab == 1 {
                body = body.child(self.review_threads(writable && self.error.is_none(), cx));
            }
            if self.tab == 0 {
                if pr.checks.is_empty() {
                    body = body.child(label("github.no_checks"));
                }
                for index in presentation::check_order(&pr.checks)
                    .into_iter()
                    .take(self.shown)
                {
                    let check = &pr.checks[index];
                    let feedback = pr.feedback(index).unwrap_or_default();
                    let mut row = h_flex()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .border_1()
                        .border_color(rgb(palette().border))
                        .bg(rgb(palette().surface))
                        .flex_wrap()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(div().font_semibold().child(check.name.clone()))
                                .child(presentation::check_status(check)),
                        )
                        .child(
                            Button::new(("github-check-attach", index))
                                .small()
                                .ghost()
                                .label(label("github.attach"))
                                .disabled(!writable || self.loading || self.error.is_some())
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
                                    .primary()
                                    .label(label(if existing.is_some() {
                                        "github.open_check_task"
                                    } else {
                                        "github.create_check_task"
                                    }))
                                    .disabled(
                                        self.loading
                                            || self.error.is_some()
                                            || (!writable && existing.is_none()),
                                    )
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
            }
            if self.tab == 2 {
                body = body.child(
                    Button::new("github-publish-comment")
                        .small()
                        .primary()
                        .label(label("github.publish_comment"))
                        .disabled(
                            !writable || self.loading || self.error.is_some() || pr.state != "OPEN",
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.open_comment(window, cx))),
                );
                if pr.comments.is_empty() {
                    body = body.child(
                        div()
                            .p_3()
                            .text_color(rgb(palette().muted))
                            .child(label("github.no_comments")),
                    );
                }
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
                        .p_3()
                        .border_1()
                        .border_color(rgb(palette().border))
                        .rounded_md()
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(comment.author.clone()),
                        )
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
                                .disabled(!writable || self.loading || self.error.is_some())
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
            }
            if (self.tab == 0 && pr.checks.len() > self.shown)
                || (self.tab == 2 && pr.comments.len() > self.shown)
            {
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
        if self.snapshot.is_none() {
            body =
                body.child(
                    v_flex()
                        .p_6()
                        .gap_3()
                        .child(
                            div()
                                .text_size(ui_text_size(18.))
                                .font_semibold()
                                .child(label(if self.loading {
                                    "github.loading"
                                } else {
                                    "github.unavailable_title"
                                })),
                        )
                        .child(div().text_color(rgb(palette().muted)).child(label(
                            if self.loading {
                                "github.loading_hint"
                            } else {
                                "github.unavailable_hint"
                            },
                        ))),
                );
        }
        let host = self.host.clone();
        let workspace = self.workspace.clone();
        v_flex()
            .id("github-document")
            .debug_selector(|| "github-document".into())
            .key_context("PullRequestDocument")
            .track_focus(&self.focus)
            .on_action(move |_: &CloseDocument, window, cx| {
                if let Some(host) = host.upgrade() {
                    host.update(cx, |app, cx| {
                        if let Some(id) = app.github_ui.id_for(&workspace) {
                            app.close_documents(&[files::DocumentTab::PullRequest(id)], window, cx);
                        }
                    });
                }
            })
            .gap_3()
            .min_h_0()
            .min_w_0()
            .when(self.document, |page| page.size_full().p_4())
            .child(header)
            .child(body)
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(label("github.snapshot_hint")),
            )
            .into_any_element()
    }
}
