//! Read-only Git diff dialog and navigation.

use super::*;

pub(super) fn git_change_color(kind: git::ChangeKind) -> u32 {
    match kind {
        git::ChangeKind::Added => palette().git_added_text,
        git::ChangeKind::Modified => palette().git_modified_text,
        git::ChangeKind::Deleted => palette().git_deleted_text,
    }
}

/// Read-only diff window for one Git change, modelled on JetBrains' diff
/// viewer: a version bar, navigation and folding tools, and a split view.
pub(super) struct GitDiffDialog {
    pub(super) embedded: bool,
    focus: FocusHandle,
    pub(super) last_patch: Option<String>,
    pub(super) review: super::git_review::ReviewUi,
    pub(super) comparison: Option<git::ComparisonRequest>,
    pub(super) commit: Option<git::CommitDiffRequest>,
    pub(super) workspace: PathBuf,
    pub(super) file: git::ChangedFile,
    pub(super) scope: git::FileDiffScope,
    pub(super) state: Entity<DiffState>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) notes: Vec<String>,
    /// Whether the viewer currently reveals every unchanged line.
    pub(super) unchanged_expanded: bool,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) _load_task: Task<()>,
    pub(super) _state_subscription: Subscription,
}

impl GitDiffDialog {
    pub(super) fn new(
        workspace: PathBuf,
        file: git::ChangedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_source(workspace, file, None, window, cx)
    }

    pub(super) fn new_document(
        workspace: PathBuf,
        file: git::ChangedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(workspace, file, window, cx);
        view.embedded = true;
        view
    }

    pub(super) fn new_source(
        workspace: PathBuf,
        file: git::ChangedFile,
        comparison: Option<git::ComparisonRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_sources(workspace, file, comparison, None, window, cx)
    }

    pub(super) fn new_commit(
        workspace: PathBuf,
        file: git::ChangedFile,
        commit: git::CommitDiffRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_sources(workspace, file, None, Some(commit), window, cx)
    }

