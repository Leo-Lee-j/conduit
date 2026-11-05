//! Chat submission, stream polling, cancellation, and approvals.

use super::*;

mod saves;
pub(super) use saves::{PendingQueueSave, PendingTurn};

/// Only the latest failed exchange can reuse its original prompt.
pub(super) fn error_retry_prompt(session: &SessionRecord, index: usize) -> Option<&str> {
    if session.messages.get(index)?.kind != ItemKind::Error
        || session.messages[index + 1..]
            .iter()
            .any(|item| matches!(item.kind, ItemKind::User | ItemKind::Assistant))
    {
        return None;
    }
    session.messages[..index]
        .iter()
        .rfind(|item| item.kind == ItemKind::User)
        .map(|item| item.text.as_str())
}

/// Remove error cards after the last visible reply, retaining their positions
/// so a rejected send can restore the original transcript.
pub(super) fn clear_retry_errors(session: &mut SessionRecord) -> Vec<(usize, ChatItem)> {
    let start = session
        .messages
        .iter()
        .rposition(|item| item.kind == ItemKind::Assistant && !item.text.trim().is_empty())
        .map_or(0, |index| index + 1);
    let mut removed = Vec::new();
    let mut index = 0;
    session.messages.retain(|item| {
        let keep = index < start || item.kind != ItemKind::Error;
        if !keep {
            removed.push((index, item.clone()));
        }
        index += 1;
        keep
    });
    removed
}

/// Clearing errors can reduce the message count below an existing turn number.
pub(super) fn next_turn_prefix(session: &SessionRecord) -> String {
    let mut turn = session.messages.len();
    loop {
        let prefix = format!("turn-{turn}");
        let key_prefix = format!("{prefix}-");
        if !session
            .messages
            .iter()
            .any(|item| item.key.starts_with(&key_prefix))
        {
            return prefix;
        }
        turn += 1;
    }
}

impl ConduitApp {
    pub(super) fn workspace_busy(&self, index: usize) -> bool {
        self.git.operation.as_ref().is_some_and(|operation| {
            git::repository_scope(&operation.worktree)
                == git::repository_scope(&self.store.workspaces[index].path)
        }) || self.store.workspaces[index].sessions.iter().any(|s| {
            self.session_control.runs.contains_key(&s.id)
                || self.session_control.queue_saves.contains_key(&s.id)
        })
    }

    /// One turn per conversation, including aliases of the same native context.
    /// Other sessions may run even when they share the same working directory.
    pub(super) fn session_busy(&self, id: u64) -> bool {
        self.session_control.runs.contains_key(&id)
            || self.session(id).is_some_and(|target| {
                self.session_control.runs.keys().any(|running| {
                    self.session(*running).is_some_and(|session| {
                        session.provider == target.provider
                            && target.native_id.as_deref().is_some_and(|native| {
                                !native.is_empty() && session.native_id.as_deref() == Some(native)
                            })
                    })
                })
            })
    }

    /// Git refs are shared by linked worktrees, so Git writes coordinate with
    /// every CLI turn using this repository. Chat-to-chat concurrency is unchanged.
    pub(super) fn worktree_has_runs(&self, path: &Path) -> bool {
        if self.session_control.runs.is_empty() {
            return false;
        }
        let root = git::repository_scope(path);
        self.session_control
            .runs
            .values()
            .any(|run| git::repository_scope(&run.worktree) == root)
    }

    /// Resolve the destination before checking concurrency or changing drafts.
    pub(super) fn send_target(&self, append_user: bool) -> Option<u64> {
        self.selected_session.filter(|id| {
            !append_user
                || self
                    .session(*id)
                    .is_some_and(|session| session.provider == self.new_session_provider)
        })
    }

