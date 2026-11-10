//! Local review controls shared by working-tree, branch and commit diff viewers.
use super::*;
use crate::review::{self, Anchor, Comment, Side, Status, Target};
use gpui_kit::component::diff::{DiffAnnotation, DiffLinePosition, DiffLineRange, DiffSide};

#[derive(Default)]
pub(super) struct ReviewUi {
    pub host: Option<WeakEntity<ConduitApp>>,
    pub fingerprint: String,
    pub open: bool,
    pub editor: Option<Entity<TextareaState>>,
    pub anchor: Option<Anchor>,
    pub draft_target: Option<Target>,
    pub selected: HashSet<u64>,
    pub error: Option<String>,
    pub shown: usize,
    pub _host_subscription: Option<Subscription>,
}

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}
fn diff_side(side: Side) -> DiffSide {
    match side {
        Side::Original => DiffSide::Original,
        Side::Modified => DiffSide::Modified,
    }
}
fn location(anchor: &Anchor) -> String {
    format!(
        "{} {}–{}",
        label(match anchor.side {
            Side::Original => "review.original",
            Side::Modified => "review.modified",
        }),
        anchor.start,
        anchor.end
    )
}

impl GitDiffDialog {
    pub(super) fn with_review(
        mut self,
        host: WeakEntity<ConduitApp>,
        cx: &mut Context<Self>,
    ) -> Self {
        self.review._host_subscription = host
            .upgrade()
            .map(|host| cx.observe(&host, |_, _, cx| cx.notify()));
        self.review.host = Some(host);
        self.review.shown = 30;
        self
    }

    fn review_host(&self) -> Option<Entity<ConduitApp>> {
        self.review.host.as_ref()?.upgrade()
    }

