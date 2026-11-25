//! A workspace-bound PR/CI snapshot. Remote writes require explicit user actions.
use super::*;
use gpui_kit::component::{skeleton::Skeleton, tag::Tag, tooltip::Tooltip};
mod checks;
mod details;
mod document;
mod presentation;
mod sidebar;
pub(super) use document::GithubUi;
mod composer;
mod sign_in;
mod threads;
pub(in crate::app) use threads::thread_body;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

fn content_skeleton(id: &'static str) -> AnyElement {
    v_flex()
        .id(id)
        .debug_selector(move || id.into())
        .w_full()
        .gap_4()
        .p_3()
        .children((0..2).map(|_| {
            v_flex()
                .w_full()
                .gap_2()
                .child(Skeleton::new().w(px(100.)).h(px(12.)).rounded_sm())
                .children([0.94, 0.82, 0.62].map(|width| {
                    Skeleton::new()
                        .w(relative(width))
                        .h(px(12.))
                        .rounded_sm()
                        .secondary()
                }))
        }))
        .into_any_element()
}

struct GithubCommentPanel {
    inline: bool,
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
    details: details::DetailsUi,
    scrolls: [ScrollHandle; 3],
    snapshot: Option<crate::github::PullRequest>,
    requested: Option<(crate::github::forge::Forge, u64, String)>,
    error: Option<String>,
    sign_in: sign_in::SignInSlot,
    sync_result: Option<String>,
    loading: bool,
    shown: usize,
    tab: usize,
    expanded_threads: std::collections::HashSet<String>,
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
    _subscription: Option<Subscription>,
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
            |pr| pr.forge.request_name(pr.number),
        );
        format!("{title} · {project}")
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_with_provider(None, cx);
    }

    fn reload_with_provider(
        &mut self,
        provider: Option<Option<crate::github::forge::Forge>>,
        cx: &mut Context<Self>,
    ) {
        self.details.cancel_load();
        if provider.is_some()
            && self.host.upgrade().is_none_or(|host| {
                let app = host.read(cx);
                !app.writable
                    || app.closing
                    || !app
                        .store
                        .workspaces
                        .iter()
                        .any(|w| w.path == self.workspace)
            })
        {
            return;
        }
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        if provider.is_some() {
            self.snapshot = None;
            self.requested = None;
            self.details.reset_request();
        }
        self.loading = true;
        self.error = None;
        self.sign_in.set(None);
        self.sync_result = None;
        let cancel = self.cancel.clone();
        let workspace = self.workspace.clone();
        let requested = self.requested.clone();
        self._task = cx.spawn(async move |this, cx| {
            let ticket = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(provider) = provider {
                        crate::github::forge::select(&workspace, provider, &ticket)
                            .map_err(sign_in::describe)?;
                    }
                    if let Some((forge, number, url)) = requested {
                        crate::github::load_request(&workspace, forge, number, &url, &ticket)
                    } else {
                        crate::github::load(&workspace, &ticket)
                    }
                    .map_err(sign_in::describe)
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
                        this.load_details(cx);
                    }
                    Err((error, required)) => {
                        this.error = Some(error);
                        this.sign_in.set(required);
                    }
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

    fn open_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_write(
            crate::github::write::Action::Comment(String::new()),
            window,
            cx,
        );
    }

    fn open_write(
        &mut self,
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
        let inline = self.document && matches!(action, crate::github::write::Action::Comment(_));
        if inline && let Some(form) = self.details.composers.get(&expected.url) {
            if let Some(editor) = &form.read(cx).editor {
                editor.read(cx).focus_handle(cx).focus(window, cx);
            }
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
            inline,
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
        if inline {
            let url = panel.read(cx).expected.url.clone();
            self.details.composers.insert(url, panel.clone());
            if let Some(editor) = &panel.read(cx).editor {
                editor.read(cx).focus_handle(cx).focus(window, cx);
            }
            cx.notify();
            return;
        }
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
        let mut document_tabs = None;
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
        let mut header = v_flex().gap_2().flex_shrink_0().child(
            h_flex()
                .gap_2()
                .when(self.document, |row| row.flex_wrap())
                .items_center()
                .justify_between()
                .child(
                    div()
                        .id("github-workspace-tag")
                        .debug_selector(|| "github-workspace-tag".into())
                        .tooltip({
                            let path = display_path(&self.workspace);
                            move |window, cx| Tooltip::new(path.clone()).build(window, cx)
                        })
                        .child(
                            Tag::secondary()
                                .small()
                                .text_size(ui_text_size(11.))
                                .max_w(px(180.))
                                .overflow_hidden()
                                .child(
                                    div().truncate().child(
                                        self.workspace
                                            .file_name()
                                            .unwrap_or(self.workspace.as_os_str())
                                            .to_string_lossy()
                                            .into_owned(),
                                    ),
                                ),
                        ),
                )
                .child({
                    let panel = cx.weak_entity();
                    Button::new("forge-provider")
                        .small()
                        .ghost()
                        .label(
                            self.snapshot
                                .as_ref()
                                .map_or(label("forge.select"), |pr| pr.forge.name()),
                        )
                        .tooltip(label("forge.select_hint"))
                        .disabled(self.loading || !writable)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (name, provider) in [
                                (label("forge.auto"), None),
                                ("GitHub", Some(crate::github::forge::Forge::Github)),
                                ("GitLab", Some(crate::github::forge::Forge::Gitlab)),
                            ] {
                                let panel = panel.clone();
                                menu = menu.item(PopupMenuItem::new(name).on_click(
                                    move |_, _, cx| {
                                        let _ = panel.update(cx, |this, cx| {
                                            this.reload_with_provider(Some(provider), cx);
                                        });
                                    },
                                ));
                            }
                            menu
                        })
                })
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
                        .label(label("github.refresh"))
                        .disabled(self.loading || self.details.loading)
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
        if let Some(form) = self.sign_in.view(window, cx, Self::reload) {
            header = header.child(form);
        }
        if self.loading {
            body = body.child(content_skeleton("github-content-skeleton"));
        } else if let Some(pr) = &self.snapshot {
            if !self.document {
                header = header.child(self.overview(pr));
            }
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
                    .label(format!(
                        "{} · {}",
                        label(if self.document && index == 2 {
                            "github.detail_activity"
                        } else {
                            key
                        }),
                        count
                    ))
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
            if self.document {
                document_tabs = Some(tabs.into_any_element());
            } else {
                header = header.child(tabs);
            }
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
                if let Some(error) = &pr.checks_error {
                    body = body.child(
                        div()
                            .text_color(rgb(palette().danger_text))
                            .child(format!("{}: {error}", label("gitlab.checks_failed"))),
                    );
                }
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
                if self.document {
                    body = body.child(self.detail_description());
                } else {
                    body = body.child(
                        Button::new("github-publish-comment")
                            .small()
                            .primary()
                            .label(label("github.publish_comment"))
                            .disabled(
                                !writable
                                    || self.loading
                                    || self.error.is_some()
                                    || pr.state != "OPEN",
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_comment(window, cx)),
                            ),
                    );
                }
                if pr.comments.is_empty() {
                    body = body.child(
                        div()
                            .p_3()
                            .text_color(rgb(palette().muted))
                            .child(label("github.no_comments")),
                    );
                }
                for index in (0..pr.comments.len().min(self.shown)).rev() {
                    let comment = &pr.comments[index];
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
                        .when(!self.document, |row| {
                            row.border_1()
                                .border_color(rgb(palette().border))
                                .rounded_md()
                        })
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(comment.author.clone()),
                        )
                        .child(if self.document {
                            TextView::markdown(
                                SharedString::from(format!("github-comment-body-{index}")),
                                comment.body.clone(),
                            )
                            .selectable(true)
                            .text_size(ui_text_size(12.))
                            .into_any_element()
                        } else {
                            div()
                                .text_size(ui_text_size(12.))
                                .child(comment.body.clone())
                                .into_any_element()
                        })
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
                if self.document {
                    body = body.child(self.review_threads(writable && self.error.is_none(), cx));
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
        if self.snapshot.is_none() && !self.loading {
            body = body.child(
                v_flex()
                    .p_6()
                    .gap_3()
                    .child(
                        div()
                            .text_size(ui_text_size(18.))
                            .font_semibold()
                            .child(label("github.unavailable_title")),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().muted))
                            .child(label("github.unavailable_hint")),
                    ),
            );
        }
        if self.document {
            return self.document_layout(
                header.into_any_element(),
                body.into_any_element(),
                document_tabs,
                window,
                cx,
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
            .gap_2()
            .min_h_0()
            .min_w_0()
            .when(self.document, |page| page.size_full().p_3())
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
