//! Git sidebar, branch status, commit generation, and operations.

use super::*;

impl ConduitApp {
    pub(super) fn selected_workspace_path(&self) -> Option<PathBuf> {
        self.selected_workspace
            .and_then(|index| self.store.workspaces.get(index))
            .map(|workspace| workspace.path.clone())
    }

    pub(super) fn ensure_commit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.commit_drafts.contains_key(&path) {
            return;
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("填写 Commit Message…")
                .auto_grow(3, 3)
        });
        let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        self.commit_drafts.insert(
            path,
            CommitDraft {
                input,
                _subscription: subscription,
            },
        );
        cx.notify();
    }

    pub(super) fn git_feedback(
        &mut self,
        path: PathBuf,
        text: impl Into<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.git_feedback.insert(
            path,
            GitFeedback {
                text: text.into(),
                error,
            },
        );
        cx.notify();
    }

    pub(super) fn start_commit(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git_operation.is_some()
            || self.commit_generation.is_some()
        {
            return;
        }
        if self.worktree_has_runs(&path) {
            self.git_feedback(
                path,
                "请等待 CLI 任务结束后提交，避免暂存区同时变化。",
                true,
                cx,
            );
            return;
        }
        let Some(snapshot) = &self.git_snapshot else {
            return;
        };
        if self.git_probe.is_some() {
            return;
        }
        let message = self
            .commit_drafts
            .get(&path)
            .map(|draft| draft.input.read(cx).value().to_string())
            .unwrap_or_default();
        if message.trim().is_empty() {
            self.git_feedback(path, "请填写 Commit Message，或点击 AI 生成。", true, cx);
            return;
        }
        let operation = git::Operation::Commit {
            message: message.clone(),
            target: snapshot.target.clone(),
            push: self.commit_and_push,
        };
        self.git_operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: message,
        });
        self.git_feedback(
            path,
            if self.commit_and_push {
                "正在提交并推送…"
            } else {
                "正在提交…"
            },
            false,
            cx,
        );
    }

    pub(super) fn start_push(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git_operation.is_some()
            || self.commit_generation.is_some()
        {
            return;
        }
        if self.worktree_has_runs(&path) {
            self.git_feedback(path, "请等待 CLI 任务结束后推送。", true, cx);
            return;
        }
        let Some(repository) = &self.git_repository else {
            return;
        };
        let Some(upstream) = repository.upstream.clone() else {
            self.details = true;
            self.detail_view = DetailView::Git;
            self.refresh_git(cx);
            self.git_feedback(
                path,
                "当前分支没有有效上游。请打开分支菜单，选择 Push… 设置远程目标，或在 Tracked Branch 中选择上游。",
                true,
                cx,
            );
            return;
        };
        let operation = git::Operation::Push {
            branch: repository.current.clone(),
            upstream,
        };
        self.git_operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: String::new(),
        });
        self.git_feedback(path, "正在推送…", false, cx);
    }

    pub(super) fn push_button(&self, view: Entity<Self>) -> Option<AnyElement> {
        let repository = self
            .git_repository
            .as_ref()
            .filter(|repository| !repository.detached && repository.ahead > 0)?;
        let busy = self.git_operation.is_some();
        let disabled = self.closing
            || !self.writable
            || busy
            || repository.integration.is_some()
            || self.commit_generation.is_some()
            || self
                .selected_workspace_path()
                .is_some_and(|path| self.worktree_has_runs(&path));
        let tooltip = match &repository.upstream {
            Some(upstream) => format!("推送 {} 个提交到 {}", repository.ahead, upstream),
            None => format!("{} 个本地未发布提交；尚未设置上游", repository.ahead),
        };
        Some(
            Button::new("git-push-pending")
                .ghost()
                .small()
                .h(px(26.))
                .px_2()
                .tooltip(tooltip)
                .disabled(disabled)
                .loading(busy)
                .when(!disabled, |button| button.cursor_pointer())
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(gpui_kit::assets::IconName::ArrowUpFromLine).small())
                        .child(
                            div()
                                .min_w(px(16.))
                                .h(px(16.))
                                .px_1()
                                .rounded_full()
                                .bg(rgb(palette().surface_hover))
                                .text_size(px(10.))
                                .text_color(rgb(palette().text))
                                .child(repository.ahead.to_string()),
                        ),
                )
                .on_click(move |_, _, cx| view.update(cx, |this, cx| this.start_push(cx)))
                .into_any_element(),
        )
    }

    pub(super) fn generate_commit_message(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git_operation.is_some()
            || self.commit_generation.is_some()
        {
            return;
        }
        let Some(snapshot) = self
            .git_snapshot
            .as_ref()
            .filter(|snapshot| !snapshot.target.staged.is_empty())
        else {
            return;
        };
        // Git settings can pin the generation CLI & model; by default both
        // follow the currently selected session.
        let provider = self
            .store
            .git
            .generation_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let model = self.store.git.generation_model.clone().or_else(|| {
            self.selected_session
                .and_then(|id| self.session(id))
                .filter(|session| session.provider == provider)
                .and_then(|session| session.model.clone())
        });
        let executable = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(error) => {
                self.git_feedback(
                    path,
                    format!("无法生成 Commit Message：{error:#}"),
                    true,
                    cx,
                );
                return;
            }
        };
        let route = match self
            .selected_session
            .filter(|id| self.session(*id).is_some_and(|s| s.provider == provider))
        {
            Some(id) => self.session_route(id),
            None => Ok(RuntimeRoute::default()),
        };
        let route = match route {
            Ok(route) => route,
            Err(error) => {
                self.git_feedback(path, format!("生成 Provider 不可用：{error:#}"), true, cx);
                return;
            }
        };
        let original = self
            .commit_drafts
            .get(&path)
            .map(|draft| draft.input.read(cx).value().to_string())
            .unwrap_or_default();
        let files: Vec<_> = snapshot
            .files
            .iter()
            .filter(|file| file.staged())
            .map(|file| file.path.as_str())
            .collect();
        let prompt = self
            .store
            .git
            .generation_prompt(&files, &snapshot.staged_diff);
        self.commit_generation = Some(CommitGeneration {
            path: path.clone(),
            runtime: ChatRuntime::start_text_with_route(provider, executable, prompt, model, route),
            started: Instant::now(),
            original,
            parts: vec![],
            error: None,
        });
        self.git_feedback(
            path,
            format!("正在使用 {} 生成 Commit Message…", provider.label()),
            false,
            cx,
        );
    }

    pub(super) fn poll_git_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_commit_draft(window, cx);
        let completed = self.git_operation.as_ref().and_then(|operation| {
            match operation.probe.events.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Git 操作线程已退出，请刷新仓库状态确认结果，避免重复提交。".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            }
        });
        if let Some(result) = completed {
            let operation = self.git_operation.take().unwrap();
            let path = operation.probe.path.clone();
            match result {
                Ok(result) => {
                    if let Some(created) = &result.worktree_created
                        && !self
                            .store
                            .workspaces
                            .iter()
                            .any(|workspace| &workspace.path == created)
                    {
                        self.store.workspaces.push(Workspace {
                            path: created.clone(),
                            sessions: vec![],
                        });
                        self.persist();
                    }
                    if result.error.is_some()
                        && self.selected_workspace_path().as_ref() == Some(&path)
                    {
                        self.details = true;
                        self.detail_view = DetailView::Git;
                    }
                    if result.committed
                        && let Some(draft) = self.commit_drafts.get(&path)
                        && draft.input.read(cx).value().as_ref() == operation.draft
                    {
                        draft
                            .input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    let text = match &result.error {
                        Some(error) => format!("{}：{}", result.message, error),
                        None => result.message,
                    };
                    self.git_feedback(path.clone(), text, result.error.is_some(), cx);
                }
                Err(error) => self.git_feedback(path.clone(), error, true, cx),
            }
            if self.selected_workspace_path().as_ref() == Some(&path) {
                self.branch_probe = None;
                self.refresh_branches();
                self.refresh_git(cx);
            }
        }
        let mut generation_result = None;
        if let Some(generation) = &mut self.commit_generation {
            if generation.started.elapsed() > Duration::from_secs(120) && generation.error.is_none()
            {
                generation.error = Some("AI 生成超时，请重试。".into());
                generation.runtime.cancel();
            }
            for _ in 0..1000 {
                let event = match generation.runtime.events.try_recv() {
                    Ok(event) => event,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        generation_result = Some(Some("AI 工作线程已退出，请重试。".into()));
                        break;
                    }
                };
                match event {
                    Event::Text { key, text } => {
                        update_commit_text(&mut generation.parts, key, text, true)
                    }
                    Event::Delta { key, text } => {
                        update_commit_text(&mut generation.parts, key, text, false)
                    }
                    Event::Approval(approval) => {
                        generation.runtime.decide(approval.id, false);
                        generation.error =
                            Some("AI 请求了工具权限，已停止；请重试生成纯文本说明。".into());
                        generation.runtime.cancel();
                    }
                    Event::Tool { .. } | Event::ToolInput { .. } => {
                        generation.error = Some("AI 尝试调用工具，已停止生成。".into());
                        generation.runtime.cancel();
                    }
                    Event::Finished { error, cancelled } => {
                        generation_result = Some(
                            generation
                                .error
                                .clone()
                                .or(error)
                                .or_else(|| cancelled.then(|| "已取消生成。".into())),
                        );
                        break;
                    }
                    _ => {}
                }
                if generation
                    .parts
                    .iter()
                    .map(|(_, text)| text.len())
                    .sum::<usize>()
                    > 16 * 1024
                {
                    generation.error = Some("AI 返回的 Commit Message 过长，已停止。".into());
                    generation.runtime.cancel();
                }
            }
        }
        if let Some(error) = generation_result {
            let generation = self.commit_generation.take().unwrap();
            let text = generation
                .parts
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("\n\n");
            if let Some(error) = error {
                self.git_feedback(generation.path, error, true, cx);
            } else if text.trim().is_empty() {
                self.git_feedback(
                    generation.path,
                    "AI 没有返回 Commit Message，请重试。",
                    true,
                    cx,
                );
            } else if let Some(draft) = self.commit_drafts.get(&generation.path) {
                if draft.input.read(cx).value().as_ref() == generation.original {
                    draft
                        .input
                        .update(cx, |input, cx| input.set_value(text.trim(), window, cx));
                    self.git_feedback(
                        generation.path,
                        "Commit Message 已生成，可编辑后提交。",
                        false,
                        cx,
                    );
                } else {
                    self.git_feedback(
                        generation.path,
                        "草稿已修改，未覆盖；可重新生成。",
                        true,
                        cx,
                    );
                }
            }
        }
    }

    pub(super) fn refresh_branches(&mut self) {
        if self.closing || self.branch_probe.is_some() {
            return;
        }
        self.last_branch_check = Instant::now();
        self.branch_probe = self.selected_workspace_path().map(BranchProbe::start);
    }

    pub(super) fn commit_editor(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let Some(path) = self.selected_workspace_path() else {
            return div().into_any_element();
        };
        let Some(draft) = self.commit_drafts.get(&path) else {
            return div().into_any_element();
        };
        let generating = self
            .commit_generation
            .as_ref()
            .is_some_and(|generation| generation.path == path);
        let busy = self.git_operation.is_some() || self.commit_generation.is_some();
        let staged = self.git_snapshot.as_ref().is_some_and(|snapshot| {
            !snapshot.target.staged.is_empty() && snapshot.target.branch.is_some()
        });
        let staged_count = self
            .git_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.staged_count());
        let disabled = self.closing
            || !self.writable
            || busy
            || self.worktree_has_runs(&path)
            || self.git_probe.is_some()
            || self
                .git_repository
                .as_ref()
                .is_some_and(|repository| repository.integration.is_some())
            || !staged;
        let can_commit = !disabled && !draft.input.read(cx).value().trim().is_empty();
        let can_generate = !disabled;
        let generate_app = view.clone();
        let commit_app = view.clone();
        let cancel_app = view.clone();
        let action_label = if self.commit_and_push {
            "Commit and Push"
        } else {
            "Commit"
        };
        let dropdown = Popover::new("git-commit-mode")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(
                Button::new("git-commit-mode-trigger")
                    .ghost()
                    .small()
                    .compact()
                    .h(px(26.))
                    .icon(IconName::ChevronDown)
                    .tooltip("选择提交操作")
                    .disabled(busy || self.closing)
                    .when(!busy && !self.closing, |button| button.cursor_pointer()),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(180.)).gap_1();
                for push in [false, true] {
                    let app = view.clone();
                    let popover = popover.clone();
                    list = list.child(
                        Button::new(if push {
                            "git-mode-commit-push"
                        } else {
                            "git-mode-commit"
                        })
                        .accessibility_label(if push { "Commit and Push" } else { "Commit" })
                        .small()
                        .ghost()
                        .w_full()
                        .child(div().w_full().text_left().child(if push {
                            "Commit and Push"
                        } else {
                            "Commit"
                        }))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.commit_and_push = push;
                                cx.notify();
                            });
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |state, cx| state.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list
            });
        let mut editor = v_flex()
            .w_full()
            .flex_shrink_0()
            .pt_3()
            .gap_2()
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .font_semibold()
                            .child("Commit Message"),
                    )
                    .when(staged_count > 0, |row| {
                        row.child(
                            h_flex()
                                .h(px(18.))
                                .px_2()
                                .items_center()
                                .rounded_full()
                                .bg(rgb(palette().surface_strong))
                                .text_size(px(10.))
                                .text_color(rgb(palette().muted))
                                .child(format!("已暂存 {staged_count}")),
                        )
                    })
                    .child(
                        Button::new("git-generate-message")
                            .ghost()
                            .small()
                            .compact()
                            .icon(gpui_kit::assets::IconName::Sparkles)
                            .tooltip("Generate Commit Message（使用当前引擎和模型）")
                            .disabled(!can_generate)
                            .loading(generating)
                            .when(can_generate, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                generate_app.update(cx, |this, cx| this.generate_commit_message(cx))
                            }),
                    )
                    .when(generating, |row| {
                        row.child(
                            Button::new("git-cancel-generate")
                                .ghost()
                                .small()
                                .compact()
                                .icon(gpui_kit::assets::IconName::X)
                                .tooltip("取消生成")
                                .cursor_pointer()
                                .on_click(move |_, _, cx| {
                                    cancel_app.update(cx, |this, cx| {
                                        if let Some(generation) = &this.commit_generation {
                                            generation.runtime.cancel();
                                        }
                                        cx.notify();
                                    })
                                }),
                        )
                    }),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .rounded(px(14.))
                    .bg(rgb(palette().surface))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div().px_1().child(
                            Textarea::new(&draft.input)
                                .appearance(false)
                                .bordered(false)
                                .h(px(96.))
                                .text_size(px(13.))
                                .aria_label("Git Commit Message")
                                .disabled(
                                    self.closing
                                        || !self.writable
                                        || generating
                                        || self
                                            .git_operation
                                            .as_ref()
                                            .is_some_and(|operation| operation.probe.path == path),
                                ),
                        ),
                    )
                    .child(
                        h_flex()
                            .px_1()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new("git-commit")
                                    .label(action_label)
                                    .primary()
                                    .small()
                                    .h(px(26.))
                                    .disabled(!can_commit)
                                    .loading(
                                        self.git_operation
                                            .as_ref()
                                            .is_some_and(|operation| operation.probe.path == path),
                                    )
                                    .when(can_commit, |button| button.cursor_pointer())
                                    .on_click(move |_, _, cx| {
                                        commit_app.update(cx, |this, cx| this.start_commit(cx))
                                    }),
                            )
                            .child(dropdown),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.4))
                    .text_color(rgb(palette().faint))
                    .child(if self.worktree_has_runs(&path) {
                        "该仓库的 CLI 任务运行中，结束后可提交"
                    } else if self
                        .git_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.target.branch.is_none())
                    {
                        "分离 HEAD：请先切回分支"
                    } else if self
                        .git_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.target.staged.is_empty())
                    {
                        "没有已暂存变更，请先暂存后刷新"
                    } else {
                        "仅提交已暂存变更"
                    }),
            );
        if let Some(feedback) = self.git_feedback.get(&path) {
            editor = editor.child(
                div()
                    .id("git-action-feedback")
                    .max_h(px(60.))
                    .overflow_y_scroll()
                    .text_size(px(11.))
                    .line_height(relative(1.4))
                    .text_color(rgb(if feedback.error {
                        palette().error_text
                    } else {
                        palette().muted
                    }))
                    .child(feedback.text.clone()),
            );
        }
        editor.into_any_element()
    }

    pub(super) fn sync_git_workspace(&mut self, cx: &mut Context<Self>) {
        let path = self.selected_workspace_path();
        if self.git_workspace == path {
            return;
        }
        self.git_workspace = path;
        self.branch_probe = None;
        self.git_repository = None;
        self.git_probe = None;
        self.git_snapshot = None;
        self.git_error = None;
        self.refresh_branches();
        if self.details && self.detail_view == DetailView::Git {
            self.refresh_git(cx);
        }
        cx.notify();
    }

    pub(super) fn poll_branches(&mut self, cx: &mut Context<Self>) {
        self.sync_git_workspace(cx);
        let update = self
            .branch_probe
            .as_ref()
            .and_then(|probe| match probe.events.try_recv() {
                Ok(result) => Some((probe.path.clone(), result.ok())),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some((probe.path.clone(), None))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        if let Some((path, repository)) = update {
            self.branch_probe = None;
            if self.selected_workspace_path().as_ref() == Some(&path)
                && self.git_repository != repository
            {
                let head_changed = self
                    .git_repository
                    .as_ref()
                    .map(|repo| (&repo.current, repo.detached))
                    != repository
                        .as_ref()
                        .map(|repo| (&repo.current, repo.detached));
                self.git_repository = repository;
                if head_changed
                    && self.details
                    && self.detail_view == DetailView::Git
                    && self.git_probe.is_none()
                {
                    self.refresh_git(cx);
                }
                cx.notify();
            }
        }
        // Pick up changes made in an external terminal, even with the dock closed.
        if self.last_branch_check.elapsed() >= Duration::from_secs(10) {
            self.refresh_branches();
        }
    }

    pub(super) fn branch_picker(&self, view: Entity<Self>) -> AnyElement {
        let Some(repository) = self.git_repository.clone() else {
            return div().into_any_element();
        };
        let label = if repository.detached {
            format!("HEAD · {}", repository.current)
        } else {
            repository.current.clone()
        };
        let trigger = Button::new("git-branch-trigger")
            .debug_selector(|| "git-branch-trigger".into())
            .ghost()
            .small()
            .h(px(26.))
            .px_2()
            .cursor_pointer()
            .tooltip(format!("当前分支：{label} · 分支操作"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(gpui_kit::assets::IconName::GitBranch).small())
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(11.))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new(SharedString::from(format!(
            "git-branch-picker-{}",
            self.git_workspace
                .as_deref()
                .map(display_path)
                .unwrap_or_default()
        )))
        .anchor(Anchor::BottomLeft)
        .offset(px(6.))
        // Nested menus handle their own outside clicks. Dismissing this parent
        // on a child's mouse-down would destroy the item before mouse-up.
        .overlay_closable(false)
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity().downgrade();
            let app = view.clone();
            let Some(repository) = app.read(cx).git_repository.clone() else {
                return div().into_any_element();
            };
            let host = git_branches::BranchMenuHost {
                workspace: app.read(cx).selected_workspace_path().unwrap_or_default(),
                interaction: window.use_keyed_state("git-branch-menu-interaction", cx, |_, _| {
                    git_branches::BranchMenuInteraction::default()
                }),
                popover: popover.clone(),
            };
            let outside_host = host.clone();
            let mut list = v_flex()
                .id("git-branches")
                .on_mouse_down_out(move |event, window, cx| {
                    outside_host.dismiss_unless_menu_open(event.position, window, cx)
                })
                .w(px(280.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .gap_1()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child("工作区分支"),
                );
            if let Some(banner) = app.read(cx).git_integration_banner(app.clone()) {
                list = list.child(banner);
            }
            if repository.detached {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
                        .child(format!("分离 HEAD · {}", repository.current)),
                );
            }
            for remote in [false, true] {
                if !repository
                    .branches
                    .iter()
                    .any(|branch| branch.remote == remote)
                {
                    continue;
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(if remote {
                            "远程分支（本地记录）"
                        } else {
                            "本地分支"
                        }),
                );
                for (index, branch) in repository
                    .branches
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b.remote == remote)
                {
                    let current =
                        !repository.detached && !remote && branch.name == repository.current;
                    let row = h_flex()
                        .id(("git-branch-row", index))
                        .debug_selector(move || format!("git-branch-row-{index}"))
                        .px_2()
                        .py_1()
                        .gap(px(4.))
                        .rounded(px(4.))
                        .child(h_flex().min_w_0().child(git_branches::branch_tag(
                            branch.name.clone(),
                            218.,
                            current,
                        )));
                    let info = repository
                        .branch_info
                        .iter()
                        .find(|info| info.name == branch.name && info.remote == remote)
                        .cloned();
                    if let Some(info) = info {
                        let menu_app = app.clone();
                        let button_app = app.clone();
                        let menu_info = info.clone();
                        let menu_host = host.clone();
                        let button_host = host.clone();
                        let bounds_host = host.clone();
                        let bounds_ref = info.full_ref.clone();
                        list = list.child(
                            row.on_prepaint(move |bounds, _, cx| {
                                bounds_host.record_branch_bounds(bounds_ref.clone(), bounds, cx)
                            })
                            .context_menu(move |menu, window, cx| {
                                git_branches::branch_menu(
                                    menu,
                                    menu_app.clone(),
                                    menu_info.clone(),
                                    menu_host.clone(),
                                    window,
                                    cx,
                                )
                            })
                            .child(
                                Button::new(("git-branch-actions", index))
                                    .debug_selector(move || format!("git-branch-actions-{index}"))
                                    .ghost()
                                    .small()
                                    .compact()
                                    .icon(FullIconName::Ellipsis)
                                    .tooltip(format!("{} 的分支操作", info.name))
                                    .accessibility_label(format!("{} 的分支操作", info.name))
                                    .dropdown_menu(move |menu, window, cx| {
                                        git_branches::branch_menu(
                                            menu,
                                            button_app.clone(),
                                            info.clone(),
                                            button_host.clone(),
                                            window,
                                            cx,
                                        )
                                    }),
                            ),
                        );
                    } else {
                        list = list.child(row);
                    }
                }
            }
            if repository.truncated {
                list = list.child(
                    div()
                        .px_2()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child("分支过多，列表已截断"),
                );
            }
            if let Some(feedback) = app.read(cx).git_feedback.get(&host.workspace) {
                list = list.child(
                    div()
                        .id("git-branch-feedback")
                        .max_h(px(64.))
                        .overflow_y_scroll()
                        .px_2()
                        .text_size(px(11.))
                        .text_color(rgb(if feedback.error {
                            palette().danger_text
                        } else {
                            palette().muted
                        }))
                        .child(feedback.text.clone()),
                );
            }
            let side_form = host.side_form(cx);
            div()
                .child(
                    list.child(
                        h_flex()
                            .pt_2()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(
                                Button::new("git-branches-refresh")
                                    .label("刷新分支")
                                    .small()
                                    .ghost()
                                    .cursor_pointer()
                                    .on_click(move |_, window, cx| {
                                        app.update(cx, |this, _| this.refresh_branches());
                                        if let Some(popover) = popover.clone().upgrade() {
                                            popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        }
                                    }),
                            ),
                    ),
                )
                .children(side_form)
                .into_any_element()
        })
        .into_any_element()
    }

    /// Open the read-only diff window for one file of the selected workspace.
    pub(super) fn open_git_diff(
        &mut self,
        file: git::ChangedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.selected_workspace_path() else {
            self.message = Some("请先选择工作区再查看差异".into());
            cx.notify();
            return;
        };
        let view = cx.new(|cx| GitDiffDialog::new(workspace, file, window, cx));
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let viewport = window.viewport_size();
            let width = (f32::from(viewport.width) - 96.).clamp(720., 1180.);
            let height = (f32::from(viewport.height) - 96.).clamp(420., 820.);
            let margin_top = ((f32::from(viewport.height) - height) * 0.5).max(24.);
            dialog
                .entrance(gpui_kit::component::dialog::DialogEntrance::Fade)
                .w(px(width))
                .margin_top(px(margin_top))
                .p_0()
                .gap(px(0.))
                .rounded(px(16.))
                .bg(rgb(palette().surface))
                .border_color(rgb(palette().border))
                .close_button(false)
                .overlay_closable(true)
                .content(move |content, _, _| {
                    content.child(
                        div()
                            .key_context("GitDiffDialog")
                            .w_full()
                            .h(px(height))
                            .child(view.clone()),
                    )
                })
        });
    }

    /// Start a background read-only Git snapshot for the selected workspace.
    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.last_branch_check.elapsed() >= Duration::from_secs(1) {
            self.refresh_branches();
        }
        self.git_probe = None;
        self.git_snapshot = None;
        let Some(path) = self.selected_workspace_path() else {
            self.git_error = Some("未选择工作区".into());
            cx.notify();
            return;
        };
        self.git_error = None;
        self.git_probe = Some(GitProbe::start(path));
        cx.notify();
    }

    /// Collect a finished Git probe; stale snapshots for another workspace are
    /// dropped so switching sessions cannot show the previous directory's diff.
    pub(super) fn poll_git(&mut self, cx: &mut Context<Self>) {
        let update = self
            .git_probe
            .as_ref()
            .and_then(|probe| match probe.events.try_recv() {
                Ok(result) => Some((probe.path.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    probe.path.clone(),
                    Err("Git 读取线程已退出，请重试".to_string()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        let Some((path, result)) = update else {
            return;
        };
        self.git_probe = None;
        if self.selected_workspace_path().as_deref() != Some(path.as_path()) {
            return;
        }
        match result {
            Ok(snapshot) => {
                self.git_snapshot = Some(snapshot);
                self.git_error = None;
            }
            Err(error) => {
                self.git_snapshot = None;
                self.git_error = Some(error);
            }
        }
        cx.notify();
    }

    /// Read-only Git view for the right-side detail panel.
    pub(super) fn git_panel(&self, view: Entity<Self>) -> AnyElement {
        let refresh_icon = |id: &'static str| -> AnyElement {
            let app = view.clone();
            Button::new(id)
                .ghost()
                .small()
                .compact()
                .icon(IconName::RefreshCw)
                .accessibility_label("刷新 Git 状态")
                .tooltip("刷新 Git 状态")
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    app.update(cx, |this, cx| this.refresh_git(cx));
                })
                .into_any_element()
        };
        let retry = |id: &'static str| -> AnyElement {
            let app = view.clone();
            Button::new(id)
                .label("重新读取")
                .small()
                .ghost()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    app.update(cx, |this, cx| this.refresh_git(cx));
                })
                .into_any_element()
        };
        let mut body = v_flex()
            .id("git-panel-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3();
        if self.git_probe.is_some() {
            return body
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .py_2()
                        .child(Spinner::new().small().color(rgb(palette().muted).into()))
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette().muted))
                                .child("正在读取工作区 Git 状态…"),
                        ),
                )
                .child(h_flex().child(retry("git-refresh-loading")))
                .into_any_element();
        }
        if let Some(error) = &self.git_error {
            return body
                .child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .gap_2()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(rgb(palette().danger_border))
                        .bg(rgb(palette().danger_bg))
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_semibold()
                                .text_color(rgb(palette().danger_text))
                                .child("无法读取 Git 状态"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .line_height(relative(1.5))
                                .text_color(rgb(palette().danger_text))
                                .child(error.clone()),
                        )
                        .child(h_flex().justify_end().child(retry("git-refresh-error"))),
                )
                .into_any_element();
        }
        let Some(snapshot) = &self.git_snapshot else {
            return body
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().faint))
                        .child("打开面板时读取工作区 Git 状态"),
                )
                .into_any_element();
        };
        body = body.child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            Icon::new(gpui_kit::assets::IconName::GitBranch)
                                .small()
                                .text_color(rgb(palette().faint)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .font_semibold()
                                .text_color(rgb(palette().text))
                                .child(snapshot.branch.clone()),
                        )
                        .child(refresh_icon("git-refresh")),
                )
                .when(!snapshot.is_clean(), |column| {
                    column.child(
                        div()
                            .pl(px(22.))
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "暂存 {} · 未暂存 {} · 未跟踪 {}",
                                snapshot.staged_count(),
                                snapshot.unstaged_count(),
                                snapshot.untracked_count()
                            )),
                    )
                }),
        );
        if let Some(banner) = self.git_integration_banner(view.clone()) {
            body = body.child(banner);
        }
        if snapshot.is_clean() {
            body = body.child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .py_10()
                    .child(
                        Icon::new(IconName::CircleCheck)
                            .with_size(px(20.))
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child("工作区没有未提交变更"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child("在终端修改文件后点击右上角刷新"),
                    ),
            );
        } else {
            let mut groups = v_flex().w_full().gap_2();
            for kind in git::ChangeKind::ALL {
                let files: Vec<_> = snapshot
                    .files
                    .iter()
                    .filter(|file| file.kind() == kind)
                    .collect();
                if files.is_empty() {
                    continue;
                }
                let color = git_change_color(kind);
                let collapsed = self.collapsed_git_groups.contains(&kind);
                let toggle_kind = kind;
                let app = view.clone();
                let mut group = v_flex().w_full().gap_1().child(
                    Button::new(SharedString::from(format!("git-group-{kind:?}")))
                        .ghost()
                        .small()
                        .compact()
                        .w_full()
                        .h(px(28.))
                        .px_1()
                        .cursor_pointer()
                        .tooltip(format!(
                            "点击{}变更",
                            if collapsed { "展开" } else { "收起" }
                        ))
                        .accessibility_label(format!(
                            "{}变更 {} 个，点击{}",
                            kind.label(),
                            files.len(),
                            if collapsed { "展开" } else { "收起" }
                        ))
                        .on_click(move |_, _, cx| {
                            app.update(cx, |this, cx| {
                                if !this.collapsed_git_groups.remove(&toggle_kind) {
                                    this.collapsed_git_groups.insert(toggle_kind);
                                }
                                cx.notify();
                            })
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(if collapsed {
                                        IconName::ChevronRight
                                    } else {
                                        IconName::ChevronDown
                                    })
                                    .small()
                                    .text_color(rgb(palette().faint)),
                                )
                                .child(
                                    div()
                                        .w(px(6.))
                                        .h(px(6.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(rgb(color)),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_semibold()
                                        .text_color(rgb(color))
                                        .child(kind.label()),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(files.len().to_string()),
                                ),
                        ),
                );
                if !collapsed {
                    let mut rows = v_flex().w_full().gap(px(1.)).pl(px(32.));
                    for file in files {
                        let app = view.clone();
                        let target = file.clone();
                        rows = rows.child(
                            h_flex()
                                .id(SharedString::from(format!("git-file-{}", file.path)))
                                .role(Role::Button)
                                .aria_label(format!("{}，查看只读差异", file.path))
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .items_center()
                                .h(px(28.))
                                .px_2()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(palette().surface_hover)))
                                .on_double_click(move |_, window, cx| {
                                    let target = target.clone();
                                    app.update(cx, |this, cx| {
                                        this.open_git_diff(target, window, cx);
                                    });
                                })
                                .child(
                                    Icon::new(file_ref::icon_for(Path::new(&file.path)))
                                        .size(px(14.))
                                        .text_color(rgb(palette().faint)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(13.))
                                        .text_color(rgb(palette().text))
                                        .child(file.path.clone()),
                                )
                                .when_some(file.original.clone(), |row, original| {
                                    row.child(
                                        div()
                                            .flex_shrink_0()
                                            .max_w(px(132.))
                                            .truncate()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette().faint))
                                            .child(format!("← {original}")),
                                    )
                                }),
                        );
                    }
                    group = group.child(rows);
                }
                groups = groups.child(group);
            }
            body = body.child(groups);
            if !snapshot.notes.is_empty() {
                body = body.child(v_flex().w_full().gap_1().pt_1().children(
                    snapshot.notes.iter().map(|note| {
                        div()
                            .text_size(px(10.))
                            .line_height(relative(1.4))
                            .text_color(rgb(palette().faint))
                            .child(note.clone())
                    }),
                ));
            }
        }
        body.into_any_element()
    }
}