    fn review_writable(&self, cx: &App) -> bool {
        self.review_host().is_some_and(|host| {
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

    fn review_target(&self) -> Target {
        let source = if let Some(commit) = &self.commit {
            format!(
                "commit:{}:{}",
                commit.parent.as_deref().unwrap_or("empty"),
                commit.oid
            )
        } else if let Some(request) = &self.comparison {
            format!(
                "comparison:{:?}:{}:{}",
                request.kind,
                request.selected.oid,
                request.expected.head.as_deref().unwrap_or("empty")
            )
        } else {
            format!("working-tree:{:?}", self.scope)
        };
        Target {
            workspace: self.workspace.clone(),
            path: self.file.path.clone(),
            source,
        }
    }

    fn review_comments(&self, cx: &App) -> Vec<Comment> {
        let Some(host) = self.review_host() else {
            return vec![];
        };
        let target = self.review_target();
        host.read(cx)
            .store
            .reviews
            .comments
            .iter()
            .filter(|c| c.target == target)
            .cloned()
            .collect()
    }

    pub(super) fn begin_review_comment(
        &mut self,
        range: DiffLineRange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.review.open = true;
        if !self.review_writable(cx) || self.loading || self.error.is_some() {
            return;
        }
        if self.review.anchor.is_some() {
            self.review.error = Some(label("review.finish_draft").into());
            cx.notify();
            return;
        }
        if range.side() != range.end_side() {
            self.review.error = Some(label("review.one_side").into());
            cx.notify();
            return;
        }
        if !self
            .state
            .read(cx)
            .files()
            .iter()
            .any(|f| f.path() == range.path())
        {
            return;
        }
        let side = if range.side() == DiffSide::Original {
            Side::Original
        } else {
            Side::Modified
        };
        let anchor = Anchor::capture(
            self.last_patch.as_deref().unwrap_or(""),
            side,
            range.start(),
            range.end(),
        );
        match anchor {
            Ok(anchor) => {
                let editor = cx.new(|cx| {
                    TextareaState::new(window, cx).placeholder(label("review.comment_placeholder"))
                });
                editor.read(cx).focus_handle(cx).focus(window, cx);
                self.review.anchor = Some(anchor);
                self.review.draft_target = Some(self.review_target());
                self.review.editor = Some(editor);
                self.review.error = None;
            }
            Err(error) => self.review.error = Some(error),
        }
        cx.notify();
    }

    fn save_review_comment(&mut self, cx: &mut Context<Self>) {
        if !self.review_writable(cx) {
            return;
        }
        let (Some(host), Some(anchor), Some(editor)) = (
            self.review_host(),
            self.review.anchor.clone(),
            self.review.editor.as_ref(),
        ) else {
            return;
        };
        let target = self.review_target();
        // A scope change must not attach an old draft to a different comparison.
        if self.review.draft_target.as_ref() != Some(&target)
            || anchor.fingerprint != self.review.fingerprint
            || self.loading
            || self.error.is_some()
        {
            self.review.error = Some(label("review.draft_outdated").into());
            cx.notify();
            return;
        }
        let text = editor.read(cx).value().to_string();
        let result = host.update(cx, |app, cx| {
            let id = app.store.reviews.add(target, anchor, &text)?;
            app.persist();
            cx.notify();
            Ok::<_, String>(id)
        });
        match result {
            Ok(id) => {
                self.review.selected.insert(id);
                self.review.draft_target = None;
                self.review.anchor = None;
                self.review.editor = None;
                self.review.error = None;
                self.review.shown = self.review.shown.max(self.review_comments(cx).len());
            }
            Err(error) => self.review.error = Some(error),
        }
        cx.notify();
    }

    fn set_review_status(&mut self, id: u64, status: Status, cx: &mut Context<Self>) {
        if !self.review_writable(cx) {
            return;
        }
        let target = self.review_target();
        if let Some(host) = self.review_host() {
            host.update(cx, |app, cx| {
                if let Some(comment) = app
                    .store
                    .reviews
                    .comments
                    .iter_mut()
                    .find(|c| c.id == id && c.target == target)
                {
                    comment.status = status;
                    app.persist();
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    fn toggle_reviewed(&mut self, cx: &mut Context<Self>) {
        if !self.review_writable(cx) || self.loading || self.error.is_some() {
            return;
        }
        let target = self.review_target();
        let fingerprint = self.review.fingerprint.clone();
        if let Some(host) = self.review_host() {
            host.update(cx, |app, cx| {
                let viewed = !app.store.reviews.is_viewed(&target, &fingerprint);
                app.store.reviews.set_viewed(target, fingerprint, viewed);
                app.persist();
                cx.notify();
            });
        }
        cx.notify();
    }

    fn attach_review_feedback(
        &mut self,
        session: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.review_writable(cx) {
            return;
        }
        let Some(host) = self.review_host() else {
            return;
        };
        let target = self.review_target();
        let ids: Vec<_> = self.review.selected.iter().copied().collect();
        let result = host
            .read(cx)
            .store
            .reviews
            .feedback(&target, &ids, &self.review.fingerprint);
        let result = result.and_then(|text| {
            host.update(cx, |app, cx| {
                app.append_review_draft(&self.workspace, session, None, &text, window, cx)
            })
        });
        match result {
            Ok(()) => {
                self.review.error = None;
                if !self.embedded {
                    window.close_dialog(cx);
                }
            }
            Err(error) => self.review.error = Some(error),
        }
        cx.notify();
    }

    fn prepare_ai_review(
        &mut self,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.review_writable(cx) || self.loading || self.error.is_some() {
            return;
        }
        let Some(patch) = self
            .last_patch
            .as_ref()
            .filter(|p| !p.is_empty() && p.len() <= review::MAX_FEEDBACK_BYTES)
        else {
            self.review.error = Some(label("review.patch_limit").into());
            cx.notify();
            return;
        };
        let Some(host) = self.review_host() else {
            return;
        };
        let capture = match review::ai::Review::capture(self.review_target(), patch) {
            Ok(capture) => capture,
            Err(error) => {
                self.review.error = Some(error);
                cx.notify();
                return;
            }
        };
        let mut prompt = conduit::i18n::format(
            "review.ai_prompt",
            &[
                ("path", self.file.path.clone()),
                ("source", self.review_target().source),
                ("fingerprint", self.review.fingerprint.clone()),
                ("notes", self.notes.join("\n")),
                (
                    "patch",
                    patch
                        .lines()
                        .map(|line| format!("    {line}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ],
        );
        prompt.push_str("\n\n");
        prompt.push_str(&capture.instructions());
        if prompt.len() > review::MAX_FEEDBACK_BYTES {
            self.review.error = Some(label("review.patch_limit").into());
            cx.notify();
            return;
        }
        let result = host.update(cx, |app, cx| {
            app.prepare_structured_review(capture, provider, &prompt, window, cx)
        });
        match result {
            Ok(()) => {
                self.review.error = None;
                if !self.embedded {
                    window.close_dialog(cx);
                }
            }
            Err(error) => self.review.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn review_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(host) = self.review_host() else {
            return div().into_any_element();
        };
        let writable = self.review_writable(cx);
        let ready = writable
            && !self.loading
            && self.error.is_none()
            && !self.review.fingerprint.is_empty();
        let viewed = host
            .read(cx)
            .store
            .reviews
            .is_viewed(&self.review_target(), &self.review.fingerprint);
        let count = self.review_comments(cx).len();
        let owner = cx.weak_entity();
        let engines: Vec<_> = Provider::ALL
            .into_iter()
            .filter(|p| host.read(cx).store.cli.enabled(*p))
            .collect();
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_1()
            .items_center()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                Button::new("review-panel")
                    .label(conduit::i18n::format(
                        "review.feedback_count",
                        &[("count", count.to_string())],
                    ))
                    .small()
                    .ghost()
                    .selected(self.review.open)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.review.open = !this.review.open;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("review-viewed")
                    .label(label(if viewed {
                        "review.viewed"
                    } else {
                        "review.mark_viewed"
                    }))
                    .small()
                    .ghost()
                    .selected(viewed)
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_reviewed(cx))),
            )
            .child(
                Button::new("review-ai")
                    .label(label("review.prepare_ai"))
                    .tooltip(label("review.ai_hint"))
                    .small()
                    .ghost()
                    .disabled(!ready || engines.is_empty())
                    .dropdown_menu(move |mut menu, _, _| {
                        for provider in &engines {
                            let provider = *provider;
                            let owner = owner.clone();
                            menu = menu.item(PopupMenuItem::new(provider.label()).on_click(
                                move |_, window, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.prepare_ai_review(provider, window, cx)
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
            .into_any_element()
    }

    pub(super) fn review_diff(&self, diff: Diff, cx: &mut Context<Self>) -> Diff {
        let writable = self.review_writable(cx) && !self.loading && self.error.is_none();
        let owner = cx.weak_entity();
        let mut diff = diff;
        if writable {
            diff = diff.on_add_annotation(move |range, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.begin_review_comment(range.clone(), window, cx)
                });
            });
        }
        let comments: Vec<_> = self
            .review_comments(cx)
            .into_iter()
            .filter(|c| {
                c.anchor.fingerprint == self.review.fingerprint
                    && !matches!(c.status, Status::Resolved | Status::Dismissed)
            })
            .collect();
        let Some(file) = self.state.read(cx).files().first() else {
            return diff;
        };
        let annotations: Vec<_> = comments
            .iter()
            .map(|c| {
                DiffAnnotation::line(
                    SharedString::from(format!("review-note-{}", c.id)),
                    DiffLinePosition::new(
                        file.path().clone(),
                        diff_side(c.anchor.side),
                        c.anchor.end,
                    ),
                )
            })
            .collect();
        let owner = cx.weak_entity();
        diff.annotations(annotations)
            .render_annotation(move |annotation, _, _| {
                let comment = comments.iter().find(|c| {
                    annotation.id() == &ElementId::Name(format!("review-note-{}", c.id).into())
                });
                let Some(comment) = comment else {
                    return div().into_any_element();
                };
                let id = comment.id;
                let owner = owner.clone();
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .bg(rgb(palette().surface_strong))
                    .border_l_2()
                    .border_color(rgb(palette().focus))
                    .child(div().text_size(ui_text_size(11.)).child(format!(
                        "#{} · {}",
                        comment.id,
                        comment.status.label()
                    )))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(comment.display_text()),
                    )
                    .child(
                        Button::new(("review-open-note", id))
                            .small()
                            .ghost()
                            .label(label("review.show_feedback"))
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.review.open = true;
                                    this.review.selected.insert(id);
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element()
            })
    }

    pub(super) fn review_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.review.open {
            return div().into_any_element();
        }
        let writable = self.review_writable(cx);
        let comments = self.review_comments(cx);
        let host = self.review_host();
        let sessions: Vec<_> = host
            .as_ref()
            .and_then(|h| {
                h.read(cx)
                    .store
                    .workspaces
                    .iter()
                    .find(|w| w.path == self.workspace)
            })
            .map(|w| {
                w.sessions
                    .iter()
                    .filter(|s| !s.archived)
                    .map(|s| (s.id, s.title.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let owner = cx.weak_entity();
        let mut panel = v_flex()
            .id("review-panel-body")
            .w_full()
            .max_h(px(
                (f32::from(window.viewport_size().height) * 0.35).clamp(100., 300.)
            ))
            .overflow_y_scroll()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(rgb(palette().border));
        if let Some(error) = &self.review.error {
            panel = panel.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(host) = &host {
            let app = host.read(cx);
            if let Some(error) = &app.persistence.error {
                panel = panel.child(div().text_color(rgb(palette().danger_text)).child(
                    conduit::i18n::format("review.save_failed", &[("error", error.clone())]),
                ));
            } else if app.persistence.busy() || app.dirty {
                panel = panel.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .child(label("review.saving")),
                );
            }
        }
        if let (Some(editor), Some(anchor)) = (&self.review.editor, &self.review.anchor) {
            panel = panel
                .child(div().text_size(ui_text_size(11.)).child(location(anchor)))
                .child(Textarea::new(editor).h(px(80.)))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("review-save")
                                .label(label("review.save_comment"))
                                .small()
                                .disabled(!writable)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.save_review_comment(cx)),
                                ),
                        )
                        .child(
                            Button::new("review-cancel")
                                .label(label("review.cancel_comment"))
                                .small()
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.review.draft_target = None;
                                    this.review.anchor = None;
                                    this.review.editor = None;
                                    this.review.error = None;
                                    cx.notify();
                                })),
                        ),
                );
        } else {
            panel = panel.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(label("review.add_hint")),
            );
        }
        panel = panel.child(
            Button::new("review-attach")
                .label(label("review.attach_selected"))
                .small()
                .disabled(!writable || self.review.selected.is_empty())
                .dropdown_menu(move |mut menu, _, _| {
                    let fresh = owner.clone();
                    menu = menu.item(PopupMenuItem::new(label("review.new_session")).on_click(
                        move |_, window, cx| {
                            let _ = fresh.update(cx, |this, cx| {
                                this.attach_review_feedback(None, window, cx)
                            });
                        },
                    ));
                    for (id, title) in &sessions {
                        let id = *id;
                        let owner = owner.clone();
                        menu = menu.item(PopupMenuItem::new(format!("{title} · #{id}")).on_click(
                            move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.attach_review_feedback(Some(id), window, cx)
                                });
                            },
                        ));
                    }
                    menu
                }),
        );
        for comment in comments.iter().take(self.review.shown.max(30)) {
            let id = comment.id;
            let current = comment.anchor.fingerprint == self.review.fingerprint;
            let owner = cx.weak_entity();
            let card = v_flex()
                .id(("review-comment", id))
                .gap_1()
                .p_2()
                .border_1()
                .border_color(rgb(palette().border))
                .rounded_md()
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new(("review-select", id))
                                .label(format!("#{} · {}", id, location(&comment.anchor)))
                                .small()
                                .ghost()
                                .selected(self.review.selected.contains(&id))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.review.selected.remove(&id) {
                                        this.review.selected.insert(id);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("review-status", id))
                                .label(comment.status.label())
                                .small()
                                .ghost()
                                .disabled(!writable)
                                .dropdown_menu(move |mut menu, _, _| {
                                    for status in Status::ALL {
                                        let owner = owner.clone();
                                        menu =
                                            menu.item(PopupMenuItem::new(status.label()).on_click(
                                                move |_, _, cx| {
                                                    let _ = owner.update(cx, |this, cx| {
                                                        this.set_review_status(id, status, cx)
                                                    });
                                                },
                                            ));
                                    }
                                    menu
                                }),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(if current {
                                    palette().muted
                                } else {
                                    palette().warning_text
                                }))
                                .child(label(if current {
                                    "review.current"
                                } else {
                                    "review.outdated"
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .child(comment.display_text()),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(comment.anchor.snippet.clone()),
                );
            panel = panel.child(card);
        }
        if comments.len() > self.review.shown.max(30) {
            panel = panel.child(
                Button::new("review-more")
                    .label(label("review.more"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.review.shown = this.review.shown.max(30) + 30;
                        cx.notify();
                    })),
            );
        }
        panel.into_any_element()
    }
}

impl ConduitApp {
    pub(super) fn append_review_draft(
        &mut self,
        workspace: &Path,
        session: Option<u64>,
        reviewer: Option<Provider>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if !self.writable || self.closing || self.composer_save_pending() {
            return Err(label("review.cannot_attach").into());
        }
        let index = self
            .store
            .workspaces
            .iter()
            .position(|w| w.path == workspace)
            .ok_or_else(|| label("review.target_missing").to_owned())?;
        if session.is_some_and(|id| {
            !self.store.workspaces[index]
                .sessions
                .iter()
                .any(|s| s.id == id && !s.archived)
        }) {
            return Err(label("review.target_missing").into());
        }
        if session
            .is_some_and(|id| self.session_draft_save_pending(id) || self.pending_new_session(id))
        {
            return Err(label("review.cannot_attach").into());
        }
        if let Some(provider) = reviewer
            && !self.store.cli.enabled(provider)
        {
            return Err(label("review.cannot_attach").into());
        }
        if let Some(id) = session {
            self.switch(index, Some(id), window, cx);
        } else {
            let previous_workspace = self.selected_workspace;
            let previous_session = self.selected_session;
            let id = self
                .create_session(index, reviewer.unwrap_or(self.new_session_provider), cx)
                .ok_or_else(|| label("review.cannot_attach").to_owned())?;
            self.selected_workspace = previous_workspace;
            self.selected_session = previous_session;
            self.switch(index, Some(id), window, cx);
            if let Some(provider) = reviewer
                && let Some(id) = self.selected_session
                && let Some(s) = self.session_mut(id)
            {
                s.permission = PermissionLevel::ReadOnly;
                s.title = format!("{} · {}", label("review.review_session"), provider.label());
            }
        }
        let id = self
            .selected_session
            .ok_or_else(|| label("review.cannot_attach").to_owned())?;
        let previous = self.input.read(cx).value().to_string();
        let next = if previous.trim().is_empty() {
            text.to_owned()
        } else {
            format!("{previous}\n\n{text}")
        };
        self.input
            .update(cx, |input, cx| input.set_value(next.clone(), window, cx));
        self.drafts.insert(id, next);
        self.settings_ui.open = false;
        self.persist();
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
        Ok(())
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

mod ai;
mod inbox;
