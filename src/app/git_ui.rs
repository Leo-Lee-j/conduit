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
                .placeholder(conduit::i18n::text_static("git_ui.enter_a_commit_message"))
                .auto_grow(3, 3)
        });
        let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        let tag = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("git_ui.tag_optional_e_g_v1_2_3"))
        });
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
        self.git_notice(
            path,
            text,
            if error {
                NoticeLevel::Error
            } else {
                NoticeLevel::Info
            },
            if error {
                NoticePresentation::Toast
            } else {
                NoticePresentation::History
            },
            cx,
        );
    }

    pub(super) fn git_result_feedback(
        &mut self,
        path: PathBuf,
        text: impl Into<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.git_notice(
            path,
            text,
            if error {
                NoticeLevel::Error
            } else {
                NoticeLevel::Success
            },
            NoticePresentation::Toast,
            cx,
        );
    }

    fn git_notice(
        &mut self,
        path: PathBuf,
        text: impl Into<String>,
        level: NoticeLevel,
        presentation: NoticePresentation,
        cx: &mut Context<Self>,
    ) {
        self.notify_user(
            Notice::new(
                NoticeSource::Git,
                level,
                match level {
                    NoticeLevel::Error => conduit::i18n::text_static("git_ui.git_operation_failed"),
                    NoticeLevel::Success => {
                        conduit::i18n::text_static("git_ui.git_operation_completed")
                    }
                    _ => conduit::i18n::text_static("git_ui.git_notice"),
                },
                text,
            )
            .target(NoticeTarget::Workspace(path))
            .presentation(presentation),
            cx,
        );
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

    fn auto_stage_tracked_files(&mut self, cx: &mut Context<Self>) {
        if !self.can_stage_files()
            || self.git.repository.as_ref().is_some_and(|repository| {
                repository.integration.is_some() || !repository.conflicts.is_empty()
            })
        {
            return;
        }
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        let Some(snapshot) = &self.git.snapshot else {
            return;
        };
        // AA/DD are conflicts too, even though neither status column contains U.
        if snapshot.files.iter().any(|file| {
            file.index == 'U'
                || file.worktree == 'U'
                || matches!((file.index, file.worktree), ('A', 'A') | ('D', 'D'))
        }) {
            return;
        }
        let files: Vec<_> = snapshot
            .files
            .iter()
            .filter(|file| !file.untracked() && file.unstaged())
            .map(|file| file.path.clone())
            .collect();
        if files.is_empty() {
            return;
        }
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(
                path.clone(),
                git::Operation::StageTracked {
                    files,
                    target: snapshot.target.clone(),
                },
            ),
            draft: String::new(),
            commit_push: None,
            automatic_stage: true,
        });
        self.git_feedback(
            path,
            conduit::i18n::text_static("git_ui.automatically_staging_changes_to_tracked_files"),
            false,
            cx,
        );
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.the_selected_files_have_no_unstaged_changes"),
                true,
                cx,
            );
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
            commit_push: None,
            automatic_stage: false,
        });
        self.git_feedback(
            path,
            conduit::i18n::text_static("git_ui.staging_selected_files"),
            false,
            cx,
        );
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.the_selected_files_have_no_staged_changes"),
                true,
                cx,
            );
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
            commit_push: None,
            automatic_stage: false,
        });
        self.git_feedback(
            path,
            conduit::i18n::text_static("git_ui.unstaging_selected_files"),
            false,
            cx,
        );
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
            let description = conduit::i18n::format(
                "git_ui.discard_all_changes_to_including_staged_changes",
                &[("arg0", file.path.to_string())],
            );
            dialog
                .title(conduit::i18n::text_static("git_ui.discard_file_changes"))
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
                            probe: git::OperationProbe::start(
                                workspace.clone(),
                                git::Operation::Discard {
                                    file: file.clone(),
                                    target: target.clone(),
                                },
                            ),
                            draft: String::new(),
                            commit_push: None,
                            automatic_stage: false,
                        });
                        this.git_feedback(
                            workspace.clone(),
                            conduit::i18n::text_static("git_ui.discarding_file_changes"),
                            false,
                            cx,
                        );
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
                    .accessibility_label(conduit::i18n::text_static(
                        "git_ui.select_all_changed_files",
                    ))
                    .tooltip(if all_selected {
                        conduit::i18n::text_static("git_ui.deselect_all")
                    } else {
                        conduit::i18n::text_static("git_ui.select_all_changed_files")
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
                    .accessibility_label(conduit::i18n::text_static("git_ui.stage_selected_files"))
                    .tooltip(conduit::i18n::format(
                        "git_ui.stage_selected_files_640698",
                        &[("arg0", format!("{}", selected.len()))],
                    ))
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
                    .accessibility_label(conduit::i18n::text_static(
                        "git_ui.unstage_selected_files",
                    ))
                    .tooltip(conduit::i18n::format(
                        "git_ui.unstage_selected_files_4f792b",
                        &[("arg0", format!("{}", selected_staged.len()))],
                    ))
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
                conduit::i18n::text_static("git_ui.wait_for_the_cli_task_to_finish"),
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.select_staged_files_to_commit"),
                true,
                cx,
            );
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.enter_a_commit_message_or_generate_one"),
                true,
                cx,
            );
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
                conduit::i18n::format(
                    "git_ui.the_last_query_found_tag_on_the",
                    &[("name", name.to_string())],
                ),
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
            commit_push: Some(push),
            automatic_stage: false,
        });
        self.git_feedback(
            path,
            if push {
                conduit::i18n::text_static("git_ui.committing_and_pushing")
            } else {
                conduit::i18n::text_static("git_ui.committing")
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.remote_tag_query_cancelled"),
                false,
                cx,
            );
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.refreshing_local_tag_cache"),
                false,
                cx,
            );
            return;
        }
        self.git.remote_tags_checked.insert(path.clone());
        self.git.remote_tags_probe = Some(git::RemoteTagsProbe::start(path.clone()));
        self.git_feedback(
            path,
            conduit::i18n::text_static("git_ui.refreshing_tag_cache"),
            false,
            cx,
        );
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
            commit_push: None,
            automatic_stage: false,
        });
        self.git_feedback(
            path,
            conduit::i18n::text_static("git_ui.retrying_tag_push"),
            false,
            cx,
        );
    }

    pub(super) fn pending_push_status(&self) -> Option<(usize, String)> {
        let repository = self
            .git
            .repository
            .as_ref()
            .filter(|repository| !repository.detached && repository.ahead > 0)?;
        let tooltip = match &repository.upstream {
            Some(upstream) => conduit::i18n::format(
                "git_ui.push_commits_to",
                &[
                    ("arg0", format!("{}", repository.ahead)),
                    ("arg1", upstream.to_string()),
                ],
            ),
            None => conduit::i18n::format(
                "git_ui.unpublished_local_commits_no_upstream_configured",
                &[("arg0", format!("{}", repository.ahead))],
            ),
        };
        Some((repository.ahead, tooltip))
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
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.git_status_is_refreshing_try_generating_again"),
                true,
                cx,
            );
            return;
        }
        let Some(snapshot) = self.git.snapshot.as_ref() else {
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.refresh_the_workspace_git_status_first"),
                true,
                cx,
            );
            return;
        };
        let selected = self.checked_staged_files(&path, snapshot);
        if selected.is_empty() {
            self.git_feedback(
                path,
                conduit::i18n::text_static("git_ui.select_staged_files_before_generating_a_commit"),
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
                self.git_feedback(
                    path,
                    conduit::i18n::format(
                        "git_ui.generation_provider_unavailable",
                        &[("error", format!("{:#}", error))],
                    ),
                    true,
                    cx,
                );
                return;
            }
        };
        let executable = match cli::resolve_for_launch(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(error) => {
                self.git_feedback(
                    path,
                    conduit::i18n::format(
                        "git_ui.unable_to_generate_commit_message",
                        &[("error", format!("{:#}", error))],
                    ),
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
            cancelled: false,
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
            cleanup_error: None,
        });
        self.git_feedback(
            path,
            conduit::i18n::format(
                "git_ui.generating_commit_message_with",
                &[("arg0", provider.label().to_string())],
            ),
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
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                        probe.path.clone(),
                        Err(conduit::i18n::text_static("git_ui.remote_tag_reader_exited").into()),
                    )),
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
                            conduit::i18n::format(
                                "git_ui.read_remote_tags_from",
                                &[
                                    ("remote", remote.to_string()),
                                    ("count", format!("{}", count)),
                                ],
                            ),
                            false,
                            cx,
                        );
                    }
                    Err(error) => self.git_feedback(
                        path,
                        conduit::i18n::format(
                            "git_ui.failed_to_read_remote_tags",
                            &[("error", error.to_string())],
                        ),
                        true,
                        cx,
                    ),
                }
            }
        }
        let completed = self.git.operation.as_ref().and_then(|operation| {
            match operation.probe.events.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err(conduit::i18n::text_static(
                        "git_ui.git_worker_exited_refresh_repository_status_to",
                    )
                    .into()))
                }
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
                    if operation.automatic_stage {
                        self.git_feedback(path.clone(), text, result.error.is_some(), cx);
                    } else {
                        self.git_result_feedback(path.clone(), text, result.error.is_some(), cx);
                    }
                }
                Err(error) => {
                    if operation.automatic_stage {
                        self.git_feedback(path.clone(), error, true, cx);
                    } else {
                        self.git_result_feedback(path.clone(), error, true, cx);
                    }
                }
            }
            if self.selected_workspace_path().as_ref() == Some(&path) {
                self.git.branch_probe = None;
                self.refresh_branches();
                self.refresh_git(cx);
                // Read back the operation result without immediately undoing an
                // explicit unstage or looping on an add failure/dirty submodule.
                // The next regular or manual refresh can auto-stage again.
                self.git.auto_stage_after_refresh = false;
            }
        }
        let mut generation_result = None;
        let mut cleanup_notice = None;
        if let Some(generation) = &mut self.git.generation {
            if generation.started.elapsed() > Duration::from_secs(120) && generation.error.is_none()
            {
                generation.error = Some(
                    conduit::i18n::text_static("git_ui.ai_generation_timed_out_please_retry")
                        .into(),
                );
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
                                .unwrap_or_else(|| {
                                    conduit::i18n::text_static(
                                        "git_ui.ai_worker_exited_please_retry",
                                    )
                                    .into()
                                }),
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
                    Event::CleanupBlocked(error) => {
                        generation.cleanup_error = Some(error.clone());
                        generation.error = Some(error.clone());
                        generation.runtime.cancel();
                        cleanup_notice = Some((generation.path.clone(), error));
                        self.closing = false;
                        cx.notify();
                    }
                    Event::Approval(approval) => {
                        generation.runtime.decide(approval.id, false);
                        generation.error = Some(
                            conduit::i18n::text_static(
                                "git_ui.ai_requested_tool_permission_and_was_stopped",
                            )
                            .into(),
                        );
                        generation.runtime.cancel();
                    }
                    Event::Tool { .. } | Event::ToolInput { .. } => {
                        generation.error = Some(
                            conduit::i18n::text_static(
                                "git_ui.ai_attempted_to_call_a_tool_generation",
                            )
                            .into(),
                        );
                        generation.runtime.cancel();
                    }
                    Event::Finished { error, cancelled } => {
                        generation.cancelled =
                            cancelled && generation.error.is_none() && error.is_none();
                        generation_result =
                            Some(generation.error.clone().or(error).or_else(|| {
                                cancelled.then(|| {
                                    conduit::i18n::text_static("git_ui.generation_cancelled").into()
                                })
                            }));
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
                    generation.error = Some(
                        conduit::i18n::text_static(
                            "git_ui.the_generated_commit_message_is_too_long",
                        )
                        .into(),
                    );
                    generation.runtime.cancel();
                }
            }
        }
        if let Some((path, error)) = cleanup_notice {
            self.git_feedback(path, error, true, cx);
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
            if generation.cleanup_error.is_some() {
                // Already delivered through the notification center when cleanup blocked.
            } else if generation.cancelled {
                self.git_feedback(
                    generation.path,
                    conduit::i18n::text_static("git_ui.generation_cancelled_draft_unchanged"),
                    false,
                    cx,
                );
            } else if let Some(error) = error {
                self.git_result_feedback(
                    generation.path,
                    conduit::i18n::format(
                        "git_ui.generation_failed",
                        &[
                            ("arg0", failure_summary(&error).to_string()),
                            ("arg1", failure_guidance(&error).to_string()),
                        ],
                    ),
                    true,
                    cx,
                );
            } else if text.trim().is_empty() {
                self.git_result_feedback(
                    generation.path,
                    generation
                        .last_diagnostic
                        .map(|error| {
                            conduit::i18n::format(
                                "git_ui.generation_failed",
                                &[
                                    ("arg0", failure_summary(&error).to_string()),
                                    ("arg1", failure_guidance(&error).to_string()),
                                ],
                            )
                        })
                        .unwrap_or_else(|| {
                            conduit::i18n::text_static(
                                "git_ui.ai_returned_no_commit_message_check_the",
                            )
                            .into()
                        }),
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
                        conduit::i18n::text_static(
                            "git_ui.commit_message_generated_review_and_edit_it",
                        ),
                        false,
                        cx,
                    );
                } else {
                    self.git_result_feedback(
                        generation.path,
                        conduit::i18n::text_static("git_ui.the_draft_was_edited_and_was_not"),
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
        let commit_push = self
            .git
            .operation
            .as_ref()
            .filter(|operation| operation.probe.path == path)
            .and_then(|operation| operation.commit_push);
        let checked_count = self.git.snapshot.as_ref().map_or(0, |snapshot| {
            self.checked_staged_files(&path, snapshot).len()
        });
        let staged_count = self
            .git
            .snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.staged_count());
        let blocked = if self.closing {
            Some(conduit::i18n::text_static("git_ui.closing_the_application"))
        } else if !self.writable {
            Some(conduit::i18n::text_static(
                "git_ui.application_data_is_read_only_resolve_the",
            ))
        } else if busy {
            Some(conduit::i18n::text_static(
                "git_ui.a_git_operation_or_ai_generation_is",
            ))
        } else if self.worktree_has_runs(&path) {
            Some(conduit::i18n::text_static(
                "git_ui.a_session_task_is_running_in_this",
            ))
        } else if self.git.probe.is_some() {
            Some(conduit::i18n::text_static(
                "git_ui.refreshing_git_status_please_wait",
            ))
        } else if self
            .git
            .repository
            .as_ref()
            .is_some_and(|repository| repository.integration.is_some())
        {
            Some(conduit::i18n::text_static(
                "git_ui.a_merge_or_rebase_is_unfinished_resolve",
            ))
        } else if self.git.snapshot.is_none() {
            Some(conduit::i18n::text_static(
                "git_ui.git_status_has_not_loaded_refresh_to",
            ))
        } else if self
            .git
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.target.branch.is_none())
        {
            Some(conduit::i18n::text_static(
                "git_ui.head_is_detached_switch_to_or_create",
            ))
        } else if checked_count == 0 {
            Some(if staged_count == 0 {
                conduit::i18n::text_static("git_ui.select_files_and_enter_a_commit_message")
            } else {
                conduit::i18n::text_static("git_ui.select_staged_files_to_commit_dbd187")
            })
        } else {
            None
        };
        let disabled = blocked.is_some();
        let commit_blocked =
            blocked.or_else(|| {
                draft.input.read(cx).value().trim().is_empty().then_some(
                    conduit::i18n::text_static(
                        "git_ui.enter_a_commit_message_or_generate_one_c1c756",
                    ),
                )
            });
        let can_commit = commit_blocked.is_none();
        let can_generate = !disabled;
        let generate_tip = blocked.unwrap_or(conduit::i18n::text_static(
            "git_ui.generate_a_commit_message_using_the_configured",
        ));
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
                    (None, _) => conduit::i18n::text_static("git_ui.local_remote_not_queried"),
                    (Some(remote), true) if remote.contains(tag.as_str()) => {
                        conduit::i18n::text_static("git_ui.local_remote")
                    }
                    (Some(_), true) => conduit::i18n::text_static("git_ui.local_only"),
                    (Some(_), false) => conduit::i18n::text_static("git_ui.remote_only"),
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
                        .text_size(ui_text_size(12.))
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
                    .tooltip(conduit::i18n::text_static(
                        "git_ui.view_existing_tags_up_to_100_type",
                    ))
                    .accessibility_label(conduit::i18n::text_static("git_ui.view_tags"))
                    .disabled(busy || self.closing || !self.writable)
                    .dropdown_menu(move |mut menu, window, _| {
                        menu = menu
                            .scrollable(true)
                            .max_h((window.viewport_size().height - px(48.)).min(px(320.)));
                        if filtered_tags.is_empty() {
                            menu = menu.item(
                                PopupMenuItem::new(if refreshing_tags {
                                    conduit::i18n::text_static("git_ui.loading_tags")
                                } else {
                                    conduit::i18n::text_static("git_ui.no_tags_yet_enter_a_new_tag")
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
                        conduit::i18n::text_static("git_ui.cancel_remote_tag_query")
                    } else {
                        conduit::i18n::text_static("git_ui.refresh_remote_tags")
                    })
                    .tooltip(if refreshing_tags {
                        conduit::i18n::text_static("git_ui.cancel_remote_tag_query")
                    } else {
                        conduit::i18n::text_static("git_ui.refresh_local_and_remote_tag_cache")
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
                    .label(conduit::i18n::text_static("git_ui.generate"))
                    .tooltip(conduit::i18n::text_static(
                        "git_ui.generate_the_next_version_from_known_local",
                    ))
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
                            .text_size(ui_text_size(12.))
                            .font_semibold()
                            .child(conduit::i18n::text_static("git_ui.commit_message")),
                    )
                    .when(staged_count > 0, |row| {
                        row.child(
                            h_flex()
                                .h(px(18.))
                                .px_2()
                                .items_center()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::format(
                                    "git_ui.selected_staged",
                                    &[
                                        ("checked_count", format!("{}", checked_count)),
                                        ("staged_count", format!("{}", staged_count)),
                                    ],
                                )),
                        )
                    })
                    .child(
                        Button::new("git-generate-message")
                            .ghost()
                            .small()
                            .compact()
                            .icon(gpui_kit::assets::IconName::Sparkles)
                            .accessibility_label(conduit::i18n::text_static(
                                "git_ui.generate_commit_message_with_ai",
                            ))
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
                                .accessibility_label(conduit::i18n::text_static(
                                    "git_ui.cancel_commit_message_generation",
                                ))
                                .tooltip(conduit::i18n::text_static("git_ui.cancel_generation"))
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
                                .text_size(ui_text_size(13.))
                                .aria_label(conduit::i18n::text_static("git_ui.git_commit_message"))
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
        if self.git.pending_tag_retry.contains_key(&path) {
            let mut row = h_flex()
                .w_full()
                .min_w_0()
                .h(px(24.))
                .flex_shrink_0()
                .gap_1()
                .items_center();
            if let Some(retry) = self.git.pending_tag_retry.get(&path) {
                let retry_app = view.clone();
                let tooltip = conduit::i18n::format(
                    "git_ui.push_only_to_ensure_the_remote_branch",
                    &[
                        ("arg0", retry.tag.to_string()),
                        (
                            "arg1",
                            (if retry.remote.is_empty() {
                                conduit::i18n::text_static("git_ui.current_upstream")
                            } else {
                                &retry.remote
                            })
                            .to_string(),
                        ),
                    ],
                );
                row = row.child(
                    Button::new("git-tag-retry")
                        .debug_selector(|| "git-tag-retry".into())
                        .small()
                        .ghost()
                        .compact()
                        .h(px(24.))
                        .icon(IconName::RefreshCw)
                        .accessibility_label(conduit::i18n::text_static("git_ui.retry_tag_push"))
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
                            .label(conduit::i18n::text_static("files.commit"))
                            .tooltip(commit_blocked.map(str::to_owned).unwrap_or_else(|| {
                                conduit::i18n::format(
                                    "git_ui.commit_only_the_selected_staged_files_review",
                                    &[("checked_count", format!("{}", checked_count))],
                                )
                            }))
                            .primary()
                            .small()
                            .h(px(30.))
                            .disabled(!can_commit)
                            .loading(commit_push == Some(false))
                            .when(commit_push == Some(false), |button| {
                                button.icon(Spinner::new())
                            })
                            .when(can_commit, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                commit_app.update(cx, |this, cx| this.start_commit(false, cx))
                            }),
                    )
                    .child(
                        Button::new("git-commit-alternate")
                            .debug_selector(|| "git-commit-alternate".into())
                            .label(conduit::i18n::text_static("git_ui.commit_and_push"))
                            .small()
                            .h(px(30.))
                            .disabled(!can_commit)
                            .loading(commit_push == Some(true))
                            .when(commit_push == Some(true), |button| {
                                button.icon(Spinner::new())
                            })
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
            self.notify_user(
                Notice::new(
                    NoticeSource::Git,
                    NoticeLevel::Warning,
                    conduit::i18n::text_static("git_ui.unable_to_view_diff"),
                    conduit::i18n::text_static("git_ui.select_a_workspace_before_viewing_diffs"),
                ),
                cx,
            );
            return;
        };
        self.open_diff_document(workspace, file, window, cx);
    }

    /// Read Git status in the background, then auto-stage tracked changes when idle.
    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.git.last_branch_check.elapsed() >= Duration::from_secs(1) {
            self.refresh_branches();
        }
        self.mark_git_refresh();
        self.git.probe = None;
        let Some(path) = self.selected_workspace_path() else {
            self.git.error =
                Some(conduit::i18n::text_static("git_ui.no_workspace_selected").into());
            cx.notify();
            return;
        };
        self.git.error = None;
        let cached_tags = self.git.local_tags.get(&path).cloned();
        self.git.probe = Some(GitProbe::start(path, cached_tags));
        self.git.auto_stage_after_refresh = true;
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
                    Err(
                        conduit::i18n::text_static("git_ui.git_reader_exited_please_retry")
                            .to_string(),
                    ),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        let Some((path, result)) = update else {
            return;
        };
        self.git.probe = None;
        let auto_stage = std::mem::take(&mut self.git.auto_stage_after_refresh);
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
                if auto_stage {
                    self.auto_stage_tracked_files(cx);
                }
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
                .accessibility_label(conduit::i18n::text_static("git_ui.refresh_git_status"))
                .tooltip(conduit::i18n::text_static("git_ui.refresh_git_status"))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    app.update(cx, |this, cx| this.refresh_git(cx));
                })
                .into_any_element()
        };
        let retry = |id: &'static str| -> AnyElement {
            let app = view.clone();
            Button::new(id)
                .label(conduit::i18n::text_static("git_diff.reload"))
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
            .debug_selector(|| "git-panel-body".into())
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
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::text_static(
                                    "git_ui.reading_workspace_git_status",
                                )),
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
                                .text_size(ui_text_size(12.))
                                .font_semibold()
                                .text_color(rgb(palette().danger_text))
                                .child(conduit::i18n::text_static(
                                    "git_ui.unable_to_read_git_status",
                                )),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
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
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::text_static(
                            "git_ui.read_workspace_git_status_when_opening_the",
                        )),
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
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static(
                                "git_ui.no_uncommitted_changes_in_this_workspace",
                            )),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(rgb(palette().faint))
                            .child(conduit::i18n::text_static(
                                "git_ui.after_editing_files_in_the_terminal_refresh",
                            )),
                    ),
            );
        } else {
            let mut groups = v_flex().w_full().flex_shrink_0().gap(px(2.));
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
                let mut group = v_flex().w_full().flex_shrink_0().child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .h(px(24.))
                        .flex_shrink_0()
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
                                .tooltip(conduit::i18n::format("git_ui.click_to_changes", &[("arg0", (if collapsed { conduit::i18n::text_static("chat.expand") } else { conduit::i18n::text_static("chat.collapse") }).to_string())]))
                                .accessibility_label(conduit::i18n::format("git_ui.changes_click_to", &[("arg0", kind.label().to_string()), ("arg1", format!("{}", files.len())), ("arg2", (if collapsed { conduit::i18n::text_static("chat.expand") } else { conduit::i18n::text_static("chat.collapse") }).to_string())]))
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
                                .accessibility_label(conduit::i18n::format("git_ui.select_group", &[("arg0", kind.label().to_string())]))
                                .tooltip(conduit::i18n::format("git_ui.selected_files_tracked_changes_are_staged_automatically", &[("selected_count", format!("{}", selected_count)), ("arg0", format!("{}", files.len()))]))
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
                                .accessibility_label(conduit::i18n::format("git_ui.files_click_to", &[("arg0", kind.label().to_string()), ("arg1", format!("{}", files.len())), ("arg2", (if collapsed { conduit::i18n::text_static("chat.expand") } else { conduit::i18n::text_static("chat.collapse") }).to_string())]))
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
                                                .text_size(ui_text_size(12.))
                                                .font_semibold()
                                                .child(kind.label()),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(ui_text_size(11.))
                                                .text_color(rgb(palette().muted))
                                                .child(if selected_count > 0 {
                                                    conduit::i18n::format("git_ui.files_selected", &[("arg0", format!("{}", files.len())), ("selected_count", format!("{}", selected_count))])
                                                } else {
                                                    format!("{} files", files.len())
                                                }),
                                        ),
                                ),
                        ),
                );
                if !collapsed {
                    let mut rows = v_flex().w_full().flex_shrink_0().pl(px(16.));
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
                        rows =
                            rows.child(
                                h_flex()
                                    .id(SharedString::from(format!("git-file-{}", file.path)))
                                    .role(Role::Button)
                                    .debug_selector({
                                        let path = file.path.clone();
                                        move || format!("git-change-file-{path}")
                                    })
                                    .aria_label(conduit::i18n::format(
                                        "git_ui.double_click_to_open_a_diff_tab",
                                        &[("arg0", file.path.to_string())],
                                    ))
                                    .w_full()
                                    .min_w_0()
                                    .gap(px(6.))
                                    .items_center()
                                    .h(px(22.))
                                    .flex_shrink_0()
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
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                tooltip.clone(),
                                            )
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
                                            && menu_app
                                                .read(cx)
                                                .git
                                                .repository
                                                .as_ref()
                                                .is_none_or(|repo| repo.integration.is_none());
                                        let disabled = !can_stage;
                                        menu.item(
                                            PopupMenuItem::new(conduit::i18n::text_static(
                                                "git_ui.stage",
                                            ))
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
                                            PopupMenuItem::new(conduit::i18n::format(
                                                "git_ui.stage_selected_files_640698",
                                                &[("arg0", format!("{}", batch.len()))],
                                            ))
                                            .disabled(
                                                batch.is_empty()
                                                    || !menu_app.read(cx).can_stage_files(),
                                            )
                                            .on_click(move |_, _, cx| {
                                                batch_app.update(cx, |this, cx| {
                                                    if this.selected_workspace_path().as_ref()
                                                        == Some(&batch_workspace)
                                                    {
                                                        this.stage_git_files(batch.clone(), cx);
                                                    }
                                                });
                                            }),
                                        )
                                        .item(
                                            PopupMenuItem::new(conduit::i18n::text_static(
                                                "git_ui.unstage",
                                            ))
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
                                            PopupMenuItem::new(conduit::i18n::format(
                                                "git_ui.unstage_selected",
                                                &[("arg0", format!("{}", batch_staged.len()))],
                                            ))
                                            .disabled(
                                                batch_staged.is_empty()
                                                    || !menu_app.read(cx).can_stage_files(),
                                            )
                                            .on_click(move |_, _, cx| {
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
                                            }),
                                        )
                                        .separator()
                                        .item(
                                            PopupMenuItem::new(conduit::i18n::text_static(
                                                "git_ui.discard_file_changes_d200fe",
                                            ))
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
                                        .accessibility_label(conduit::i18n::format(
                                            "git_ui.select",
                                            &[("arg0", file.path.to_string())],
                                        ))
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
                                            .text_size(ui_text_size(12.))
                                            .text_color(rgb(git_change_color(file.kind())))
                                            .child(filename),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(ui_text_size(11.))
                                            .text_color(rgb(palette().muted))
                                            .child(parent),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_size(ui_text_size(11.))
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
                                                .text_size(ui_text_size(10.))
                                                .text_color(rgb(palette().highlight_text))
                                                .child(if file.unstaged() {
                                                    conduit::i18n::text_static(
                                                        "git_ui.partially_staged",
                                                    )
                                                } else {
                                                    conduit::i18n::text_static("git_ui.staged")
                                                }),
                                        )
                                    })
                                    .when_some(file.original.clone(), |row, original| {
                                        row.child(
                                            div()
                                                .flex_shrink_0()
                                                .max_w(px(132.))
                                                .truncate()
                                                .text_size(ui_text_size(10.))
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
                body = body.child(v_flex().w_full().flex_shrink_0().gap_1().pt_1().children(
                    snapshot.notes.iter().map(|note| {
                        div()
                            .text_size(ui_text_size(10.))
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
                                conduit::i18n::text_static("git_ui.expand_all_change_groups")
                            } else {
                                conduit::i18n::text_static("git_ui.collapse_all_change_groups")
                            })
                            .tooltip(if collapsed {
                                conduit::i18n::text_static("git_ui.expand_all_change_groups")
                            } else {
                                conduit::i18n::text_static("git_ui.collapse_all_change_groups")
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
                            .accessibility_label(conduit::i18n::text_static("git_ui.group_changes"))
                            .tooltip(conduit::i18n::text_static("git_ui.group_changes"))
                            .dropdown_menu(move |mut menu, _, _| {
                                for (status, label) in [
                                    (
                                        false,
                                        conduit::i18n::text_static(
                                            "git_ui.group_by_version_control_status",
                                        ),
                                    ),
                                    (
                                        true,
                                        conduit::i18n::text_static(
                                            "git_ui.group_by_added_modified_deleted",
                                        ),
                                    ),
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
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(Icon::new(gpui_kit::assets::IconName::GitBranch).size(px(13.)))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(snapshot.branch.clone()),
                    )
                    .child(conduit::i18n::format(
                        "git_ui.staged_unstaged_untracked",
                        &[
                            ("arg0", format!("{}", snapshot.staged_count())),
                            ("arg1", format!("{}", snapshot.unstaged_count())),
                            ("arg2", format!("{}", snapshot.untracked_count())),
                        ],
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