    fn with_sources(
        workspace: PathBuf,
        file: git::ChangedFile,
        comparison: Option<git::ComparisonRequest>,
        commit: Option<git::CommitDiffRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| {
            DiffState::new(Vec::new(), cx)
                .with_mode(DiffMode::Split)
                .with_context_lines(Some(3))
        });
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let mut dialog = Self {
            embedded: false,
            focus: cx.focus_handle(),
            last_patch: None,
            review: Default::default(),
            comparison,
            commit,
            workspace,
            scope: if file.fully_staged() {
                git::FileDiffScope::Staged
            } else {
                git::FileDiffScope::All
            },
            file,
            state,
            loading: true,
            error: None,
            notes: Vec::new(),
            unchanged_expanded: false,
            cancelled: Arc::new(AtomicBool::new(false)),
            _load_task: Task::ready(()),
            _state_subscription: subscription,
        };
        dialog.reload(window, cx);
        dialog
    }

    pub(super) fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancelled.store(true, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.cancelled = cancelled.clone();
        self.loading = true;
        self.error = None;
        if !self.embedded {
            self.notes.clear();
            self.unchanged_expanded = false;
            self.state
                .update(cx, |state, cx| state.set_files(Vec::new(), cx));
        }
        cx.notify();
        let workspace = self.workspace.clone();
        let file = self.file.clone();
        let scope = self.scope;
        let comparison = self.comparison.clone();
        let commit = self.commit.clone();
        let ticket = cancelled.clone();
        self._load_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if let Some(commit) = commit {
                        git::load_commit_diff(&workspace, &commit, &file, &cancelled)
                    } else if let Some(comparison) = comparison {
                        git::load_comparison_diff(&workspace, &comparison, &file, &cancelled)
                    } else if scope == git::FileDiffScope::All {
                        git::load_file_diff(&workspace, &file, &cancelled)
                    } else {
                        git::load_file_diff_scoped(&workspace, &file, scope, &cancelled)
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if ticket.load(Ordering::Relaxed) || !Arc::ptr_eq(&ticket, &this.cancelled) {
                    return;
                }
                this.finish_load(result, window, cx);
            });
        });
    }

    pub(super) fn finish_load(
        &mut self,
        result: anyhow::Result<git::FileDiff>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_focus = self.focus.is_focused(window)
            || self.state.read(cx).focus_handle(cx).is_focused(window);
        self.loading = false;
        match result {
            Ok(diff)
                if self.embedded && self.last_patch.as_deref() == Some(diff.patch.as_str()) =>
            {
                self.notes = diff.notes;
                self.error = None;
            }
            Ok(diff) => match DiffFile::parse(&diff.patch) {
                Ok(files) => {
                    self.review.fingerprint = crate::review::fingerprint(&diff.patch);
                    self.last_patch = Some(diff.patch);
                    self.unchanged_expanded = false;
                    self.notes = diff.notes;
                    self.state
                        .update(cx, |state, cx| state.set_files(files, cx));
                }
                Err(error) => {
                    self.error = Some(conduit::i18n::format(
                        "git_diff.unable_to_parse_this_file_s_diff",
                        &[("error", format!("{}", error))],
                    ))
                }
            },
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        // Empty, binary and error bodies do not render DiffState. Keep keyboard actions
        // attached to a visible focus target, including same-patch recovery after a failure.
        if !self.embedded || had_focus {
            self.focus_diff(window, cx);
        }
        cx.notify();
    }

    pub(super) fn focus_diff(&self, window: &mut Window, cx: &mut Context<Self>) {
        let files = self.state.read(cx).files();
        if files.is_empty() || files.iter().any(DiffFile::is_binary) || self.error.is_some() {
            self.focus.focus(window, cx);
        } else {
            window.focus(&self.state.read(cx).focus_handle(cx), cx);
        }
    }

    pub(super) fn set_mode(&mut self, mode: DiffMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).mode() == mode {
            self.focus_diff(window, cx);
            return;
        }
        self.state.update(cx, |state, cx| state.set_mode(mode, cx));
        self.focus_diff(window, cx);
        cx.notify();
    }

    pub(super) fn next_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.next_change(cx));
        self.focus_diff(window, cx);
    }

    pub(super) fn previous_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.previous_change(cx));
        self.focus_diff(window, cx);
    }

    pub(super) fn toggle_unchanged(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unchanged_expanded = !self.unchanged_expanded;
        let expanded = self.unchanged_expanded;
        self.state.update(cx, |state, cx| {
            if expanded {
                state.expand_unchanged(cx);
            } else {
                state.collapse_unchanged(cx);
            }
        });
        self.focus_diff(window, cx);
        cx.notify();
    }

    pub(super) fn versions(&self) -> (String, String) {
        if let Some(commit) = &self.commit {
            return (
                commit
                    .parent
                    .as_ref()
                    .map(|oid| {
                        conduit::i18n::format(
                            "git_diff.parent_commit",
                            &[("arg0", oid[..7].to_string())],
                        )
                    })
                    .unwrap_or_else(|| {
                        conduit::i18n::text_static("git_diff.empty_tree_initial_commit").into()
                    }),
                conduit::i18n::format("git_diff.commit", &[("arg0", commit.oid[..7].to_string())]),
            );
        }
        if let Some(request) = &self.comparison {
            let before = format!(
                "{} · {}",
                request.selected.name,
                &request.selected.oid[..7.min(request.selected.oid.len())]
            );
            let after = if request.kind == git::ComparisonKind::WorkingTree {
                conduit::i18n::text_static("git_diff.working_tree_including_staged").into()
            } else {
                format!(
                    "{} · {}",
                    request
                        .expected
                        .current_ref
                        .as_deref()
                        .unwrap_or("HEAD")
                        .trim_start_matches("refs/heads/"),
                    request
                        .expected
                        .head
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(7)
                        .collect::<String>()
                )
            };
            return (before, after);
        }
        match self.scope {
            git::FileDiffScope::Staged => (
                conduit::i18n::text_static("git_diff.head_empty_tree_if_no_commits").into(),
                conduit::i18n::text_static("git_diff.staging_area_changes_to_commit").into(),
            ),
            git::FileDiffScope::Unstaged => (
                conduit::i18n::text_static("git_diff.staging_area").into(),
                conduit::i18n::text_static("git_diff.working_tree_unstaged_changes").into(),
            ),
            git::FileDiffScope::All => (
                conduit::i18n::text_static("git_diff.head_empty_tree_if_no_commits").into(),
                conduit::i18n::text_static("git_diff.working_tree_all_changes").into(),
            ),
        }
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let kind = self.file.kind();
        let color = git_change_color(kind);
        let (before, after) = self.versions();
        let version_tag = |label: String, emphasized: bool| {
            div()
                .min_w_0()
                .max_w(px(260.))
                .truncate()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(4.))
                .bg(rgb(palette().surface_strong))
                .text_size(ui_text_size(9.))
                .text_color(rgb(if emphasized {
                    palette().text
                } else {
                    palette().muted
                }))
                .child(label)
        };
        let files = self.state.read(cx).files();
        let additions: usize = files.iter().map(DiffFile::additions).sum();
        let deletions: usize = files.iter().map(DiffFile::deletions).sum();
        let binary = files.iter().any(DiffFile::is_binary);
        let stats = h_flex()
            .flex_shrink_0()
            .gap(px(5.))
            .text_size(ui_text_size(10.))
            .child(
                div()
                    .text_color(rgb(palette().git_added_text))
                    .child(format!("+{additions}")),
            )
            .child(
                div()
                    .text_color(rgb(palette().git_deleted_text))
                    .child(format!("-{deletions}")),
            );
        let mut details = h_flex()
            .w_full()
            .min_w_0()
            .gap(px(8.))
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_text_size(13.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child(self.file.path.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .bg(rgb(palette().surface_strong))
                    .text_size(ui_text_size(9.))
                    .font_semibold()
                    .text_color(rgb(color))
                    .child(kind.label()),
            )
            .when(!binary, |row| row.child(stats))
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(5.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .text_size(ui_text_size(9.))
                    .text_color(rgb(palette().faint))
                    .child(conduit::i18n::text_static("git_diff.read_only")),
            );
        if let Some(original) = &self.file.original {
            details = details.child(
                div()
                    .flex_shrink_0()
                    .text_size(ui_text_size(9.))
                    .text_color(rgb(palette().faint))
                    .child(format!("← {original}")),
            );
        }
        v_flex()
            .w_full()
            .flex_shrink_0()
            .gap(px(4.))
            .px(px(16.))
            .pt(px(14.))
            .pb(px(10.))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap(px(10.))
                    .items_center()
                    .child(
                        Icon::new(FullIconName::FileDiff)
                            .size(px(16.))
                            .text_color(rgb(palette().muted)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(5.))
                            .child(details)
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .gap(px(5.))
                                    .items_center()
                                    .child(version_tag(before, true))
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .size(px(11.))
                                            .text_color(rgb(palette().faint)),
                                    )
                                    .child(version_tag(after, true))
                                    .when(binary, |row| {
                                        row.child(
                                            div()
                                                .text_size(ui_text_size(9.))
                                                .text_color(rgb(palette().warning_text))
                                                .child(conduit::i18n::text_static(
                                                    "git_diff.binary",
                                                )),
                                        )
                                    }),
                            ),
                    )
                    .when(!self.embedded, |row| {
                        row.child(
                            Button::new("git-diff-close")
                                .ghost()
                                .small()
                                .compact()
                                .icon(FullIconName::X)
                                .tooltip(conduit::i18n::text_static(
                                    "git_diff.close_diff_window_esc",
                                ))
                                .accessibility_label(conduit::i18n::text_static(
                                    "git_diff.close_diff_window",
                                ))
                                .cursor_pointer()
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                    }),
            )
            .when(
                self.comparison.is_none() && self.commit.is_none(),
                |column| {
                    column.child(
                        h_flex().gap_1().flex_wrap().children(
                            [
                                git::FileDiffScope::Staged,
                                git::FileDiffScope::Unstaged,
                                git::FileDiffScope::All,
                            ]
                            .into_iter()
                            .enumerate()
                            .map(|(index, scope)| {
                                Button::new(("git-diff-scope", index))
                                    .label(scope.label())
                                    .small()
                                    .ghost()
                                    .selected(self.scope == scope)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.scope != scope {
                                            this.scope = scope;
                                            this.last_patch = None;
                                            this.state.update(cx, |state, cx| {
                                                state.set_files(Vec::new(), cx)
                                            });
                                            this.reload(window, cx);
                                        }
                                    }))
                            }),
                        ),
                    )
                },
            )
            .into_any_element()
    }

    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.state.read(cx).mode();
        let separator = || {
            div()
                .w(px(1.))
                .h(px(16.))
                .flex_shrink_0()
                .bg(rgb(palette().border))
        };
        let mode_button = |id: &'static str, label: &'static str, icon: FullIconName, value| {
            Button::new(id)
                .label(label)
                .icon(icon)
                .ghost()
                .small()
                .selected(mode == value)
                .tooltip(conduit::i18n::format(
                    "git_diff.view",
                    &[("label", label.to_string())],
                ))
                .accessibility_label(conduit::i18n::format(
                    "git_diff.view",
                    &[("label", label.to_string())],
                ))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_mode(value, window, cx);
                }))
        };
        h_flex()
            .w_full()
            .flex_shrink_0()
            .gap(px(6.))
            .items_center()
            .px(px(12.))
            .py(px(6.))
            .border_t_1()
            .border_b_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().sidebar_bg))
            .child(mode_button(
                "git-diff-split",
                conduit::i18n::text_static("chat.turn_diff.side_by_side"),
                FullIconName::Columns2,
                DiffMode::Split,
            ))
            .child(mode_button(
                "git-diff-unified",
                conduit::i18n::text_static("chat.turn_diff.unified"),
                FullIconName::Rows3,
                DiffMode::Unified,
            ))
            .child(separator())
            .child(
                Button::new("git-diff-previous")
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::ChevronUp)
                    .tooltip(conduit::i18n::format(
                        "git_diff.previous_change",
                        &[("arg0", shortcuts::hint("previous_diff", cx).to_string())],
                    ))
                    .accessibility_label(conduit::i18n::text_static(
                        "git_diff.previous_change_1ca9b9",
                    ))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.previous_change(window, cx);
                    })),
            )
            .child(
                Button::new("git-diff-next")
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::ChevronDown)
                    .tooltip(conduit::i18n::format(
                        "git_diff.next_change",
                        &[("arg0", shortcuts::hint("next_diff", cx).to_string())],
                    ))
                    .accessibility_label(conduit::i18n::text_static("git_diff.next_change_425f6c"))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.next_change(window, cx);
                    })),
            )
            .child(separator())
            .child(
                Button::new("git-diff-fold")
                    .label(if self.unchanged_expanded {
                        conduit::i18n::text_static("git_diff.fold_unchanged_lines")
                    } else {
                        conduit::i18n::text_static("git_diff.expand_unchanged_lines")
                    })
                    .ghost()
                    .small()
                    .tooltip(if self.unchanged_expanded {
                        conduit::i18n::text_static("git_diff.show_only_context_around_changes")
                    } else {
                        conduit::i18n::text_static("git_diff.show_all_unchanged_lines")
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_unchanged(window, cx);
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("git-diff-refresh")
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::RefreshCw)
                    .tooltip(conduit::i18n::text_static("git_diff.reload_diff"))
                    .accessibility_label(conduit::i18n::text_static("git_diff.reload_diff"))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
            )
            .into_any_element()
    }

    /// Captions above the two panes. Without them a one-sided change (an added
    /// or deleted file) leaves an empty half that reads like a single column.
    pub(super) fn split_titles(&self, cx: &App) -> AnyElement {
        if self.loading || self.error.is_some() || self.state.read(cx).mode() != DiffMode::Split {
            return div().into_any_element();
        }
        let files = self.state.read(cx).files();
        if files.is_empty() || files.iter().any(DiffFile::is_binary) {
            return div().into_any_element();
        }
        let (before, after) = self.versions();
        let caption = |text: String| {
            div()
                .flex_1()
                .min_w_0()
                .px(px(16.))
                .truncate()
                .text_size(ui_text_size(10.))
                .text_color(rgb(palette().faint))
                .child(text)
        };
        h_flex()
            .w_full()
            .flex_shrink_0()
            .h(px(26.))
            .items_center()
            .border_b_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(caption(before))
            .child(div().w(px(1.)).h(px(12.)).bg(rgb(palette().border)))
            .child(caption(after))
            .into_any_element()
    }

    pub(super) fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.loading && self.state.read(cx).files().is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(Spinner::new().small().color(rgb(palette().muted).into()))
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("git_diff.loading_diff")),
                )
                .into_any_element();
        }
        if let Some(error) = &self.error {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .px(px(48.))
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .size(px(22.))
                        .text_color(rgb(palette().danger_text)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .line_height(relative(1.5))
                        .text_align(gpui::TextAlign::Center)
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                )
                .child(
                    Button::new("git-diff-retry")
                        .label(conduit::i18n::text_static("git_diff.reload"))
                        .small()
                        .ghost()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                )
                .into_any_element();
        }
        let files = self.state.read(cx).files();
        if files.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static(
                            "git_diff.no_text_diff_to_display",
                        )),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::text_static(
                            "git_diff.the_file_may_have_been_restored_or",
                        )),
                )
                .into_any_element();
        }
        if files.iter().any(DiffFile::is_binary) {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .px(px(48.))
                .child(
                    Icon::new(FullIconName::File)
                        .size(px(24.))
                        .text_color(rgb(palette().muted)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().text))
                        .child(conduit::i18n::text_static("git_diff.binary_file_changed")),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .line_height(relative(1.5))
                        .text_align(gpui::TextAlign::Center)
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static(
                            "git_diff.this_window_shows_text_diffs_only_use",
                        )),
                )
                .into_any_element();
        }
        // Split columns are each half the viewport; a long line would push the
        // right column out of view, so wrap instead of scrolling horizontally.
        let split = self.state.read(cx).mode() == DiffMode::Split;
        let diff = Diff::new(&self.state)
            .header_visible(false)
            .line_number(true)
            .syntax_highlight(true)
            .change_indicator(DiffChangeIndicator::Bars)
            .hunk_separator(DiffHunkSeparator::LineInfo)
            .hover_highlight(DiffHoverHighlight::Line)
            .soft_wrap(split);
        self.review_diff(diff, cx).into_any_element()
    }

    pub(super) fn notes_banner(&self) -> AnyElement {
        v_flex()
            .w_full()
            .flex_shrink_0()
            .gap(px(2.))
            .px(px(16.))
            .py(px(6.))
            .border_b_1()
            .border_color(rgb(palette().warning_border))
            .bg(rgb(palette().warning_bg))
            .children(self.notes.iter().map(|note| {
                div()
                    .text_size(ui_text_size(10.))
                    .line_height(relative(1.4))
                    .text_color(rgb(palette().warning_text))
                    .child(note.clone())
            }))
            .into_any_element()
    }

    pub(super) fn footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let files = self.state.read(cx).files();
        let binary = files.iter().any(DiffFile::is_binary);
        h_flex()
            .w_full()
            .flex_shrink_0()
            .gap(px(12.))
            .items_center()
            .px(px(14.))
            .py(px(6.))
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .text_size(ui_text_size(9.))
                    .text_color(rgb(palette().faint))
                    .child(if binary {
                        conduit::i18n::text_static(
                            "git_diff.line_by_line_diffs_are_unavailable_for",
                        )
                    } else if self.comparison.is_some() {
                        conduit::i18n::text_static("git_diff.read_only_diff_compare_both_versions")
                    } else {
                        conduit::i18n::text_static("git_diff.read_only_diff_compare_with_head")
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(ui_text_size(9.))
                    .text_color(rgb(palette().faint))
                    .child(if self.embedded {
                        conduit::i18n::format(
                            "git_diff.next_change_previous_close_tab",
                            &[
                                ("arg0", shortcuts::hint("next_diff", cx).to_string()),
                                ("arg1", shortcuts::hint("previous_diff", cx).to_string()),
                                ("arg2", shortcuts::hint("close_diff", cx).to_string()),
                            ],
                        )
                    } else {
                        conduit::i18n::format(
                            "git_diff.next_change_previous_esc_close",
                            &[
                                ("arg0", shortcuts::hint("next_diff", cx).to_string()),
                                ("arg1", shortcuts::hint("previous_diff", cx).to_string()),
                            ],
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Drop for GitDiffDialog {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Render for GitDiffDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("git-diff-dialog")
            .debug_selector(|| "git-diff-dialog".into())
            .key_context("GitDiffDialog")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .bg(rgb(palette().surface))
            .when(!self.embedded, |view| view.rounded(px(16.)))
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &NextDiff, window, cx| {
                this.next_change(window, cx);
            }))
            .on_action(cx.listener(|this, _: &PreviousDiff, window, cx| {
                this.previous_change(window, cx);
            }))
            .child(self.header(cx))
            .child(self.toolbar(cx))
            .child(self.review_toolbar(cx))
            .when(!self.notes.is_empty(), |dialog| {
                dialog.child(self.notes_banner())
            })
            .child(self.split_titles(cx))
            .child(div().flex_1().min_h_0().w_full().child(self.body(cx)))
            .child(self.review_panel(window, cx))
            .child(self.footer(cx))
    }
}
