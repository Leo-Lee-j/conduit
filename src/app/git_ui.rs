//! Git sidebar, branch status, commit generation, and operations.

use super::*;
use gpui_kit::component::checkbox::Checkbox;

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

    pub(super) fn can_stage_files(&self) -> bool {
        self.writable
            && !self.closing
            && self.git_operation.is_none()
            && self.commit_generation.is_none()
            && self.git_probe.is_none()
            && self.git_snapshot.is_some()
            && self
                .selected_workspace_path()
                .is_some_and(|path| !self.worktree_has_runs(&path))
    }

    pub(super) fn stage_git_files(&mut self, selected: Vec<String>, cx: &mut Context<Self>) {
        if !self.can_stage_files() {
            return;
        }
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        let Some(snapshot) = &self.git_snapshot else {
            return;
        };
        let mut files = vec![];
        for file in &snapshot.files {
            if selected.contains(&file.path) && (file.unstaged() || file.untracked()) {
                files.push(file.path.clone());
                if matches!(file.worktree, 'R' | 'C')
                    && let Some(original) = &file.original
                {
                    files.push(original.clone());
                }
            }
        }
        files.sort();
        files.dedup();
        if files.is_empty() {
            self.git_feedback(path, "所选文件没有未暂存的修改。", true, cx);
            return;
        }
        let operation = git::Operation::Stage {
            files,
            target: snapshot.target.clone(),
        };
        self.git_operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: String::new(),
        });
        self.git_feedback(path, "正在暂存所选文件…", false, cx);
    }

    pub(super) fn unstage_git_files(&mut self, selected: Vec<String>, cx: &mut Context<Self>) {
        if !self.can_stage_files() {
            return;
        }
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        let Some(snapshot) = &self.git_snapshot else {
            return;
        };
        let mut files = vec![];
        for file in &snapshot.files {
            if selected.contains(&file.path) && file.staged() {
                files.push(file.path.clone());
                if let Some(original) = &file.original {
                    files.push(original.clone());
                }
            }
        }
        files.sort();
        files.dedup();
        if files.is_empty() {
            self.git_feedback(path, "所选文件没有已暂存的修改。", true, cx);
            return;
        }
        let operation = git::Operation::Unstage {
            files,
            target: snapshot.target.clone(),
        };
        self.git_operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: String::new(),
        });
        self.git_feedback(path, "正在将所选文件移出暂存区…", false, cx);
    }

    fn git_staging_actions(&self, snapshot: &git::Snapshot, view: Entity<Self>) -> AnyElement {
        let path = self.selected_workspace_path().unwrap_or_default();
        let available: Vec<String> = snapshot
            .files
            .iter()
            .filter(|f| f.unstaged() || f.untracked())
            .map(|f| f.path.clone())
            .collect();
        let selected: Vec<String> = available
            .iter()
            .filter(|p| {
                self.selected_git_files
                    .get(&path)
                    .is_some_and(|set| set.contains(*p))
            })
            .cloned()
            .collect();
        let staged: Vec<String> = snapshot
            .files
            .iter()
            .filter(|file| file.staged())
            .map(|file| file.path.clone())
            .collect();
        let selected_staged: Vec<String> = staged
            .iter()
            .filter(|file_path| {
                self.selected_git_files
                    .get(&path)
                    .is_some_and(|set| set.contains(*file_path))
            })
            .cloned()
            .collect();
        let select_app = view.clone();
        let selected_app = view.clone();
        let unstage_selected_app = view.clone();
        let all_app = view;
        let all_for_select = available.clone();
        let all = available.clone();
        let selected_for_unstage = selected_staged.clone();
        h_flex()
            .w_full()
            .gap_1()
            .flex_wrap()
            .child(
                Button::new("git-select-unstaged")
                    .ghost()
                    .small()
                    .label("全选")
                    .disabled(available.is_empty())
                    .on_click(move |_, _, cx| {
                        select_app.update(cx, |this, cx| {
                            this.selected_git_files
                                .insert(path.clone(), all_for_select.iter().cloned().collect());
                            cx.notify();
                        });
                    }),
            )
            .child(
                Button::new("git-stage-selected")
                    .ghost()
                    .small()
                    .label(format!("暂存选中 {}", selected.len()))
                    .disabled(!self.can_stage_files() || selected.is_empty())
                    .on_click(move |_, _, cx| {
                        selected_app
                            .update(cx, |this, cx| this.stage_git_files(selected.clone(), cx));
                    }),
            )
            .child(
                Button::new("git-unstage-selected")
                    .ghost()
                    .small()
                    .label(format!("移出暂存区选中 {}", selected_staged.len()))
                    .disabled(!self.can_stage_files() || selected_staged.is_empty())
                    .on_click(move |_, _, cx| {
                        unstage_selected_app.update(cx, |this, cx| {
                            this.unstage_git_files(selected_for_unstage.clone(), cx)
                        });
                    }),
            )
            .child(
                Button::new("git-stage-all")
                    .ghost()
                    .small()
                    .label(format!("全部暂存 {}", all.len()))
                    .disabled(!self.can_stage_files() || all.is_empty())
                    .on_click(move |_, _, cx| {
                        all_app.update(cx, |this, cx| this.stage_git_files(all.clone(), cx));
                    }),
            )
            .into_any_element()
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
            self.show_git_navigation(cx);
            self.git_feedback(
                path,
                "当前分支没有有效上游。请打开分支菜单，选择 Push 设置远程目标，或在 Tracked Branch 中选择上游。",
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
        if self.git_probe.is_some() {
            self.git_feedback(path, "正在刷新 Git 状态，请稍后再生成。", true, cx);
            return;
        }
        let Some(snapshot) = self.git_snapshot.as_ref() else {
            self.git_feedback(path, "请先刷新工作区 Git 状态。", true, cx);
            return;
        };
        if snapshot.target.staged.is_empty() {
            self.git_feedback(
                path,
                "请先选择修改文件并加入暂存区，再生成 Commit Message。",
                true,
                cx,
            );
            return;
        }
        // Git settings independently pin engine, provider and model; otherwise
        // follow the currently selected session.
        let (provider, model, route) = match self.generation_request(
            super::generation::GenerationKind::Git,
            self.selected_session,
        ) {
            Ok(selection) => selection,
            Err(error) => {
                self.git_feedback(path, format!("生成 Provider 不可用：{error:#}"), true, cx);
                return;
            }
        };
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
            runtime: ChatRuntime::start_text_with_route_in(
                provider,
                executable,
                prompt,
                model,
                route,
                path.clone(),
            ),
            started: Instant::now(),
            original,
            last_diagnostic: None,
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
                        self.show_git_navigation(cx);
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
                        generation_result = Some(Some(
                            generation
                                .error
                                .clone()
                                .or_else(|| generation.last_diagnostic.clone())
                                .unwrap_or_else(|| "AI 工作线程已退出，请重试。".into()),
                        ));
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
                    Event::ErrorBanner(error) => {
                        // A later successful reply may recover from this diagnostic.
                        generation.last_diagnostic = Some(error);
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
            let text = normalize_commit_message(&text);
            if error.as_deref() == Some("已取消生成。") {
                self.git_feedback(generation.path, "已取消生成，草稿未修改。", false, cx);
            } else if let Some(error) = error {
                self.git_feedback(
                    generation.path,
                    format!(
                        "生成失败：{}。{}",
                        failure_summary(&error),
                        failure_guidance(&error)
                    ),
                    true,
                    cx,
                );
            } else if text.trim().is_empty() {
                self.git_feedback(
                    generation.path,
                    generation.last_diagnostic.map(|error| format!("生成失败：{}。{}", failure_summary(&error), failure_guidance(&error))).unwrap_or_else(|| "AI 没有返回 Commit Message，请检查生成引擎、Provider 和模型配置后重试。".into()),
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
        let generate_tip = if !staged {
            "请先将需要提交的修改加入暂存区"
        } else if self.worktree_has_runs(&path) {
            "请等待此工作区的 CLI 任务结束"
        } else if busy {
            "Git 操作或 AI 生成正在进行"
        } else {
            "使用设置中的引擎、Provider 和模型生成 Commit Message"
        };
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
                            .accessibility_label("AI 生成提交信息")
                            .tooltip(generate_tip)
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
                                .accessibility_label("取消生成提交信息")
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
        if self.git_navigation_open() {
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
                if head_changed && self.git_navigation_open() && self.git_probe.is_none() {
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
        .p_1p5()
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
            let (local_expanded, remote_expanded) = {
                let interaction = host.interaction.read(cx);
                (interaction.local_expanded, interaction.remote_expanded)
            };
            let mut list = v_flex()
                .id("git-branches")
                .on_mouse_down_out(move |event, window, cx| {
                    outside_host.dismiss_unless_menu_open(event.position, window, cx)
                })
                .w(px(280.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .gap_0p5();
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
            let section_host = host.clone();
            let section_app = app.clone();
            let section_header =
                move |label: &'static str, remote: bool, expanded: bool| -> AnyElement {
                    let host = section_host.clone();
                    let app = section_app.clone();
                    h_flex()
                        .id(if remote {
                            "git-branches-remote-section"
                        } else {
                            "git-branches-local-section"
                        })
                        .debug_selector(move || {
                            if remote {
                                "git-branches-remote-section".to_string()
                            } else {
                                "git-branches-local-section".to_string()
                            }
                        })
                        .w_full()
                        .cursor_pointer()
                        .items_center()
                        .gap(px(2.))
                        .px_2()
                        .pt_2()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .hover(|style| style.text_color(rgb(palette().muted)))
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(10.))
                            .flex_shrink_0(),
                        )
                        .child(label)
                        .on_click(move |_, _, cx| {
                            host.interaction.update(cx, |state, _| {
                                if remote {
                                    state.remote_expanded = !state.remote_expanded;
                                } else {
                                    state.local_expanded = !state.local_expanded;
                                }
                            });
                            app.update(cx, |_, cx| cx.notify());
                        })
                        .into_any_element()
                };
            for remote in [false, true] {
                if !repository
                    .branches
                    .iter()
                    .any(|branch| branch.remote == remote)
                {
                    continue;
                }
                let expanded = if remote {
                    remote_expanded
                } else {
                    local_expanded
                };
                list = list.child(section_header(
                    if remote {
                        "远程分支（本地记录）"
                    } else {
                        "本地分支"
                    },
                    remote,
                    expanded,
                ));
                if !expanded {
                    continue;
                }
                for (index, branch) in repository
                    .branches
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b.remote == remote)
                {
                    let current =
                        !repository.detached && !remote && branch.name == repository.current;
                    let group = SharedString::from(format!("git-branch-row-{index}"));
                    let row = h_flex()
                        .id(("git-branch-row", index))
                        .debug_selector(move || format!("git-branch-row-{index}"))
                        .group(group.clone())
                        .min_w_0()
                        .px_2()
                        .py_1();
                    let info = repository
                        .branch_info
                        .iter()
                        .find(|info| info.name == branch.name && info.remote == remote)
                        .cloned();
                    let action = info.as_ref().map(|info| {
                        let button_app = app.clone();
                        let button_host = host.clone();
                        let button_info = info.clone();
                        let name = button_info.name.clone();
                        Button::new(("git-branch-actions", index))
                            .debug_selector(move || format!("git-branch-actions-{index}"))
                            .ghost()
                            .small()
                            .compact()
                            .icon(FullIconName::Ellipsis)
                            .tooltip(format!("{name} 的分支操作"))
                            .accessibility_label(format!("{name} 的分支操作"))
                            .dropdown_menu(move |menu, window, cx| {
                                git_branches::branch_menu(
                                    menu,
                                    button_app.clone(),
                                    button_info.clone(),
                                    button_host.clone(),
                                    window,
                                    cx,
                                )
                            })
                            .into_any_element()
                    });
                    let tag =
                        git_branches::branch_row_tag(branch.name.clone(), current, group, action);
                    if let Some(info) = info {
                        let menu_app = app.clone();
                        let menu_host = host.clone();
                        let bounds_host = host.clone();
                        let bounds_ref = info.full_ref.clone();
                        let row = row
                            .on_prepaint(move |bounds, _, cx| {
                                bounds_host.record_branch_bounds(bounds_ref.clone(), bounds, cx)
                            })
                            .context_menu(move |menu, window, cx| {
                                git_branches::branch_menu(
                                    menu,
                                    menu_app.clone(),
                                    info.clone(),
                                    menu_host.clone(),
                                    window,
                                    cx,
                                )
                            });
                        list = list.child(row.child(tag));
                    } else {
                        list = list.child(row.child(tag));
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
        self.open_diff_document(workspace, file, window, cx);
    }

    /// Start a background read-only Git snapshot for the selected workspace.
    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.last_branch_check.elapsed() >= Duration::from_secs(1) {
            self.refresh_branches();
        }
        self.mark_git_refresh();
        self.git_probe = None;
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
    pub(super) fn poll_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                if let Some(selected) = self.selected_git_files.get_mut(&path) {
                    selected.retain(|p| {
                        snapshot
                            .files
                            .iter()
                            .any(|f| f.path == *p && (f.staged() || f.unstaged() || f.untracked()))
                    });
                }
                self.refresh_diff_documents(&path, &snapshot, window, cx);
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

    /// Git status and staging list in the contextual left navigation.
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
        if self.git_probe.is_some() && self.git_snapshot.is_none() {
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
                .p_3()
                .gap_1()
                .rounded(px(10.))
                .border_1()
                .border_color(rgb(palette().border))
                .bg(rgb(palette().surface))
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
        if !snapshot.is_clean() {
            body = body.child(self.git_staging_actions(snapshot, view.clone()));
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
                                        .text_color(rgb(palette().text))
                                        .child(kind.label()),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .px_2()
                                        .rounded_full()
                                        .bg(rgb(palette().surface_strong))
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().muted))
                                        .child(files.len().to_string()),
                                ),
                        ),
                );
                if !collapsed {
                    let mut rows = v_flex().w_full().gap(px(1.)).pl(px(16.));
                    for file in files {
                        let app = view.clone();
                        let target = file.clone();
                        let can_stage =
                            self.can_stage_files() && (file.unstaged() || file.untracked());
                        let can_unstage = self.can_stage_files() && file.staged();
                        let workspace = self.selected_workspace_path().unwrap_or_default();
                        let checked = self
                            .selected_git_files
                            .get(&workspace)
                            .is_some_and(|set| set.contains(&file.path));
                        let select_app = view.clone();
                        let select_path = file.path.clone();
                        let menu_app = view.clone();
                        let menu_file = file.clone();
                        let menu_workspace = workspace.clone();
                        let diff_workspace = workspace.clone();
                        rows = rows.child(
                            h_flex()
                                .id(SharedString::from(format!("git-file-{}", file.path)))
                                .role(Role::Button)
                                .debug_selector({
                                    let path = file.path.clone();
                                    move || format!("git-change-file-{path}")
                                })
                                .aria_label(format!("{}，双击查看差异标签，右键操作", file.path))
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
                                    let workspace = diff_workspace.clone();
                                    app.update(cx, |this, cx| {
                                        if this.selected_workspace_path().as_ref()
                                            == Some(&workspace)
                                        {
                                            this.open_git_diff(target, window, cx);
                                        }
                                    });
                                })
                                .context_menu(move |menu, _, cx| {
                                    let single_app = menu_app.clone();
                                    let single = menu_file.path.clone();
                                    let single_workspace = menu_workspace.clone();
                                    let unstage_app = menu_app.clone();
                                    let unstage = menu_file.path.clone();
                                    let unstage_workspace = menu_workspace.clone();
                                    let batch_workspace = menu_workspace.clone();
                                    let batch_app = menu_app.clone();
                                    let batch: Vec<String> = menu_app
                                        .read(cx)
                                        .selected_git_files
                                        .get(&menu_workspace)
                                        .map(|set| set.iter().cloned().collect())
                                        .unwrap_or_default();
                                    let batch_staged: Vec<String> = menu_app
                                        .read(cx)
                                        .git_snapshot
                                        .as_ref()
                                        .map(|snapshot| {
                                            snapshot
                                                .files
                                                .iter()
                                                .filter(|file| {
                                                    file.staged() && batch.contains(&file.path)
                                                })
                                                .map(|file| file.path.clone())
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    let batch_staged_app = menu_app.clone();
                                    let batch_staged_workspace = menu_workspace.clone();
                                    let disabled = !can_stage;
                                    menu.item(
                                        PopupMenuItem::new("加入暂存区")
                                            .disabled(disabled)
                                            .on_click(move |_, _, cx| {
                                                single_app.update(cx, |this, cx| {
                                                    if this.selected_workspace_path().as_ref()
                                                        == Some(&single_workspace)
                                                    {
                                                        this.stage_git_files(
                                                            vec![single.clone()],
                                                            cx,
                                                        );
                                                    }
                                                });
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new(format!(
                                            "暂存选中文件 ({})",
                                            batch.len()
                                        ))
                                        .disabled(
                                            batch.is_empty()
                                                || !menu_app.read(cx).can_stage_files(),
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                batch_app.update(cx, |this, cx| {
                                                    if this.selected_workspace_path().as_ref()
                                                        == Some(&batch_workspace)
                                                    {
                                                        this.stage_git_files(batch.clone(), cx);
                                                    }
                                                });
                                            },
                                        ),
                                    )
                                    .item(
                                        PopupMenuItem::new("移出暂存区")
                                            .disabled(!can_unstage)
                                            .on_click(move |_, _, cx| {
                                                unstage_app.update(cx, |this, cx| {
                                                    if this.selected_workspace_path().as_ref()
                                                        == Some(&unstage_workspace)
                                                    {
                                                        this.unstage_git_files(
                                                            vec![unstage.clone()],
                                                            cx,
                                                        );
                                                    }
                                                });
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new(format!(
                                            "移出暂存区选中 ({})",
                                            batch_staged.len()
                                        ))
                                        .disabled(
                                            batch_staged.is_empty()
                                                || !menu_app.read(cx).can_stage_files(),
                                        )
                                        .on_click(
                                            move |_, _, cx| {
                                                batch_staged_app.update(cx, |this, cx| {
                                                    if this.selected_workspace_path().as_ref()
                                                        == Some(&batch_staged_workspace)
                                                    {
                                                        this.unstage_git_files(
                                                            batch_staged.clone(),
                                                            cx,
                                                        );
                                                    }
                                                });
                                            },
                                        ),
                                    )
                                })
                                .child(
                                    Checkbox::new(SharedString::from(format!(
                                        "git-select-{}",
                                        file.path
                                    )))
                                    .checked(checked)
                                    .disabled(
                                        !(file.staged() || file.unstaged() || file.untracked()),
                                    )
                                    .accessibility_label(format!("选择 {}", file.path))
                                    .on_click(
                                        move |checked, _, cx| {
                                            select_app.update(cx, |this, cx| {
                                                let set = this
                                                    .selected_git_files
                                                    .entry(workspace.clone())
                                                    .or_default();
                                                if *checked {
                                                    set.insert(select_path.clone());
                                                } else {
                                                    set.remove(&select_path);
                                                }
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
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
                                .when(file.staged(), |row| {
                                    row.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette().highlight_text))
                                            .child("已暂存"),
                                    )
                                })
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

pub(super) fn normalize_commit_message(text: &str) -> String {
    let text = text.trim();
    let text = if text.starts_with("```") && text.ends_with("```") {
        text.split_once('\n')
            .map(|(_, body)| body.strip_suffix("```").unwrap_or(body).trim())
            .unwrap_or(text)
    } else {
        text
    };
    text.to_string()
}
