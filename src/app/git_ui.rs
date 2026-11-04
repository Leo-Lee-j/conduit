//! Git sidebar, branch status, commit generation, and operations.

use super::*;

pub(super) use super::git_state::{GitChangeGroup, GitUi};
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
        if self.git.drafts.contains_key(&path) {
            return;
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("填写 Commit Message…")
                .auto_grow(3, 3)
        });
        let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        let tag = cx.new(|cx| InputState::new(window, cx).placeholder("Tag（可选，例如 v1.2.3）"));
        let tag_subscription = cx.subscribe(&tag, |_, _, _: &InputEvent, cx| cx.notify());
        self.git.drafts.insert(
            path,
            CommitDraft {
                input,
                tag,
                _tag_subscription: tag_subscription,
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
        self.git.feedback.insert(
            path,
            GitFeedback {
                text: text.into(),
                error,
            },
        );
        cx.notify();
    }

    fn git_result_feedback(
        &mut self,
        path: PathBuf,
        text: impl Into<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        let text = text.into();
        let presentation = if self.git_navigation_open()
            && self.selected_workspace_path().as_ref() == Some(&path)
        {
            NoticePresentation::History
        } else {
            NoticePresentation::Toast
        };
        self.notify_user(
            Notice::new(
                NoticeSource::Git,
                if error {
                    NoticeLevel::Error
                } else {
                    NoticeLevel::Success
                },
                if error {
                    "Git 操作失败"
                } else {
                    "Git 操作完成"
                },
                text.clone(),
            )
            .target(NoticeTarget::Workspace(path.clone()))
            .presentation(presentation),
            cx,
        );
        self.git_feedback(path, text, error, cx);
    }

    pub(super) fn can_stage_files(&self) -> bool {
        self.writable
            && !self.closing
            && self.git.operation.is_none()
            && self.git.generation.is_none()
            && self.git.probe.is_none()
            && self.git.snapshot.is_some()
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
        let Some(snapshot) = &self.git.snapshot else {
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
        self.git.operation = Some(GitOperation {
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
        let Some(snapshot) = &self.git.snapshot else {
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
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: String::new(),
        });
        self.git_feedback(path, "正在将所选文件移出暂存区…", false, cx);
    }

    fn confirm_git_discard(
        &mut self,
        workspace: PathBuf,
        file: git::ChangedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_stage_files() || self.selected_workspace_path().as_ref() != Some(&workspace) {
            return;
        }
        let Some(snapshot) = self.git.snapshot.as_ref() else {
            return;
        };
        let target = snapshot.target.clone();
        let app = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            let workspace = workspace.clone();
            let file = file.clone();
            let target = target.clone();
            let description = format!(
                "回滚 {} 的全部修改（包括暂存区）？已跟踪文件将恢复到 HEAD，新增和未跟踪文件将被删除。此操作不可撤销。",
                file.path
            );
            dialog
                .title("回滚文件修改")
                .content(move |content, _, _| content.child(description.clone()))
                .on_ok(move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        if !this.can_stage_files()
                            || this.selected_workspace_path().as_ref() != Some(&workspace)
                        {
                            return false;
                        }
                        this.git.operation = Some(GitOperation {
                            worktree: git::worktree_root(&workspace),
                            probe: git::OperationProbe::start(workspace.clone(), git::Operation::Discard {
                                file: file.clone(),
                                target: target.clone(),
                            }),
                            draft: String::new(),
                        });
                        this.git_feedback(workspace.clone(), "正在回滚文件修改…", false, cx);
                        true
                    })
                })
        });
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
                self.git
                    .selected_files
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
                self.git
                    .selected_files
                    .get(&path)
                    .is_some_and(|set| set.contains(*file_path))
            })
            .cloned()
            .collect();
        let select_app = view.clone();
        let selected_app = view.clone();
        let unstage_selected_app = view.clone();
        let all_for_select: Vec<_> = snapshot
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect();
        let all_selected = all_for_select.iter().all(|file| {
            self.git
                .selected_files
                .get(&path)
                .is_some_and(|set| set.contains(file))
        });
        let selected_for_unstage = selected_staged.clone();
        h_flex()
            .w_full()
            .gap_1()
            .items_center()
            .h(px(30.))
            .child(
                Button::new("git-select-unstaged")
                    .debug_selector(|| "git-select-unstaged".into())
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::Check)
                    .accessibility_label("全选变更文件")
                    .tooltip(if all_selected {
                        "取消全选"
                    } else {
                        "全选变更文件"
                    })
                    .disabled(snapshot.is_clean())
                    .on_click(move |_, _, cx| {
                        select_app.update(cx, |this, cx| {
                            let set = this.git.selected_files.entry(path.clone()).or_default();
                            if all_selected {
                                set.clear();
                            } else {
                                set.extend(all_for_select.iter().cloned());
                            }
                            cx.notify();
                        });
                    }),
            )
            .child(
                Button::new("git-stage-selected")
                    .debug_selector(|| "git-stage-selected".into())
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::Plus)
                    .accessibility_label("暂存选中文件")
                    .tooltip(format!("暂存选中文件 ({})", selected.len()))
                    .disabled(!self.can_stage_files() || selected.is_empty())
                    .on_click(move |_, _, cx| {
                        selected_app
                            .update(cx, |this, cx| this.stage_git_files(selected.clone(), cx));
                    }),
            )
            .child(
                Button::new("git-unstage-selected")
                    .debug_selector(|| "git-unstage-selected".into())
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::Minus)
                    .accessibility_label("移出暂存区选中文件")
                    .tooltip(format!("移出暂存区选中文件 ({})", selected_staged.len()))
                    .disabled(!self.can_stage_files() || selected_staged.is_empty())
                    .on_click(move |_, _, cx| {
                        unstage_selected_app.update(cx, |this, cx| {
                            this.unstage_git_files(selected_for_unstage.clone(), cx)
                        });
                    }),
            )
            .into_any_element()
    }

    fn checked_staged_files(&self, path: &Path, snapshot: &git::Snapshot) -> Vec<String> {
        let selected = self.git.selected_files.get(path);
        snapshot
            .files
            .iter()
            .filter(|file| file.staged() && selected.is_some_and(|set| set.contains(&file.path)))
            .map(|file| file.path.clone())
            .collect()
    }

    fn start_commit(&mut self, push: bool, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git.operation.is_some()
            || self.git.generation.is_some()
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
        let Some(snapshot) = &self.git.snapshot else {
            return;
        };
        if self.git.probe.is_some() {
            return;
        }
        let selected = self.checked_staged_files(&path, snapshot);
        if selected.is_empty() {
            self.git_feedback(path, "请勾选需要提交的已暂存文件。", true, cx);
            return;
        }
        let mut files = selected.clone();
        for file in &snapshot.files {
            if selected.contains(&file.path) && file.index == 'R' {
                files.extend(file.original.iter().cloned());
            }
        }
        files.sort();
        files.dedup();
        let message = self
            .git
            .drafts
            .get(&path)
            .map(|draft| draft.input.read(cx).value().to_string())
            .unwrap_or_default();
        if message.trim().is_empty() {
            self.git_feedback(path, "请填写 Commit Message，或点击 AI 生成。", true, cx);
            return;
        }
        let tag = self
            .git
            .drafts
            .get(&path)
            .map(|draft| draft.tag.read(cx).value().to_string())
            .filter(|value| !value.is_empty());
        if let Some(name) = tag.as_deref()
            && self.git.remote_tags.get(&path).is_some_and(|remote| {
                snapshot.target.branch.as_ref() == Some(&remote.branch)
                    && remote.names.iter().any(|item| item == name)
            })
        {
            self.git_feedback(
                path,
                format!("上次查询显示远程已存在标签 {name}，请选择新标签"),
                true,
                cx,
            );
            return;
        }
        self.git.remote_tags_probe = None;
        let operation = git::Operation::Commit {
            files: Some(files),
            message: message.clone(),
            target: snapshot.target.clone(),
            push,
            tag,
        };
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: message,
        });
        self.git_feedback(
            path,
            if push {
                "正在提交并推送…"
            } else {
                "正在提交…"
            },
            false,
            cx,
        );
    }

    fn refresh_remote_tags(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.git.remote_tags_probe.is_some() {
            self.git.remote_tags_probe = None;
            self.git_feedback(path, "已取消远程标签查询", false, cx);
            return;
        }
        if self.closing || self.git.operation.is_some() {
            return;
        }
        self.git.local_tags.remove(&path);
        self.refresh_git(cx);
        if self
            .git
            .repository
            .as_ref()
            .is_some_and(|repo| repo.remotes.is_empty())
        {
            self.git_feedback(path, "正在刷新本地标签缓存…", false, cx);
            return;
        }
        self.git.remote_tags_checked.insert(path.clone());
        self.git.remote_tags_probe = Some(git::RemoteTagsProbe::start(path.clone()));
        self.git_feedback(path, "正在刷新标签缓存…", false, cx);
    }

    fn retry_tag(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git.operation.is_some()
            || self.git.generation.is_some()
            || self.worktree_has_runs(&path)
        {
            return;
        }
        let Some(retry) = self.git.pending_tag_retry.get(&path).cloned() else {
            return;
        };
        self.git.remote_tags_probe = None;
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), git::Operation::RetryTag(retry)),
            draft: String::new(),
        });
        self.git_feedback(path, "正在重试推送标签…", false, cx);
    }

    pub(super) fn start_push(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git.operation.is_some()
            || self.git.generation.is_some()
        {
            return;
        }
        if self.worktree_has_runs(&path) {
            self.git_feedback(path, "请等待 CLI 任务结束后推送。", true, cx);
            return;
        }
        let Some(repository) = &self.git.repository else {
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
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: String::new(),
        });
        self.git_feedback(path, "正在推送…", false, cx);
    }

    pub(super) fn push_button(&self, view: Entity<Self>) -> Option<AnyElement> {
        let repository = self
            .git
            .repository
            .as_ref()
            .filter(|repository| !repository.detached && repository.ahead > 0)?;
        let busy = self.git.operation.is_some();
        let disabled = self.closing
            || !self.writable
            || busy
            || repository.integration.is_some()
            || self.git.generation.is_some()
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
            || self.git.operation.is_some()
            || self.git.generation.is_some()
        {
            return;
        }
        if self.git.probe.is_some() {
            self.git_feedback(path, "正在刷新 Git 状态，请稍后再生成。", true, cx);
            return;
        }
        let Some(snapshot) = self.git.snapshot.as_ref() else {
            self.git_feedback(path, "请先刷新工作区 Git 状态。", true, cx);
            return;
        };
        let selected = self.checked_staged_files(&path, snapshot);
        if selected.is_empty() {
            self.git_feedback(
                path,
                "请先勾选需要提交的已暂存文件，再生成 Commit Message。",
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
            .git
            .drafts
            .get(&path)
            .map(|draft| draft.input.read(cx).value().to_string())
            .unwrap_or_default();
        let files: Vec<_> = selected.iter().map(String::as_str).collect();
        let patch = git::selected_staged_patch(snapshot, &selected);
        let prompt = self.store.git.generation_prompt(&files, &patch);
        self.git.generation = Some(CommitGeneration {
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
        if self.git_navigation_open()
            && !self.closing
            && self.git.operation.is_none()
            && self.git.remote_tags_probe.is_none()
            && self
                .git
                .repository
                .as_ref()
                .is_some_and(|repo| !repo.remotes.is_empty())
            && let Some(path) = self.selected_workspace_path()
            && !self.git.remote_tags.contains_key(&path)
            && self.git.remote_tags_checked.insert(path.clone())
        {
            self.git.remote_tags_probe = Some(git::RemoteTagsProbe::start(path));
        }
        let remote_result =
            self.git
                .remote_tags_probe
                .as_ref()
                .and_then(|probe| match probe.events.try_recv() {
                    Ok(result) => Some((probe.path.clone(), result)),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        Some((probe.path.clone(), Err("远程标签读取线程已退出".into())))
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                });
        if let Some((path, result)) = remote_result {
            self.git.remote_tags_probe = None;
            if self.selected_workspace_path().as_ref() == Some(&path) {
                match result {
                    Ok(tags) => {
                        let count = tags.names.len();
                        let remote = tags.remote.clone();
                        self.git.remote_tags.insert(path.clone(), tags);
                        self.git_feedback(
                            path,
                            format!("已读取 {remote} 的 {count} 个远程标签"),
                            false,
                            cx,
                        );
                    }
                    Err(error) => {
                        self.git_feedback(path, format!("读取远程标签失败：{error}"), true, cx)
                    }
                }
            }
        }
        let completed = self.git.operation.as_ref().and_then(|operation| {
            match operation.probe.events.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Git 操作线程已退出，请刷新仓库状态确认结果，避免重复提交。".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            }
        });
        if let Some(result) = completed {
            let operation = self.git.operation.take().unwrap();
            let path = operation.probe.path.clone();
            match result {
                Ok(result) => {
                    let tags_changed = result.retry_completed
                        || result.retry_tag.is_some()
                        || (result.committed
                            && self
                                .git
                                .drafts
                                .get(&path)
                                .is_some_and(|draft| !draft.tag.read(cx).value().is_empty()));
                    if tags_changed {
                        self.git.local_tags.remove(&path);
                        self.git.remote_tags.remove(&path);
                        self.git.remote_tags_checked.remove(&path);
                    }
                    if result.retry_completed {
                        self.git.pending_tag_retry.remove(&path);
                    }
                    if let Some(retry) = result.retry_tag.clone() {
                        self.git.pending_tag_retry.insert(path.clone(), retry);
                    }
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
                        && !self.branch_navigation_open()
                    {
                        self.show_git_navigation(cx);
                    }
                    if result.committed
                        && let Some(draft) = self.git.drafts.get(&path)
                        && draft.input.read(cx).value().as_ref() == operation.draft
                    {
                        draft
                            .input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        if result.error.is_none() {
                            draft
                                .tag
                                .update(cx, |tag, cx| tag.set_value("", window, cx));
                        }
                    }
                    let text = match &result.error {
                        Some(error) => format!("{}：{}", result.message, error),
                        None => result.message,
                    };
                    self.git_result_feedback(path.clone(), text, result.error.is_some(), cx);
                }
                Err(error) => self.git_result_feedback(path.clone(), error, true, cx),
            }
            if self.selected_workspace_path().as_ref() == Some(&path) {
                self.git.branch_probe = None;
                self.refresh_branches();
                self.refresh_git(cx);
            }
        }
        let mut generation_result = None;
        if let Some(generation) = &mut self.git.generation {
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
            let generation = self.git.generation.take().unwrap();
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
                self.git_result_feedback(
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
                self.git_result_feedback(
                    generation.path,
                    generation.last_diagnostic.map(|error| format!("生成失败：{}。{}", failure_summary(&error), failure_guidance(&error))).unwrap_or_else(|| "AI 没有返回 Commit Message，请检查生成引擎、Provider 和模型配置后重试。".into()),
                    true,
                    cx,
                );
            } else if let Some(draft) = self.git.drafts.get(&generation.path) {
                if draft.input.read(cx).value().as_ref() == generation.original {
                    draft
                        .input
                        .update(cx, |input, cx| input.set_value(text.trim(), window, cx));
                    self.git_result_feedback(
                        generation.path,
                        "Commit Message 已生成，可编辑后提交。",
                        false,
                        cx,
                    );
                } else {
                    self.git_result_feedback(
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
        if self.closing || self.git.branch_probe.is_some() {
            return;
        }
        self.git.last_branch_check = Instant::now();
        self.git.branch_probe = self.selected_workspace_path().map(BranchProbe::start);
    }

    pub(super) fn commit_editor(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let Some(path) = self.selected_workspace_path() else {
            return div().into_any_element();
        };
        let Some(draft) = self.git.drafts.get(&path) else {
            return div().into_any_element();
        };
        let generating = self
            .git
            .generation
            .as_ref()
            .is_some_and(|generation| generation.path == path);
        let busy = self.git.operation.is_some() || self.git.generation.is_some();
        let checked_count = self.git.snapshot.as_ref().map_or(0, |snapshot| {
            self.checked_staged_files(&path, snapshot).len()
        });
        let staged = checked_count > 0
            && self
                .git
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.target.branch.is_some());
        let staged_count = self
            .git
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.staged_count());
        let disabled = self.closing
            || !self.writable
            || busy
            || self.worktree_has_runs(&path)
            || self.git.probe.is_some()
            || self
                .git
                .repository
                .as_ref()
                .is_some_and(|repository| repository.integration.is_some())
            || !staged;
        let can_commit = !disabled && !draft.input.read(cx).value().trim().is_empty();
        let can_generate = !disabled;
        let generate_tip = if !staged {
            "请勾选需要提交的已暂存文件"
        } else if self.worktree_has_runs(&path) {
            "请等待此工作区的 CLI 任务结束"
        } else if busy {
            "Git 操作或 AI 生成正在进行"
        } else {
            "使用设置中的引擎、Provider 和模型生成 Commit Message"
        };
        let generate_app = view.clone();
        let commit_app = view.clone();
        let alternate_commit_app = view.clone();
        let cancel_app = view.clone();
        let tag_input = draft.tag.clone();
        let tag_for_generate = tag_input.clone();
        let local_tags = self.git.local_tags.get(&path).cloned().unwrap_or_default();
        let remote = self.git.remote_tags.get(&path);
        let remote_names = remote.map(|tags| tags.names.clone());
        let mut tags = local_tags.clone();
        if let Some(remote) = &remote_names {
            tags.extend(remote.iter().cloned());
        }
        tags.sort();
        tags.dedup();
        git::sort_tags(&mut tags);
        let candidate = git::next_tag(&tags, self.store.git.tag_increment, self.store.git.tag_step);
        let tag_query = tag_input.read(cx).value().to_lowercase();
        let local_set: std::collections::HashSet<_> =
            local_tags.iter().map(String::as_str).collect();
        let remote_set = remote_names.as_ref().map(|names| {
            names
                .iter()
                .map(String::as_str)
                .collect::<std::collections::HashSet<_>>()
        });
        // A draft for a new tag must not hide every existing tag in the menu.
        let has_matches = tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(&tag_query));
        let filtered_tags: Vec<_> = tags
            .into_iter()
            .filter(|tag| !has_matches || tag.to_lowercase().contains(&tag_query))
            .take(100)
            .map(|tag| {
                let local = local_set.contains(tag.as_str());
                let status = match (&remote_set, local) {
                    (None, _) => "本地 · 未查询远程",
                    (Some(remote), true) if remote.contains(tag.as_str()) => "本地 / 远程",
                    (Some(_), true) => "仅本地",
                    (Some(_), false) => "仅远程",
                };
                (tag, status)
            })
            .collect();
        let refresh_app = view.clone();
        let refreshing_tags = self.git.remote_tags_probe.is_some();
        let tag_menu_input = tag_input.clone();
        let tag_row = h_flex()
            .w_full()
            .min_w_0()
            .h(px(30.))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&tag_input)
                        .h(px(28.))
                        .text_size(px(12.))
                        .aria_label("Git Tag")
                        .disabled(busy || self.closing || !self.writable),
                ),
            )
            .child(
                Button::new("git-tag-list")
                    .debug_selector(|| "git-tag-list".into())
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::ChevronDown)
                    .tooltip("查看已有标签（最多 100 项），可输入关键字筛选；列表使用缓存")
                    .accessibility_label("查看标签")
                    .disabled(busy || self.closing || !self.writable)
                    .dropdown_menu(move |mut menu, window, _| {
                        menu = menu
                            .scrollable(true)
                            .max_h((window.viewport_size().height - px(48.)).min(px(320.)));
                        if filtered_tags.is_empty() {
                            menu = menu.item(
                                PopupMenuItem::new(if refreshing_tags {
                                    "正在读取标签…"
                                } else {
                                    "暂无标签，可输入新标签或点击刷新"
                                })
                                .disabled(true),
                            );
                        }
                        for (tag, status) in &filtered_tags {
                            let input = tag_menu_input.clone();
                            let selected = tag.clone();
                            menu = menu.item(
                                PopupMenuItem::new(format!("{tag}  ·  {status}")).on_click(
                                    move |_, window, cx| {
                                        input.update(cx, |input, cx| {
                                            input.set_value(selected.clone(), window, cx)
                                        });
                                    },
                                ),
                            );
                        }
                        menu
                    }),
            )
            .child(
                Button::new("git-tag-refresh-remote")
                    .debug_selector(|| "git-tag-refresh-remote".into())
                    .small()
                    .ghost()
                    .compact()
                    .icon(if refreshing_tags {
                        gpui_kit::assets::IconName::X
                    } else {
                        gpui_kit::assets::IconName::RefreshCw
                    })
                    .accessibility_label(if refreshing_tags {
                        "取消查询远程标签"
                    } else {
                        "刷新远程标签"
                    })
                    .tooltip(if refreshing_tags {
                        "取消查询远程标签"
                    } else {
                        "刷新本地和远程标签缓存"
                    })
                    .disabled(self.closing || (busy && !refreshing_tags))
                    .on_click(move |_, _, cx| {
                        refresh_app.update(cx, |this, cx| this.refresh_remote_tags(cx))
                    }),
            )
            .child(
                Button::new("git-tag-generate")
                    .debug_selector(|| "git-tag-generate".into())
                    .small()
                    .ghost()
                    .label("生成")
                    .tooltip("根据已知的本地和远程稳定标签生成下一个版本")
                    .disabled(
                        busy || self.closing
                            || !self.writable
                            || candidate.is_err()
                            || self.git.probe.is_some()
                            || self.git.snapshot.is_none(),
                    )
                    .on_click(move |_, window, cx| {
                        if let Ok(tag) = &candidate {
                            tag_for_generate
                                .update(cx, |input, cx| input.set_value(tag.clone(), window, cx));
                        }
                    }),
            );
        let mut editor = v_flex()
            .debug_selector(|| "git-commit-editor".into())
            .w_full()
            .h_full()
            .min_h_0()
            .min_w_0()
            .px_1()
            .pb_1()
            .gap_1()
            .child(
                h_flex()
                    .h(px(24.))
                    .flex_shrink_0()
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
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child(format!("已勾选 {checked_count} / 已暂存 {staged_count}")),
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
                                        if let Some(generation) = &this.git.generation {
                                            generation.runtime.cancel();
                                        }
                                        cx.notify();
                                    })
                                }),
                        )
                    }),
            )
            .child(tag_row)
            .child(
                v_flex()
                    .debug_selector(|| "git-commit-input".into())
                    .flex_1()
                    .min_h(px(40.))
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(px(4.))
                    .bg(rgb(palette().surface))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div().size_full().min_h_0().child(
                            Textarea::new(&draft.input)
                                .appearance(false)
                                .bordered(false)
                                .h(relative(1.))
                                .text_size(px(13.))
                                .aria_label("Git Commit Message")
                                .disabled(
                                    self.closing
                                        || !self.writable
                                        || generating
                                        || self
                                            .git
                                            .operation
                                            .as_ref()
                                            .is_some_and(|operation| operation.probe.path == path),
                                ),
                        ),
                    ),
            );
        if self.git.feedback.contains_key(&path) || self.git.pending_tag_retry.contains_key(&path) {
            let mut row = h_flex()
                .w_full()
                .min_w_0()
                .h(px(24.))
                .flex_shrink_0()
                .gap_1()
                .items_center();
            if let Some(feedback) = self.git.feedback.get(&path) {
                row = row.child(
                    div()
                        .id("git-action-feedback")
                        .debug_selector(|| "git-action-feedback".into())
                        .flex_1()
                        .min_w_0()
                        .h(px(24.))
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
            if let Some(retry) = self.git.pending_tag_retry.get(&path) {
                let retry_app = view.clone();
                let tooltip = format!(
                    "仅推送 {} 至 {}；需先确保远程分支包含该提交",
                    retry.tag,
                    if retry.remote.is_empty() {
                        "当前上游"
                    } else {
                        &retry.remote
                    }
                );
                row = row.child(
                    Button::new("git-tag-retry")
                        .debug_selector(|| "git-tag-retry".into())
                        .small()
                        .ghost()
                        .compact()
                        .h(px(24.))
                        .icon(IconName::RefreshCw)
                        .accessibility_label("重试推送标签")
                        .tooltip(tooltip)
                        .disabled(
                            busy || self.closing || !self.writable || self.worktree_has_runs(&path),
                        )
                        .on_click(move |_, _, cx| {
                            retry_app.update(cx, |this, cx| this.retry_tag(cx))
                        }),
                );
            }
            editor = editor.child(row);
        }
        editor
            .child(
                h_flex()
                    .debug_selector(|| "git-commit-actions".into())
                    .w_full()
                    .h(px(30.))
                    .flex_shrink_0()
                    .gap_1()
                    .justify_end()
                    .child(
                        Button::new("git-commit")
                            .debug_selector(|| "git-commit".into())
                            .label("Commit")
                            .tooltip(format!("仅提交勾选的 {checked_count} 个已暂存文件"))
                            .primary()
                            .small()
                            .h(px(30.))
                            .disabled(!can_commit)
                            .loading(
                                self.git
                                    .operation
                                    .as_ref()
                                    .is_some_and(|operation| operation.probe.path == path),
                            )
                            .when(can_commit, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                commit_app.update(cx, |this, cx| this.start_commit(false, cx))
                            }),
                    )
                    .child(
                        Button::new("git-commit-alternate")
                            .debug_selector(|| "git-commit-alternate".into())
                            .label("Commit & Push")
                            .small()
                            .h(px(30.))
                            .disabled(!can_commit)
                            .when(can_commit, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                alternate_commit_app
                                    .update(cx, |this, cx| this.start_commit(true, cx))
                            }),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn sync_git_workspace(&mut self, cx: &mut Context<Self>) {
        let path = self.selected_workspace_path();
        if self.git.workspace == path {
            return;
        }
        self.git.workspace = path;
        if let Some(probe) = self.git.remote_tags_probe.take()
            && !self.git.remote_tags.contains_key(&probe.path)
        {
            self.git.remote_tags_checked.remove(&probe.path);
        }
        self.git.branch_probe = None;
        self.git.repository = None;
        self.git.probe = None;
        self.git.snapshot = None;
        self.clear_tree_git_status();
        self.git.error = None;
        self.refresh_branches();
        if self.git_status_visible() {
            self.refresh_git(cx);
        }
        cx.notify();
    }

    pub(super) fn poll_branches(&mut self, cx: &mut Context<Self>) {
        self.sync_git_workspace(cx);
        let update =
            self.git
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
            self.git.branch_probe = None;
            if self.selected_workspace_path().as_ref() == Some(&path)
                && self.git.repository != repository
            {
                let head_changed = self
                    .git
                    .repository
                    .as_ref()
                    .map(|repo| (&repo.current, repo.detached))
                    != repository
                        .as_ref()
                        .map(|repo| (&repo.current, repo.detached));
                self.git.repository = repository;
                if head_changed && self.git_status_visible() && self.git.probe.is_none() {
                    self.refresh_git(cx);
                }
                cx.notify();
            }
        }
        // Pick up changes made in an external terminal, even with the dock closed.
        if self.git.last_branch_check.elapsed() >= Duration::from_secs(10) {
            self.refresh_branches();
        }
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
        if self.git.last_branch_check.elapsed() >= Duration::from_secs(1) {
            self.refresh_branches();
        }
        self.mark_git_refresh();
        self.git.probe = None;
        let Some(path) = self.selected_workspace_path() else {
            self.git.error = Some("未选择工作区".into());
            cx.notify();
            return;
        };
        self.git.error = None;
        let cached_tags = self.git.local_tags.get(&path).cloned();
        self.git.probe = Some(GitProbe::start(path, cached_tags));
        cx.notify();
    }

    /// Collect a finished Git probe; stale snapshots for another workspace are
    /// dropped so switching sessions cannot show the previous directory's diff.
    pub(super) fn poll_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let update = self
            .git
            .probe
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
        self.git.probe = None;
        if self.selected_workspace_path().as_deref() != Some(path.as_path()) {
            return;
        }
        match result {
            Ok(snapshot) => {
                if let Some(selected) = self.git.selected_files.get_mut(&path) {
                    selected.retain(|p| {
                        snapshot
                            .files
                            .iter()
                            .any(|f| f.path == *p && (f.staged() || f.unstaged() || f.untracked()))
                    });
                }
                self.refresh_diff_documents(&path, &snapshot, window, cx);
                self.update_tree_git_status(&path, &snapshot);
                self.git
                    .local_tags
                    .insert(path.clone(), snapshot.tags.clone());
                self.git.snapshot = Some(snapshot);
                self.git.error = None;
            }
            Err(error) => {
                self.git.snapshot = None;
                self.clear_tree_git_status();
                self.git.error = Some(error);
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
        let mut body = sidebar::navigation_content()
            .id("git-panel-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_1()
            .pb_1();
        if self.git.probe.is_some() && self.git.snapshot.is_none() {
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
        if let Some(error) = &self.git.error {
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
        let Some(snapshot) = &self.git.snapshot else {
            return body
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().faint))
                        .child("打开面板时读取工作区 Git 状态"),
                )
                .into_any_element();
        };
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
            let mut groups = v_flex().w_full().gap(px(2.));
            for kind in if self.git.group_by_status {
                GitChangeGroup::STATUS.to_vec()
            } else {
                GitChangeGroup::ALL.to_vec()
            } {
                let files: Vec<_> = snapshot
                    .files
                    .iter()
                    .filter(|file| kind.contains(file))
                    .collect();
                if files.is_empty() {
                    continue;
                }
                let collapsed = self.git.collapsed_groups.contains(&kind);
                let toggle_kind = kind;
                let app = view.clone();
                let workspace = self.selected_workspace_path().unwrap_or_default();
                let selected_count = files
                    .iter()
                    .filter(|file| {
                        self.git
                            .selected_files
                            .get(&workspace)
                            .is_some_and(|set| set.contains(&file.path))
                    })
                    .count();
                let group_files: Vec<_> = files.iter().map(|file| file.path.clone()).collect();
                let select_group_app = view.clone();
                let group_checked = selected_count == files.len();
                let label_app = view.clone();
                let mut group = v_flex().w_full().child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .h(px(24.))
                        .items_center()
                        .bg(rgb(palette().surface_strong))
                        .rounded(px(3.))
                        .child(
                            Button::new(SharedString::from(format!("git-group-{kind:?}")))
                                .debug_selector(move || format!("git-group-{kind:?}"))
                                .ghost()
                                .small()
                                .compact()
                                .w(px(24.))
                                .h(px(24.))
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
                                        if !this.git.collapsed_groups.remove(&toggle_kind) {
                                            this.git.collapsed_groups.insert(toggle_kind);
                                        }
                                        cx.notify();
                                    })
                                })
                                .icon(if collapsed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                }),
                        )
                        .child(
                            Checkbox::new(SharedString::from(format!("git-select-group-{kind:?}")))
                                .debug_selector(move || format!("git-select-group-{kind:?}"))
                                .checked(group_checked)
                                .accessibility_label(format!("选择整组 {}", kind.label()))
                                .tooltip(format!(
                                    "已选择 {selected_count} / {} 个文件；暂存、移出暂存区和提交仅操作勾选文件",
                                    files.len()
                                ))
                                .on_click(move |checked, _, cx| {
                                    select_group_app.update(cx, |this, cx| {
                                        if this.selected_workspace_path().as_ref()
                                            != Some(&workspace)
                                        {
                                            return;
                                        }
                                        let selected = this
                                            .git
                                            .selected_files
                                            .entry(workspace.clone())
                                            .or_default();
                                        for file in &group_files {
                                            if *checked {
                                                selected.insert(file.clone());
                                            } else {
                                                selected.remove(file);
                                            }
                                        }
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new(SharedString::from(format!("git-group-label-{kind:?}")))
                                .ghost()
                                .small()
                                .compact()
                                .flex_1()
                                .min_w_0()
                                .px_2()
                                .accessibility_label(format!(
                                    "{}，{} 个文件，点击{}",
                                    kind.label(),
                                    files.len(),
                                    if collapsed { "展开" } else { "收起" }
                                ))
                                .on_click(move |_, _, cx| {
                                    label_app.update(cx, |this, cx| {
                                        if !this.git.collapsed_groups.remove(&kind) {
                                            this.git.collapsed_groups.insert(kind);
                                        }
                                        cx.notify();
                                    });
                                })
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .font_semibold()
                                                .child(kind.label()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(px(11.))
                                                .text_color(rgb(palette().muted))
                                                .child(if selected_count > 0 {
                                                    format!(
                                                        "{} files · 已选 {selected_count}",
                                                        files.len()
                                                    )
                                                } else {
                                                    format!("{} files", files.len())
                                                }),
                                        ),
                                ),
                        ),
                );
                if !collapsed {
                    let mut rows = v_flex().w_full().pl(px(16.));
                    for file in files {
                        let app = view.clone();
                        let target = file.clone();
                        let can_stage =
                            self.can_stage_files() && (file.unstaged() || file.untracked());
                        let can_unstage = self.can_stage_files() && file.staged();
                        let workspace = self.selected_workspace_path().unwrap_or_default();
                        let checked = self
                            .git
                            .selected_files
                            .get(&workspace)
                            .is_some_and(|set| set.contains(&file.path));
                        let select_app = view.clone();
                        let select_path = file.path.clone();
                        let menu_app = view.clone();
                        let menu_file = file.clone();
                        let menu_workspace = workspace.clone();
                        let diff_workspace = workspace.clone();
                        let file_path = Path::new(&file.path);
                        let filename = file_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        let parent = file_path
                            .parent()
                            .map(|path| path.to_string_lossy().into_owned())
                            .unwrap_or_default();
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
                                .gap(px(6.))
                                .items_center()
                                .h(px(22.))
                                .px_2()
                                .rounded(px(3.))
                                .cursor_pointer()
                                .tooltip({
                                    let tooltip = format!(
                                        "{}\n{} · index: {} / worktree: {}",
                                        file.path,
                                        file.kind().label(),
                                        file.index,
                                        file.worktree
                                    );
                                    move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                                            .build(window, cx)
                                    }
                                })
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
                                        .git
                                        .selected_files
                                        .get(&menu_workspace)
                                        .map(|set| set.iter().cloned().collect())
                                        .unwrap_or_default();
                                    let batch_staged: Vec<String> = menu_app
                                        .read(cx)
                                        .git
                                        .snapshot
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
                                    let discard_app = menu_app.clone();
                                    let discard_file = menu_file.clone();
                                    let discard_workspace = menu_workspace.clone();
                                    let can_discard = menu_app.read(cx).can_stage_files()
                                        && !menu_app
                                            .read(cx)
                                            .git
                                            .repository
                                            .as_ref()
                                            .is_some_and(|repo| repo.integration.is_some());
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
                                    .separator()
                                    .item(
                                        PopupMenuItem::new("回滚文件修改…")
                                            .disabled(!can_discard)
                                            .on_click(move |_, window, cx| {
                                                discard_app.update(cx, |this, cx| {
                                                    this.confirm_git_discard(
                                                        discard_workspace.clone(),
                                                        discard_file.clone(),
                                                        window,
                                                        cx,
                                                    );
                                                });
                                            }),
                                    )
                                })
                                .child(
                                    Checkbox::new(SharedString::from(format!(
                                        "git-select-{}",
                                        file.path
                                    )))
                                    .debug_selector({
                                        let path = file.path.clone();
                                        move || format!("git-select-{path}")
                                    })
                                    .checked(checked)
                                    .disabled(
                                        !(file.staged() || file.unstaged() || file.untracked()),
                                    )
                                    .accessibility_label(format!("选择 {}", file.path))
                                    .on_click(
                                        move |checked, _, cx| {
                                            select_app.update(cx, |this, cx| {
                                                let set = this
                                                    .git
                                                    .selected_files
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
                                        .text_color(rgb(palette().activity_running)),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_shrink_1()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(rgb(git_change_color(file.kind())))
                                        .child(filename),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(rgb(palette().muted))
                                        .child(parent),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_size(px(11.))
                                        .text_color(rgb(git_change_color(file.kind())))
                                        .child(if file.untracked() {
                                            "?".to_string()
                                        } else if file.worktree != ' ' {
                                            file.worktree.to_string()
                                        } else {
                                            file.index.to_string()
                                        }),
                                )
                                .when(file.staged(), |row| {
                                    row.child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette().highlight_text))
                                            .child(if file.unstaged() {
                                                "部分暂存"
                                            } else {
                                                "已暂存"
                                            }),
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
        let collapse_app = view.clone();
        let active_groups = if self.git.group_by_status {
            GitChangeGroup::STATUS.to_vec()
        } else {
            GitChangeGroup::ALL.to_vec()
        };
        let collapsed = active_groups
            .iter()
            .all(|kind| self.git.collapsed_groups.contains(kind));
        let options_app = view.clone();
        let group_by_status = self.git.group_by_status;
        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                h_flex()
                    .h(px(32.))
                    .flex_shrink_0()
                    .items_center()
                    .px_1()
                    .gap_1()
                    .child(refresh_icon("git-refresh"))
                    .child(self.git_staging_actions(snapshot, view))
                    .child(
                        Button::new("git-collapse-all")
                            .debug_selector(|| "git-collapse-all".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(if collapsed {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .accessibility_label(if collapsed {
                                "展开所有变更组"
                            } else {
                                "折叠所有变更组"
                            })
                            .tooltip(if collapsed {
                                "展开所有变更组"
                            } else {
                                "折叠所有变更组"
                            })
                            .on_click(move |_, _, cx| {
                                collapse_app.update(cx, |this, cx| {
                                    if collapsed {
                                        this.git.collapsed_groups.clear();
                                    } else {
                                        this.git
                                            .collapsed_groups
                                            .extend(active_groups.iter().copied());
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("git-group-options")
                            .debug_selector(|| "git-group-options".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Ellipsis)
                            .accessibility_label("变更分组方式")
                            .tooltip("变更分组方式")
                            .dropdown_menu(move |mut menu, _, _| {
                                for (status, label) in [
                                    (false, "按版本控制分组"),
                                    (true, "按新增 / 修改 / 删除分组"),
                                ] {
                                    let app = options_app.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(label)
                                            .checked(group_by_status == status)
                                            .on_click(move |_, _, cx| {
                                                app.update(cx, |this, cx| {
                                                    this.git.group_by_status = status;
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    ),
            )
            .child(
                h_flex()
                    .h(px(26.))
                    .flex_shrink_0()
                    .min_w_0()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child(Icon::new(gpui_kit::assets::IconName::GitBranch).size(px(13.)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(snapshot.branch.clone()),
                    )
                    .child(format!(
                        "暂存 {} · 未暂存 {} · 未跟踪 {}",
                        snapshot.staged_count(),
                        snapshot.unstaged_count(),
                        snapshot.untracked_count()
                    ))
                    .when(self.git.probe.is_some(), |row| {
                        row.child(Spinner::new().small())
                    }),
            )
            .child(body)
            .into_any_element()
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
