//! Persistent branch navigation with independently loaded history and commit details.
use super::*;
use gpui_kit::component::skeleton::Skeleton;
mod graph;
mod tree;

pub(super) struct GitLogView {
    app: WeakEntity<ConduitApp>,
    pub(super) tree: Entity<tree::BranchTree>,
    pub(super) branch_search: Entity<InputState>,
    text: Entity<InputState>,
    author: Entity<InputState>,
    path: Entity<InputState>,
    workspace: Option<PathBuf>,
    favorite_scope: Option<PathBuf>,
    active: bool,
    repository: Option<git::Repository>,
    interaction: Entity<git_branches::BranchMenuInteraction>,
    query: git::HistoryQuery,
    history: Option<git::History>,
    graph: Vec<graph::GraphRow>,
    graph_width: f32,
    selected: Option<String>,
    details: Option<git::CommitDetails>,
    loading: bool,
    details_loading: bool,
    error: Option<String>,
    details_error: Option<String>,
    log_cancel: Arc<AtomicBool>,
    detail_cancel: Arc<AtomicBool>,
    sequence: u64,
    collapsed: HashSet<String>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl GitLogView {
    pub(super) fn new(
        app: WeakEntity<ConduitApp>,
        workspace: Option<PathBuf>,
        repository: Option<git::Repository>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let branch_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("git_log.branch_or_tag"))
        });
        let text = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("git_log.commit_message_or_hash"))
        });
        let author = cx.new(|cx| {
            InputState::new(window, cx).placeholder(conduit::i18n::text_static("git_log.author"))
        });
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("git_log.file_or_directory_path"))
        });
        let interaction = cx.new(|_| git_branches::BranchMenuInteraction::default());
        let owner = cx.weak_entity();
        let tree = cx.new(|cx| tree::BranchTree::new(owner, cx));
        let mut subscriptions =
            vec![
                cx.subscribe(&branch_search, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }),
            ];
        for input in [&text, &author, &path] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query.text = this.text.read(cx).value().to_string();
                    this.query.author = this.author.read(cx).value().to_string();
                    this.query.path = this.path.read(cx).value().to_string();
                    this.query.limit = 200;
                    this.reload(true, cx);
                }
            }));
        }
        if let Some(app) = app.upgrade() {
            subscriptions.push(cx.observe(&app, |this, app, cx| {
                let app = app.read(cx);
                if !app.branch_navigation_open() {
                    this.active = false;
                    return;
                }
                let reopened = !this.active;
                this.active = true;
                let workspace = app.selected_workspace_path();
                let repository = app.git.repository.clone();
                let changed =
                    reopened || this.workspace != workspace || this.repository != repository;
                if this.workspace != workspace {
                    this.favorite_scope = workspace.as_deref().map(git::repository_scope);
                    this.query.reference = None;
                    this.history = None;
                    this.selected = None;
                    this.details = None;
                    this.detail_cancel.store(true, Ordering::Relaxed);
                    this.details_loading = false;
                    this.details_error = None;
                    this.graph.clear();
                    this.collapsed = HashSet::from(["git-tags-section".into()]);
                    this.interaction.update(cx, |state, cx| {
                        *state = Default::default();
                        cx.notify();
                    });
                }
                this.workspace = workspace;
                this.repository = repository;
                if this.query.reference.as_ref().is_some_and(|reference| {
                    reference.starts_with("refs/heads/") || reference.starts_with("refs/remotes/")
                }) && this.repository.as_ref().is_some_and(|repo| {
                    !repo
                        .branch_info
                        .iter()
                        .any(|branch| Some(&branch.full_ref) == this.query.reference.as_ref())
                }) {
                    this.query.reference = None;
                }
                if changed {
                    this.reload(false, cx);
                }
                cx.notify();
            }));
        }
        subscriptions.push(cx.observe(&interaction, |_, _, cx| cx.notify()));
        let favorite_scope = workspace.as_deref().map(git::repository_scope);
        let mut view = Self {
            app,
            tree,
            branch_search,
            text,
            author,
            path,
            workspace,
            favorite_scope,
            active: true,
            repository,
            interaction,
            query: Default::default(),
            history: None,
            graph: vec![],
            graph_width: 22.,
            selected: None,
            details: None,
            loading: false,
            details_loading: false,
            error: None,
            details_error: None,
            log_cancel: Arc::new(AtomicBool::new(false)),
            detail_cancel: Arc::new(AtomicBool::new(false)),
            sequence: 0,
            collapsed: HashSet::from(["git-tags-section".into()]),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        view.reload(false, cx);
        view
    }

    fn host(&self) -> git_branches::BranchMenuHost {
        git_branches::BranchMenuHost {
            workspace: self.workspace.clone().unwrap_or_default(),
            interaction: self.interaction.clone(),
            popover: None,
        }
    }

    fn reload(&mut self, debounce: bool, cx: &mut Context<Self>) {
        self.log_cancel.store(true, Ordering::Relaxed);
        self.log_cancel = Arc::new(AtomicBool::new(false));
        self.sequence += 1;
        let sequence = self.sequence;
        let Some(workspace) = self.workspace.clone() else {
            self.loading = false;
            self.history = None;
            self.error = None;
            cx.notify();
            return;
        };
        let cancel = self.log_cancel.clone();
        let ticket = cancel.clone();
        let query = self.query.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(180))
                    .await;
            }
            if ticket.load(Ordering::Relaxed) {
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { git::load_history(&workspace, &query, &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if sequence != this.sequence || ticket.load(Ordering::Relaxed) {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(history) => {
                        this.graph = graph::layout(&history.commits);
                        this.graph_width = this
                            .graph
                            .iter()
                            .map(|row| row.lanes)
                            .max()
                            .unwrap_or(1)
                            .min(12) as f32
                            * 12.
                            + 10.;
                        let next = this
                            .selected
                            .as_ref()
                            .filter(|oid| history.commits.iter().any(|c| &c.oid == *oid))
                            .cloned()
                            .or_else(|| history.commits.first().map(|c| c.oid.clone()));
                        this.history = Some(history);
                        if next != this.selected || this.details.is_none() {
                            this.select_commit(next, cx);
                        }
                    }
                    Err(error) => {
                        this.error = Some(format!("{error:#}"));
                        this.history = None;
                        this.select_commit(None, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_ref(&mut self, reference: Option<String>, cx: &mut Context<Self>) {
        if self.query.reference == reference {
            return;
        }
        self.detail_cancel.store(true, Ordering::Relaxed);
        self.details_loading = false;
        self.query.reference = reference;
        self.query.limit = 200;
        self.selected = None;
        self.details = None;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.reload(false, cx);
    }

    fn select_commit(&mut self, oid: Option<String>, cx: &mut Context<Self>) {
        self.detail_cancel.store(true, Ordering::Relaxed);
        self.detail_cancel = Arc::new(AtomicBool::new(false));
        self.selected = oid.clone();
        self.details = None;
        self.details_error = None;
        self.details_loading = false;
        let (Some(workspace), Some(oid)) = (self.workspace.clone(), oid) else {
            cx.notify();
            return;
        };
        let cancel = self.detail_cancel.clone();
        let ticket = cancel.clone();
        self.details_loading = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { git::load_commit_details(&workspace, &oid, &cancel) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if ticket.load(Ordering::Relaxed) {
                    return;
                }
                this.details_loading = false;
                match result {
                    Ok(details) => this.details = Some(details),
                    Err(error) => this.details_error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_file(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(workspace), Some(details)) = (self.workspace.clone(), &self.details) else {
            return;
        };
        let Some(file) = details.files.get(index).cloned() else {
            return;
        };
        let request = git::CommitDiffRequest {
            oid: details.oid.clone(),
            parent: details.parent.clone(),
        };
        let viewer = cx.new(|cx| GitDiffDialog::new_commit(workspace, file, request, window, cx));
        window.open_dialog(cx, move |dialog, window, _| {
            let viewport = window.viewport_size();
            let width = (f32::from(viewport.width) - 48.).clamp(320., 1360.);
            let height = (f32::from(viewport.height) - 72.).clamp(240., 880.);
            let margin = ((f32::from(viewport.height) - height) * 0.5).max(16.);
            let viewer = viewer.clone();
            dialog
                .w(px(width))
                .margin_top(px(margin))
                .p_0()
                .gap_0()
                .close_button(false)
                .content(move |content, _, _| {
                    content.child(div().w_full().h(px(height)).child(viewer.clone()))
                })
        });
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| {
                app.refresh_branches();
                cx.notify();
            });
        }
        self.reload(false, cx);
    }

    fn new_branch(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(app), Some(repository)) = (self.app.upgrade(), self.repository.clone()) else {
            return;
        };
        let selected = repository
            .branch_info
            .iter()
            .find(|b| Some(&b.full_ref) == repository.context.current_ref.as_ref())
            .cloned();
        let request = git::BranchRequest {
            expected: repository.context.clone(),
            selected,
            action: git::BranchAction::Create {
                name: String::new(),
                checkout: true,
            },
        };
        let host = self.host();
        app.update(cx, |_, cx| {
            host.open_simple_form(
                cx.entity(),
                repository,
                request,
                git_branches::FormKind::Create,
                window,
                cx,
            )
        });
    }

    fn fetch(&self, cx: &mut Context<Self>) {
        let (Some(app), Some(workspace), Some(repository)) = (
            self.app.upgrade(),
            self.workspace.clone(),
            self.repository.as_ref(),
        ) else {
            return;
        };
        let request = git::BranchRequest {
            expected: repository.context.clone(),
            selected: None,
            action: git::BranchAction::Fetch,
        };
        app.update(cx, |app, cx| {
            app.start_branch_operation(workspace, request, cx);
        });
    }

    fn commit_row(&self, index: usize, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let commit = &self.history.as_ref().unwrap().commits[index];
        let oid = commit.oid.clone();
        let copy = oid.clone();
        let selected = self.selected.as_ref() == Some(&oid);
        let timestamp = chrono::DateTime::from_timestamp(commit.timestamp, 0)
            .map(|d| d.with_timezone(&Local).format("%Y/%m/%d %H:%M").to_string())
            .unwrap_or_default();
        let refs = commit
            .refs
            .iter()
            .map(|reference| {
                reference
                    .replace("refs/heads/", "")
                    .replace("refs/remotes/", "")
                    .replace("refs/tags/", "")
            })
            .collect::<Vec<_>>()
            .join(", ");
        h_flex()
            .id(("git-log-commit", index))
            .debug_selector(move || format!("git-log-commit-{index}"))
            .w_full()
            .h(px(30.))
            .flex_shrink_0()
            .items_center()
            .min_w_0()
            .pr_2()
            .gap_2()
            .cursor_pointer()
            .when(selected, |row| row.bg(rgb(palette().surface_hover)))
            .hover(|row| row.bg(rgb(palette().surface_hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focus.focus(window, cx);
                this.select_commit(Some(oid.clone()), cx);
            }))
            .context_menu(move |menu, _, _| {
                let copy = copy.clone();
                menu.item(
                    PopupMenuItem::new(conduit::i18n::text_static("git_log.copy_commit_hash"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                )
            })
            .child(graph::render(self.graph[index].clone(), self.graph_width))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_text_size(12.))
                    .child(commit.subject.clone()),
            )
            .when(!refs.is_empty(), |row| {
                row.child(
                    div()
                        .max_w(px(if wide { 200. } else { 100. }))
                        .px_1()
                        .rounded(px(3.))
                        .bg(rgb(palette().surface_strong))
                        .text_color(rgb(palette().git_added_text))
                        .text_size(ui_text_size(10.))
                        .truncate()
                        .child(refs),
                )
            })
            .when(wide, |row| {
                row.child(
                    div()
                        .w(px(90.))
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .truncate()
                        .child(commit.author.clone()),
                )
                .child(
                    div()
                        .w(px(124.))
                        .text_size(ui_text_size(10.))
                        .text_color(rgb(palette().muted))
                        .child(timestamp),
                )
            })
            .into_any_element()
    }

    fn details_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = v_flex()
            .id("git-log-details")
            .debug_selector(|| "git-log-details".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(palette().sidebar_bg));
        if self.details_loading {
            return panel
                .child(Self::loading_placeholder(
                    "git-log-details-loading",
                    conduit::i18n::text_static("git_log.loading_commits"),
                ))
                .into_any_element();
        }
        if let Some(error) = &self.details_error {
            return panel
                .child(
                    div()
                        .p_3()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                )
                .into_any_element();
        }
        let Some(details) = &self.details else {
            return panel
                .child(
                    div()
                        .p_3()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::text_static(
                            "git_log.select_a_commit_to_view_details_and",
                        )),
                )
                .into_any_element();
        };
        let files = details.files.clone();
        let view = cx.entity();
        panel = panel
            .child(
                h_flex()
                    .h(px(34.))
                    .px_3()
                    .items_center()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::format(
                        "git_log.changed_files",
                        &[("arg0", format!("{}", files.len()))],
                    )),
            )
            .child(
                uniform_list("git-log-files", files.len(), move |range, _, cx| {
                    view.update(cx, |_, cx| {
                        range
                            .map(|index| {
                                let file = &files[index];
                                h_flex()
                                    .id(("git-log-file", index))
                                    .debug_selector(move || format!("git-log-file-{index}"))
                                    .w_full()
                                    .min_w_0()
                                    .h(px(28.))
                                    .flex_shrink_0()
                                    .items_center()
                                    .px_3()
                                    .gap_2()
                                    .cursor_pointer()
                                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_file(index, window, cx)
                                    }))
                                    .child(
                                        div()
                                            .w(px(12.))
                                            .flex_shrink_0()
                                            .text_size(ui_text_size(10.))
                                            .text_color(rgb(git_change_color(file.kind())))
                                            .child(file.index.to_string()),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(ui_text_size(12.))
                                            .text_color(rgb(git_change_color(file.kind())))
                                            .child(file.path.clone()),
                                    )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .debug_selector(|| "git-log-files".into())
                .flex_1()
                .w_full()
                .min_h(px(28.)),
            )
            .child(
                v_flex()
                    .id("git-log-message")
                    .debug_selector(|| "git-log-message".into())
                    .max_h(relative(0.5))
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex_shrink_1()
                    .p_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child({
                        let oid = details.oid.clone();
                        Button::new("git-log-copy-hash")
                            .ghost()
                            .small()
                            .flex_shrink_0()
                            .label(details.oid[..12].to_string())
                            .icon(FullIconName::Copy)
                            .tooltip(conduit::i18n::text_static("git_log.copy_full_commit_hash"))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(oid.clone()))
                            })
                    })
                    .children(
                        self.history
                            .as_ref()
                            .and_then(|h| h.commits.iter().find(|c| c.oid == details.oid))
                            .map(|commit| {
                                div()
                                    .flex_shrink_0()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "{} <{}>\n{}",
                                        commit.author,
                                        commit.email,
                                        chrono::DateTime::from_timestamp(commit.timestamp, 0)
                                            .map(|d| d
                                                .with_timezone(&Local)
                                                .format("%Y-%m-%d %H:%M:%S")
                                                .to_string())
                                            .unwrap_or_default()
                                    ))
                            }),
                    )
                    .when(details.merge, |pane| {
                        pane.child(
                            div()
                                .flex_shrink_0()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().faint))
                                .child(conduit::i18n::text_static(
                                    "git_log.merge_commit_changes_relative_to_the_first",
                                )),
                        )
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(ui_text_size(12.))
                            .child(details.message.clone()),
                    ),
            );
        panel.into_any_element()
    }

    fn loading_placeholder(id: &'static str, label: &'static str) -> impl IntoElement {
        v_flex()
            .id(id)
            .debug_selector(move || id.into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(Spinner::new().small())
                    .child(label),
            )
            .children((0..8).map(|index| {
                h_flex()
                    .id((id, index))
                    .h(px(30.))
                    .flex_shrink_0()
                    .items_center()
                    .gap_3()
                    .child(Skeleton::new().size(px(12.)).rounded_full().flex_shrink_0())
                    .child(
                        Skeleton::new()
                            .h(px(10.))
                            .w(relative([0.65, 0.45, 0.75, 0.55][index % 4]))
                            .rounded(px(3.)),
                    )
            }))
    }
}