    /// Shared by keyboard sends and the composer button.
    pub(super) fn send_blocked(&self, append_user: bool) -> Option<&'static str> {
        if self.composer_save_pending() {
            return Some("正在保存消息，请稍候；草稿会保留到保存成功。");
        }
        if self
            .send_target(append_user)
            .and_then(|id| self.session(id))
            .is_some_and(|session| !session.transcript.loaded)
        {
            return Some("会话正文尚未加载，请等待加载完成或重试。");
        }
        if self
            .send_target(append_user)
            .is_some_and(|id| self.session_busy(id))
        {
            return Some("该会话已有任务运行，请等待完成或取消后发送。");
        }
        if self.git.operation.as_ref().is_some_and(|operation| {
            self.selected_workspace_path().is_some_and(|path| {
                git::repository_scope(&operation.worktree) == git::repository_scope(&path)
            })
        }) {
            return Some("该仓库正在执行 Git 操作，请等待完成后发送。");
        }
        None
    }

    pub(super) fn send_message(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_prompt(window, cx);
    }

    pub(super) fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        let target = self.composer_target();
        let text = self.input.read(cx).value().to_string();
        let references = self
            .current_files()
            .map(|files| files.references.as_slice())
            .unwrap_or_default();
        if let Some(target) = &target {
            let paths = references
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>();
            let (_, rejected) = file_ref::resolve(&paths, &target.workspace);
            if !rejected.is_empty() {
                self.composer_files
                    .entry(target.clone())
                    .or_default()
                    .feedback = Some(format!(
                    "文件已不可引用，请移除或重新添加后发送：{}",
                    rejection_summary(&rejected)
                ));
                cx.notify();
                return;
            }
        }
        let prompt = file_ref::compose_prompt(&text, references);
        if let Some(reason) = self.composer_send_blocked() {
            self.message = Some(reason.into());
            cx.notify();
            return;
        }
        let accepted = if let Some(id) = self.send_target(true)
            && self.session_control.runs.contains_key(&id)
        {
            self.enqueue_message(id, text, references.to_vec(), cx)
        } else {
            self.send_text(prompt, true, window, cx)
        };
        if !accepted && let Some(target) = target {
            // A failed send may have attempted to create or retarget a session.
            // Keep the draft and its files visible in their original conversation.
            self.selected_session = target.session;
        }
    }

    /// Starts one engine turn. `append_user` is false when regenerating: the
    /// user message is already in the transcript and must not be duplicated.
    pub(super) fn send_text(
        &mut self,
        prompt: String,
        append_user: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.closing || !self.writable {
            return false;
        }
        let Some(index) = self.selected_workspace else {
            self.message = Some("请先在左上角选择工作区。".into());
            cx.notify();
            return false;
        };
        if let Some(reason) = self.send_blocked(append_user) {
            self.message = Some(reason.into());
            cx.notify();
            return false;
        }
        if prompt.trim().is_empty() {
            return false;
        }
        if prompt.len() > 256 * 1024 {
            self.message = Some("单条消息最多 256 KiB。".into());
            cx.notify();
            return false;
        }
        // Resolve the target engine and validate the environment before creating
        // a session, so a failed send cannot leave an empty conversation behind.
        // A pending engine switch (composer engine differs from the selected
        // session's) retargets the message to a fresh session instead of
        // jumping to history. Regeneration always stays on its own session.
        if !append_user && self.selected_session.is_none() {
            return false;
        }
        let target = self.send_target(append_user);
        let composer = append_user.then(|| self.sent_composer(cx)).flatten();
        match self.start_turn(
            index,
            target,
            self.new_session_provider,
            prompt,
            append_user,
            cx,
        ) {
            Ok(id) => {
                if let Some(pending) = self
                    .session_control
                    .runs
                    .get_mut(&id)
                    .and_then(|run| run.pending_save.as_mut())
                {
                    pending.composer = composer;
                }
                self.input.read(cx).focus_handle(cx).focus(window, cx);
                self.message = Some("正在保存消息…".into());
                self.scroll_to_end();
                cx.notify();
                true
            }
            Err(error) => {
                let destination = target
                    .and_then(|id| self.session_notice_target(id))
                    .unwrap_or(NoticeTarget::Settings);
                self.notify_user(
                    Notice::new(
                        NoticeSource::Session,
                        NoticeLevel::Error,
                        "会话无法启动",
                        error.clone(),
                    )
                    .target(destination)
                    .presentation(NoticePresentation::History),
                    cx,
                );
                self.message = Some(error);
                cx.notify();
                false
            }
        }
    }

    /// Explicit destination, with no composer, focus, selection or scroll side effects.
    pub(super) fn start_turn(
        &mut self,
        index: usize,
        target: Option<u64>,
        provider: Provider,
        prompt: String,
        append_user: bool,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        if self.closing || !self.writable {
            return Err("当前无法发送消息。".into());
        }
        if target.is_some_and(|id| self.session_busy(id)) {
            return Err("该会话已有任务运行，请等待完成或取消后发送。".into());
        }
        if target
            .and_then(|id| self.session(id))
            .is_some_and(|session| !session.transcript.loaded)
        {
            if let Some(id) = target {
                self.request_transcript(id, cx);
            }
            return Err("会话正文尚未加载，请等待加载完成或重试。".into());
        }
        let cwd = self
            .store
            .workspaces
            .get(index)
            .ok_or("工作区已不存在。")?
            .path
            .clone();
        if self.git.operation.as_ref().is_some_and(|operation| {
            git::repository_scope(&operation.worktree) == git::repository_scope(&cwd)
        }) {
            return Err("该仓库正在执行 Git 操作，请等待完成后发送。".into());
        }
        if prompt.trim().is_empty() || prompt.len() > 256 * 1024 {
            return Err("消息不能为空，且最多 256 KiB。".into());
        }
        let provider = if let Some(id) = target {
            let session = self.session(id).ok_or("会话已不存在。")?;
            if session.native_id.is_none()
                && session
                    .messages
                    .iter()
                    .any(|m| m.kind == ItemKind::Assistant)
            {
                return Err("该会话缺少可靠的 CLI 会话标识，请新建会话，避免丢失上下文。".into());
            }
            session.provider
        } else {
            provider
        };
        let executable = cli::resolve(provider, &self.store.cli)
            .map_err(|error| {
                format!(
                    "{}：{error:#}。可在左下角「设置」中检查路径。",
                    provider.label()
                )
            })?
            .path;
        if !cwd.is_dir() {
            return Err("工作区目录已移动或不可访问。".into());
        }
        let worktree = git::worktree_root(&cwd);
        let prepared = target.is_none().then(|| self.prepare_session(provider));
        let selection = prepared.as_ref().map(ComposerSelection::from_session);
        let mut route = if let Some(session) = &prepared {
            let mut route = self
                .channel_route(provider, &session.channel)
                .map_err(|error| format!("无法使用所选 Provider：{error:#}"))?;
            if session.channel == ModelChannel::Cli {
                route.opencode_variant = session.opencode.variant.clone();
            }
            route
        } else {
            self.session_route(target.unwrap())
                .map_err(|error| format!("无法使用所选 Provider：{error:#}"))?
        };
        let before = target.and_then(|id| self.session(id).cloned());
        let id = if let Some(session) = prepared {
            let id = session.id;
            self.next_id += 1;
            self.store.workspaces[index].sessions.insert(0, session);
            id
        } else {
            target.unwrap()
        };
        let native_id = self
            .session(id)
            .and_then(|session| session.native_id.clone());
        let model = self.session(id).and_then(|session| session.model.clone());
        let permission = self
            .session(id)
            .map(|session| session.permission)
            .unwrap_or_default();
        let effort = self
            .session(id)
            .map(|s| s.reasoning_effort)
            .unwrap_or_default();
        let model = model.or_else(|| route.default_model.clone());
        let session = self.session_mut(id).unwrap();
        let prefix = next_turn_prefix(session);
        if append_user {
            if !session.messages.iter().any(|m| m.kind == ItemKind::User) {
                session.title = prompt
                    .lines()
                    .next()
                    .unwrap_or("新会话")
                    .trim()
                    .chars()
                    .take(26)
                    .collect();
            }
            session
                .messages
                .push(ChatItem::from(crate::domain::ChatItemData {
                    elapsed_ms: None,
                    key: format!("{prefix}-user"),
                    created_at: unix_now(),
                    kind: ItemKind::User,
                    text: prompt.clone(),
                    input: String::new(),
                    detail: String::new(),
                    status: Default::default(),
                    approval: None,
                    model: None,
                }));
        }
        session.interrupted = true;
        let receipt = match self.persist_confirmed() {
            Ok(receipt) => receipt,
            Err(error) => {
                self.restore_unsent_turn(id, before);
                return Err(error);
            }
        };
        let first_turn = append_user
            && !self.session(id).is_some_and(|session| {
                session
                    .messages
                    .iter()
                    .any(|message| message.kind == ItemKind::Assistant)
            });
        let naming_snippet = (self.store.ui.auto_rename
            && self.store.ui.naming_mode == NamingMode::FirstMessage
            && first_turn)
            .then(|| prompt.clone());
        let opencode_variant = route.opencode_variant.clone();
        let fresh_session = native_id.is_none();
        let context_window_fallback = self
            .session(id)
            .map(|session| {
                self.usage_context_window_fallback(provider, &session.channel, model.as_deref())
            })
            .unwrap_or(0);
        let (runtime, release) = ChatRuntime::start_after_save(crate::chat_runtime::StartRequest {
            provider,
            executable,
            cwd,
            prompt,
            native_id,
            model: model.clone(),
            permission,
            effort,
            route: std::mem::take(&mut route),
        });
        let usage = self
            .session(id)
            .map(|session| session.usage)
            .unwrap_or_default();
        self.session_control.runs.insert(
            id,
            ActiveRun::new(runtime, worktree, prefix, usage, model, effort),
        );
        if let Some(run) = self.session_control.runs.get_mut(&id) {
            run.status = "正在保存消息…".into();
            run.activity = run.status.clone();
            run.pending_save = Some(PendingTurn {
                receipt,
                release,
                before,
                composer: None,
                queued: None,
                naming_snippet,
                selection,
            });
            run.opencode_variant = opencode_variant;
            run.context_window_fallback = context_window_fallback;
            if fresh_session && provider == Provider::Codex {
                run.output_tokens_at_start = Some(0);
            }
        }
        Ok(id)
    }

    /// Retry the original prompt after clearing errors since the last visible reply.
    pub(super) fn retry_error(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || !self.writable {
            return;
        }
        let Some(id) = self.selected_session else {
            return;
        };
        let Some(session) = self.session(id) else {
            return;
        };
        if self.session_busy(id) {
            return;
        }
        let Some(prompt) = error_retry_prompt(session, index).map(str::to_owned) else {
            return;
        };
        let before = session.clone();
        let removed = clear_retry_errors(self.session_mut(id).unwrap());
        if self.send_text(prompt, false, window, cx) {
            if let Some(pending) = self
                .session_control
                .runs
                .get_mut(&id)
                .and_then(|run| run.pending_save.as_mut())
            {
                pending.before = Some(before);
            }
            for (_, item) in removed {
                self.expanded_errors.remove(&(id, item.key.clone()));
            }
        } else if let Some(session) = self.session_mut(id) {
            for (index, item) in removed {
                session.messages.insert(index, item);
            }
            cx.notify();
        }
    }

    /// Re-runs the latest exchange: keeps its user message, drops everything
    /// after it, and starts a fresh engine turn with the same prompt.
    pub(super) fn regenerate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        let Some(id) = self.selected_session else {
            return;
        };
        let Some(session) = self.session(id) else {
            return;
        };
        if self.session_busy(id)
            || session
                .messages
                .get(index)
                .is_none_or(|item| item.kind != ItemKind::Assistant)
            || session.messages[index + 1..]
                .iter()
                .any(|item| matches!(item.kind, ItemKind::User | ItemKind::Assistant))
        {
            return;
        }
        let Some(user_index) = session.messages[..index]
            .iter()
            .rposition(|item| item.kind == ItemKind::User)
        else {
            return;
        };
        if session.native_id.is_none() {
            self.message =
                Some("该会话缺少可靠的 CLI 会话标识，请新建会话，避免丢失上下文。".into());
            cx.notify();
            return;
        }
        let prompt = session.messages[user_index].text.clone();
        let before = session.clone();
        let removed = self
            .session_mut(id)
            .map(|session| session.messages.split_off(user_index + 1))
            .unwrap_or_default();
        if self.send_text(prompt, false, window, cx) {
            if let Some(pending) = self
                .session_control
                .runs
                .get_mut(&id)
                .and_then(|run| run.pending_save.as_mut())
            {
                pending.before = Some(before);
            }
        } else if let Some(session) = self.session_mut(id) {
            session.messages.extend(removed);
            cx.notify();
        }
    }

    pub(super) fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.poll_persistence(cx);
        self.poll_pending_sends(window, cx);
        if self.closing
            && let Some(error) = self
                .session_control
                .runs
                .values()
                .find_map(|run| run.cleanup_error.clone())
                .or_else(|| {
                    self.session_control
                        .namings
                        .values()
                        .find_map(|naming| naming.cleanup_error.clone())
                })
                .or_else(|| {
                    self.git
                        .generation
                        .as_ref()
                        .and_then(|generation| generation.cleanup_error.clone())
                })
        {
            self.closing = false;
            self.message = Some(error);
            cx.notify();
        }
        if !self.search_query.trim().is_empty()
            && self.search_ui.revision != self.session_control.global_revision
            && self.search_ui.requested.elapsed() >= Duration::from_millis(500)
        {
            self.refresh_search(cx, false);
        }
        self.poll_updates(window, cx);
        self.publish_update_notification(cx);
        self.poll_notifications(window, cx);
        self.sync_plugin_context();
        #[cfg(conduit_shell_desktop)]
        self.refresh_shell_context(cx);
        if self.updater.exiting || self.updater.preparing() {
            return;
        }
        self.poll_files(cx);
        self.poll_git_navigation(cx);
        // Release the startup page once the initial history import merged
        // and its minimum display passed -- or when the timeout gives up on
        // a stalled scan -- so the first main-layout frame is complete.
        let startup_released = self.startup.as_ref().is_some_and(|startup| {
            let elapsed = startup.started.elapsed();
            (startup.imports_done && elapsed >= STARTUP_MIN_DISPLAY) || elapsed >= STARTUP_TIMEOUT
        });
        if startup_released {
            if self.startup.as_ref().is_some_and(|s| !s.imports_done) {
                self.session_control.cancel_imports();
                self.history_loading.clear();
                self.message = Some("历史读取超时，已保留现有对话；可重新导入。".into());
                self.notify_user(
                    Notice::new(
                        NoticeSource::App,
                        NoticeLevel::Warning,
                        "历史读取超时",
                        "已保留现有对话，可在工作区重新导入历史。",
                    ),
                    cx,
                );
            }
            self.startup = None;
            cx.notify();
        }
        self.poll_opencode_models(cx);
        self.poll_extensions(window, cx);
        // Refresh session ages every minute while idle, and running elapsed
        // labels once a second; avoid repainting every 33 ms poll otherwise.
        let label_interval = if self.session_control.runs.is_empty() {
            60
        } else {
            1
        };
        if self.last_live_tick.elapsed() >= Duration::from_secs(label_interval) {
            self.last_live_tick = Instant::now();
            cx.notify();
        }
        if self.store.ui.performance_monitor && self.performance.read(cx).sample_due() {
            // Notify only the diagnostics view; idle chats need no extra rebuild.
            self.performance.update(cx, |monitor, cx| {
                monitor.sample();
                cx.notify();
            });
        }
        if self.dirty && self.last_save.elapsed() >= Duration::from_secs(1) {
            self.persist();
            cx.notify();
        }
        let mut completed = vec![];
        for (provider, probe) in &self.probes {
            match probe.events.try_recv() {
                Ok(result) => completed.push((*provider, probe.path.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => completed.push((
                    *provider,
                    probe.path.clone(),
                    Err("检测线程已退出，请重新检测".into()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        for (provider, path, result) in completed {
            self.probes.remove(&provider);
            let diagnostic_level = if result.is_err() {
                NoticeLevel::Error
            } else {
                NoticeLevel::Success
            };
            let text = if cli::resolve(provider, &self.store.cli)
                .ok()
                .is_none_or(|cli| cli.path != path)
            {
                "配置已变更，请重新检测。".into()
            } else {
                match result {
                    Ok(info) => format!(
                        "版本 {} · {}",
                        info.version,
                        if info.verified_version {
                            "与本项目已验证版本一致"
                        } else {
                            "尚未经过完整回归"
                        }
                    ),
                    Err(error) if error == "检测已取消" => error,
                    Err(error) => format!("检测失败：{error}"),
                }
            };
            self.notify_user(
                Notice::new(
                    NoticeSource::Settings,
                    diagnostic_level,
                    format!("{} CLI 检测", provider.label()),
                    text.clone(),
                )
                .target(NoticeTarget::Settings)
                .presentation(NoticePresentation::History),
                cx,
            );
            self.diagnostics.insert(provider, text);
            cx.notify();
        }
        self.poll_branches(cx);
        self.poll_git(window, cx);
        self.poll_git_actions(window, cx);
        self.poll_session_namings(cx);
        if self
            .message_focus
            .as_ref()
            .is_some_and(|(_, _, started)| started.elapsed() > Duration::from_secs(2))
        {
            self.message_focus = None;
            cx.notify();
        }
        self.poll_streams(window, cx);
        if self.closing
            && !self.updater.preparing()
            && self.session_control.runs.is_empty()
            && self.session_control.queue_saves.is_empty()
            && self.probes.is_empty()
            && self.opencode_model_probes.is_empty()
            && self.git.operation.is_none()
            && self.git.generation.is_none()
            && self.session_control.namings.is_empty()
            && !self.dirty
            && !self.persistence.busy()
            && self.persistence.error.is_none()
            && self.notifications.finish()
        {
            self.discard_composer_attachments();
            window.remove_window();
            cx.quit();
        }
    }

    pub(super) fn toggle_approval_detail(
        &mut self,
        session: u64,
        id: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(run) = self.session_control.runs.get_mut(&session)
            && !run.expanded_approvals.remove(&id)
        {
            run.expanded_approvals.insert(id);
        }
        cx.notify();
    }

    pub(super) fn decide(
        &mut self,
        session: u64,
        request: String,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .session_control
            .runs
            .get(&session)
            .and_then(|run| run.approvals.front())
            .is_some_and(|approval| approval.id == request)
            && self.submit_approval(session, &request, allow, cx)
        {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Exact-ticket submission shared by chat and notifications, without navigation or focus changes.
    pub(super) fn submit_approval(
        &mut self,
        session: u64,
        request: &str,
        allow: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.closing {
            return false;
        }
        let Some(run) = self.session_control.runs.get_mut(&session) else {
            return false;
        };
        if run.runtime.is_cancelled()
            || !run.approvals.get(request).is_some_and(|approval| {
                !allow || (approval.allow_once && !approval.is_user_input())
            })
            || !run.approvals.begin(request)
        {
            return false;
        }
        let sent = run.runtime.try_decide(request.to_owned(), allow);
        if sent {
            run.status = "正在提交审批…".into();
        } else {
            run.approvals.remove(request);
            run.status = "审批提交失败，运行连接已关闭；请查看会话。".into();
            self.finish_notification_approval(
                session,
                request,
                crate::notifications::ApprovalOutcome::Expired,
                cx,
            );
        }
        self.refresh_notifications(cx);
        sent
    }
}