impl Drop for GitLogView {
    fn drop(&mut self) {
        self.log_cancel.store(true, Ordering::Relaxed);
        self.detail_cancel.store(true, Ordering::Relaxed);
    }
}

impl Render for GitLogView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let wide = window.viewport_size().width >= px(1250.);
        let horizontal = window.viewport_size().width >= px(1100.);
        let view = cx.entity();
        let since = match self.query.since.as_deref() {
            Some("1 day ago") => conduit::i18n::text_static("git_log.last_day"),
            Some("1 week ago") => conduit::i18n::text_static("git_log.last_week"),
            Some("1 month ago") => conduit::i18n::text_static("git_log.last_month"),
            Some("1 year ago") => conduit::i18n::text_static("git_log.last_year"),
            _ => conduit::i18n::text_static("git_log.all_time"),
        };
        let busy = self.app.upgrade().is_none_or(|app| {
            let app = app.read(cx);
            !app.writable
                || app.closing
                || app.git.operation.is_some()
                || app.git.generation.is_some()
                || self
                    .workspace
                    .as_ref()
                    .is_some_and(|path| app.worktree_has_runs(path))
        });
        let mut history = v_flex().flex_1().min_w_0().min_h_0().overflow_hidden();
        if let Some(error) = &self.error {
            history = history.child(
                div()
                    .p_3()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        } else if self.workspace.is_none() {
            history = history.child(div().p_3().child(conduit::i18n::text_static(
                "git_log.select_a_workspace_first",
            )));
        } else if self.loading
            && self
                .history
                .as_ref()
                .is_none_or(|log| log.commits.is_empty())
        {
            history = history.child(Self::loading_placeholder(
                "git-log-loading",
                conduit::i18n::text_static("git_log.loading_git_log"),
            ));
        } else if let Some(log) = &self.history {
            let length = log.commits.len();
            if length == 0 {
                history = history.child(
                    div()
                        .p_3()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("git_log.no_matching_commits")),
                );
            } else {
                let view = cx.entity();
                history = history.child(
                    uniform_list("git-log-commits", length, move |range, _, cx| {
                        view.update(cx, |this, cx| {
                            range
                                .map(|index| this.commit_row(index, wide, cx))
                                .collect::<Vec<_>>()
                        })
                    })
                    .debug_selector(|| "git-log-commits".into())
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0()
                    .w_full(),
                );
            }
            if log.has_more && self.query.limit < 5000 {
                history = history.child(
                    Button::new("git-log-more")
                        .ghost()
                        .small()
                        .flex_shrink_0()
                        .label(conduit::i18n::text_static("git_log.load_more_commits"))
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.query.limit += 200;
                            this.reload(false, cx);
                        })),
                )
            } else if log.has_more {
                history = history.child(
                    div()
                        .flex_shrink_0()
                        .p_2()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::text_static(
                            "git_log.showing_5_000_commits_use_filters_to",
                        )),
                );
            }
        }
        let reference = self
            .query
            .reference
            .as_deref()
            .map(|r| {
                r.trim_start_matches("refs/heads/")
                    .trim_start_matches("refs/remotes/")
                    .trim_start_matches("refs/tags/")
            })
            .unwrap_or(conduit::i18n::text_static("git_log.tree.all_branches"));
        v_flex()
            .id("git-log-panel")
            .debug_selector(|| "git-log-panel".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(rgb(palette().canvas))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.focus.is_focused(window) {
                    return;
                }
                let direction = match event.keystroke.key.as_str() {
                    "up" => -1,
                    "down" => 1,
                    _ => return,
                };
                if let Some(history) = &this.history
                    && !history.commits.is_empty()
                {
                    let current = this
                        .selected
                        .as_ref()
                        .and_then(|oid| history.commits.iter().position(|c| &c.oid == oid))
                        .unwrap_or(0);
                    let next = (current as isize + direction)
                        .clamp(0, history.commits.len() as isize - 1)
                        as usize;
                    let oid = history.commits[next].oid.clone();
                    this.select_commit(Some(oid), cx);
                    this.scroll.scroll_to_item(next, ScrollStrategy::Nearest);
                    cx.stop_propagation();
                }
            }))
            .child(
                sidebar::navigation_header()
                    .debug_selector(|| "git-log-header".into())
                    .px_3()
                    .gap_2()
                    .items_center()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .font_semibold()
                            .child(conduit::i18n::text_static("git_log.git_log")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(reference.to_string()),
                    )
                    .when(self.loading, |row| row.child(Spinner::new().small()))
                    .child(
                        Button::new("git-log-new-branch")
                            .debug_selector(|| "git-log-new-branch".into())
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(conduit::i18n::text_static("git_log.new_branch_from_head"))
                            .accessibility_label(conduit::i18n::text_static(
                                "git_branches.new_branch",
                            ))
                            .disabled(
                                busy || self
                                    .repository
                                    .as_ref()
                                    .is_none_or(|r| r.context.head.is_none()),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.new_branch(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("git-log-fetch")
                            .debug_selector(|| "git-log-fetch".into())
                            .ghost()
                            .small()
                            .icon(FullIconName::Download)
                            .tooltip(conduit::i18n::text_static("git_log.fetch_all_remotes"))
                            .accessibility_label(conduit::i18n::text_static(
                                "git_log.fetch_remote_branches",
                            ))
                            .disabled(
                                busy || self
                                    .repository
                                    .as_ref()
                                    .is_none_or(|r| r.remotes.is_empty()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
                    )
                    .child(
                        Button::new("git-branches-refresh")
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip(conduit::i18n::text_static(
                                "git_log.refresh_branches_and_log",
                            ))
                            .accessibility_label(conduit::i18n::text_static(
                                "git_log.refresh_branches_and_log",
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                h_flex()
                    .px_2()
                    .py_2()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(150.))
                            .debug_selector(|| "git-log-search".into())
                            .child(Input::new(&self.text).h(px(30.)).focus_bordered(false)),
                    )
                    .child(
                        div()
                            .w(px(104.))
                            .debug_selector(|| "git-log-author".into())
                            .child(Input::new(&self.author).h(px(30.)).focus_bordered(false)),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .debug_selector(|| "git-log-path".into())
                            .child(Input::new(&self.path).h(px(30.)).focus_bordered(false)),
                    )
                    .child(
                        Button::new("git-log-date")
                            .ghost()
                            .small()
                            .label(since)
                            .icon(IconName::ChevronDown)
                            .dropdown_menu(move |mut menu, _, _| {
                                for (label, value) in [
                                    (conduit::i18n::text_static("git_log.all_time"), None),
                                    (
                                        conduit::i18n::text_static("git_log.last_day"),
                                        Some("1 day ago"),
                                    ),
                                    (
                                        conduit::i18n::text_static("git_log.last_week"),
                                        Some("1 week ago"),
                                    ),
                                    (
                                        conduit::i18n::text_static("git_log.last_month"),
                                        Some("1 month ago"),
                                    ),
                                    (
                                        conduit::i18n::text_static("git_log.last_year"),
                                        Some("1 year ago"),
                                    ),
                                ] {
                                    let view = view.clone();
                                    menu = menu.item(PopupMenuItem::new(label).on_click(
                                        move |_, _, cx| {
                                            view.update(cx, |this, cx| {
                                                this.query.since = value.map(str::to_string);
                                                this.query.limit = 200;
                                                this.reload(false, cx);
                                            })
                                        },
                                    ));
                                }
                                menu
                            }),
                    ),
            )
            .child(if horizontal {
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(history)
                    .child(
                        div()
                            .w(px(300.))
                            .max_w(relative(0.45))
                            .h_full()
                            .flex_shrink_0()
                            .border_l_1()
                            .border_color(rgb(palette().border))
                            .child(self.details_pane(cx)),
                    )
                    .into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(history)
                    .child(
                        div()
                            .h(relative(0.45))
                            .min_h(px(132.))
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(self.details_pane(cx)),
                    )
                    .into_any_element()
            })
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

impl GitLogView {
    pub(super) fn refresh_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (input, key) in [
            (&self.branch_search, "git_log.branch_or_tag"),
            (&self.text, "git_log.commit_message_or_hash"),
            (&self.author, "git_log.author"),
            (&self.path, "git_log.file_or_directory_path"),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(conduit::i18n::text_static(key), window, cx)
            });
        }
        self.tree.update(cx, |_, cx| cx.notify());
        cx.notify();
    }
}
