//! Chat submission, stream polling, cancellation, and approvals.

use super::*;

impl ConduitApp {
    pub(super) fn workspace_busy(&self, index: usize) -> bool {
        self.git_operation.as_ref().is_some_and(|operation| {
            git::repository_scope(&operation.worktree)
                == git::repository_scope(&self.store.workspaces[index].path)
        }) || self.store.workspaces[index]
            .sessions
            .iter()
            .any(|s| self.runs.contains_key(&s.id))
    }

    /// One turn per conversation, including aliases of the same native context.
    /// Other sessions may run even when they share the same working directory.
    pub(super) fn session_busy(&self, id: u64) -> bool {
        self.runs.contains_key(&id)
            || self.session(id).is_some_and(|target| {
                self.runs.keys().any(|running| {
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
        if self.runs.is_empty() {
            return false;
        }
        let root = git::repository_scope(path);
        self.runs
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
        if self
            .send_target(append_user)
            .is_some_and(|id| self.session_busy(id))
        {
            return Some("该会话已有任务运行，请等待完成或取消后发送。");
        }
        if self.git_operation.as_ref().is_some_and(|operation| {
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
        if self.send_text(prompt, true, window, cx) {
            // The sent prompt is now the newest history entry; stop any
            // browsing so the next Up recalls it from the top.
            self.prompt_history = None;
            if let Some(target) = target {
                self.composer_files.remove(&target);
                if let Some(id) = target.session {
                    self.drafts.remove(&id);
                } else {
                    self.workspace_drafts.remove(&target.workspace);
                }
            }
        } else if let Some(target) = target {
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
        let worktree = git::worktree_root(&self.store.workspaces[index].path);
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
        if append_user {
            let retarget = self.selected_session.is_some_and(|id| {
                self.session(id)
                    .is_some_and(|session| session.provider != self.new_session_provider)
            });
            if retarget && let Some(id) = self.selected_session {
                self.drafts
                    .insert(id, self.input.read(cx).value().to_string());
                self.selected_session = None;
            }
        } else if self.selected_session.is_none() {
            return false;
        }
        let provider = match self.selected_session {
            Some(id) => self.session(id).unwrap().provider,
            None => self.new_session_provider,
        };
        if let Some(id) = self.selected_session {
            let session = self.session(id).unwrap();
            if session.native_id.is_none()
                && session
                    .messages
                    .iter()
                    .any(|m| m.kind == ItemKind::Assistant)
            {
                self.message =
                    Some("该会话缺少可靠的 CLI 会话标识，请新建会话，避免丢失上下文。".into());
                cx.notify();
                return false;
            }
        }
        let executable = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(error) => {
                self.message = Some(format!(
                    "{}：{error:#}。可在左下角「设置」中检查路径。",
                    provider.label()
                ));
                cx.notify();
                return false;
            }
        };
        let cwd = self.store.workspaces[index].path.clone();
        if !cwd.is_dir() {
            self.message = Some("工作区目录已移动或不可访问。".into());
            cx.notify();
            return false;
        }
        // Typing in a workspace without a selected session starts one with the
        // last used engine instead of asking the user to click "新建会话".
        let id = match self.selected_session {
            Some(id) => id,
            None => match self.create_session(index, provider, cx) {
                Some(id) => id,
                None => return false,
            },
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
        let route = match self.session_route(id) {
            Ok(route) => route,
            Err(error) => {
                self.message = Some(format!("无法使用所选 Provider：{error:#}"));
                cx.notify();
                return false;
            }
        };
        let model = model.or_else(|| route.default_model.clone());
        let session = self.session_mut(id).unwrap();
        let prefix = format!("turn-{}", session.messages.len());
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
            session.messages.push(ChatItem {
                elapsed_ms: None,
                key: format!("{prefix}-user"),
                created_at: unix_now(),
                kind: ItemKind::User,
                text: prompt.clone(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            });
        }
        session.interrupted = true;
        if !self.persist() {
            let session = self.session_mut(id).unwrap();
            if append_user {
                session.messages.pop();
            }
            session.interrupted = false;
            cx.notify();
            return false;
        }
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
        let runtime = ChatRuntime::start_with_options(
            provider,
            executable,
            cwd,
            prompt,
            native_id,
            model.clone(),
            permission,
            effort,
            route,
        );
        let usage = self
            .session(id)
            .map(|session| session.usage)
            .unwrap_or_default();
        self.plans.remove(&id);
        self.unread_completed.remove(&id);
        self.runs.insert(
            id,
            ActiveRun::new(runtime, worktree, prefix, usage, model, effort),
        );
        if let Some(run) = self.runs.get_mut(&id) {
            run.opencode_variant = opencode_variant;
            run.context_window_fallback = context_window_fallback;
            if fresh_session && provider == Provider::Codex {
                run.output_tokens_at_start = Some(0);
            }
        }
        if append_user {
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.drafts.remove(&id);
        }
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        self.message = None;
        if let Some(snippet) = naming_snippet {
            self.start_session_naming(id, &snippet, cx);
        }
        self.scroll_to_end();
        cx.notify();
        true
    }

    /// Retry the failed exchange in place, preserving the original user prompt.
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
        if self.runs.contains_key(&id)
            || session
                .messages
                .get(index)
                .is_none_or(|item| item.kind != ItemKind::Error)
            || session.messages[index + 1..]
                .iter()
                .any(|item| matches!(item.kind, ItemKind::User | ItemKind::Assistant))
            || session.native_id.is_none()
        {
            return;
        }
        let Some(user_index) = session.messages[..index]
            .iter()
            .rposition(|item| item.kind == ItemKind::User)
        else {
            return;
        };
        let prompt = session.messages[user_index].text.clone();
        let removed = self
            .session_mut(id)
            .map(|session| session.messages.split_off(user_index + 1))
            .unwrap_or_default();
        if !self.send_text(prompt, false, window, cx)
            && let Some(session) = self.session_mut(id)
        {
            session.messages.extend(removed);
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
        if self.runs.contains_key(&id)
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
        let removed = self
            .session_mut(id)
            .map(|session| session.messages.split_off(user_index + 1))
            .unwrap_or_default();
        if !self.send_text(prompt, false, window, cx)
            && let Some(session) = self.session_mut(id)
        {
            session.messages.extend(removed);
            cx.notify();
        }
    }

    pub(super) fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Release the startup page once the initial history import merged
        // and its minimum display passed -- or when the timeout gives up on
        // a stalled scan -- so the first main-layout frame is complete.
        let startup_released = self.startup.as_ref().is_some_and(|startup| {
            let elapsed = startup.started.elapsed();
            (startup.imports_done && elapsed >= STARTUP_MIN_DISPLAY) || elapsed >= STARTUP_TIMEOUT
        });
        if startup_released {
            self.startup = None;
            cx.notify();
        }
        self.poll_opencode_models(cx);
        // Refresh session ages every minute while idle, and running elapsed
        // labels once a second; avoid repainting every 33 ms poll otherwise.
        let label_interval = if self.runs.is_empty() { 60 } else { 1 };
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
            self.diagnostics.insert(provider, text);
            cx.notify();
        }
        self.poll_branches(cx);
        self.poll_git(cx);
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
        let navigating = std::mem::take(&mut self.pending_message_jump);
        let follow = self.scroll.is_scrolled_to_end().unwrap_or_else(|| {
            self.scroll.logical_scroll_top().item_ix >= self.scroll.item_count()
        });
        // Re-render when the scroll position crosses the "at the end" threshold
        // so the floating arrow appears/disappears without polling every frame.
        if !navigating && follow != self.at_bottom {
            self.at_bottom = follow;
            cx.notify();
        }
        let mut incoming = vec![];
        for (id, run) in &self.runs {
            for event in run.runtime.events.try_iter().take(1000) {
                incoming.push((*id, run.prefix.clone(), event));
            }
        }
        if self.store.ui.performance_monitor {
            self.performance
                .update(cx, |monitor, _| monitor.record_events(incoming.len()));
        }
        let mut changed = false;
        let mut terminal = false;
        let mut visible = false;
        let mut dirty_messages = Vec::new();
        let mut structural_change = false;
        let mut completed_turns = Vec::new();
        for (id, prefix, event) in incoming {
            changed = true;
            visible |= Some(id) == self.selected_session;
            if Some(id) == self.selected_session
                && matches!(
                    event,
                    Event::Approval(_)
                        | Event::ApprovalCancelled(_)
                        | Event::ApprovalResolved { .. }
                        | Event::ReasoningSummary { .. }
                        | Event::Finished { .. }
                )
            {
                structural_change = true;
            }
            match event {
                Event::OpenCodeConfig(options) => {
                    let variant = if let Some(session) = self.session_mut(id) {
                        session.opencode.options = options;
                        session.opencode.effective_variant().map(str::to_owned)
                    } else {
                        None
                    };
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.opencode_variant = variant;
                    }
                    self.dirty = true;
                    cx.emit(crate::providers::ProviderCatalogChanged);
                }
                Event::Plan { steps } => {
                    let steps = steps
                        .into_iter()
                        .filter_map(|(text, status)| {
                            let text = text.trim().to_string();
                            (!text.is_empty()).then(|| PlanStep {
                                text,
                                status: PlanStatus::from_wire(&status),
                            })
                        })
                        .collect::<Vec<_>>();
                    if steps.is_empty() {
                        self.plans.remove(&id);
                    } else {
                        self.plans.insert(id, steps);
                    }
                }
                Event::Usage(mut usage) => {
                    if let Some(run) = self.runs.get_mut(&id) {
                        if (usage.output_tokens < run.output_tokens_at_start.unwrap_or(0)
                            || usage.output_tokens < run.usage.output_tokens)
                            && usage.turn_output_tokens.is_none()
                        {
                            // A counter reset is not a new speed measurement.
                            run.output_tokens_at_start = None;
                        }
                        let elapsed =
                            run.turn_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        usage = usage.sample_turn(run.output_tokens_at_start, elapsed);
                        if usage.context_window == 0 && run.context_window_fallback > 0 {
                            usage.context_window = run.context_window_fallback;
                            usage.context_window_estimated = true;
                        }
                        run.usage = usage;
                    }
                    if let Some(session) = self.session_mut(id) {
                        session.usage = usage;
                    }
                }
                Event::Approval(approval) => {
                    let accepted = self
                        .runs
                        .get(&id)
                        .is_some_and(|run| !self.closing && !run.runtime.is_cancelled());
                    if !accepted {
                        continue;
                    }
                    if let Some(session) = self.session_mut(id) {
                        session.record_tool_approval(
                            &prefix,
                            &approval.id,
                            &approval.title,
                            &approval.detail,
                            "等待中",
                        );
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.status = "等待你的审批".into();
                        run.activity = "等待你的审批".into();
                        run.approvals.push(approval);
                    }
                }
                Event::ApprovalCancelled(request) => {
                    let mut approval = None;
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.expanded_approvals.remove(&request);
                        approval = run.approvals.remove(&request);
                        if run.approvals.is_empty() && !run.runtime.is_cancelled() {
                            run.status = "正在执行…".into();
                            run.activity = "正在继续处理…".into();
                        }
                    }
                    if let (Some(approval), Some(session)) = (approval, self.session_mut(id)) {
                        session.record_tool_approval(
                            &prefix,
                            &request,
                            &approval.title,
                            &approval.detail,
                            "已取消",
                        );
                    }
                }
                Event::ApprovalResolved { id: request, allow } => {
                    let approval = self.runs.get_mut(&id).and_then(|run| {
                        run.expanded_approvals.remove(&request);
                        let approval = run.approvals.remove(&request);
                        run.status = if run.approvals.is_empty() {
                            "正在执行…"
                        } else {
                            "等待你的审批"
                        }
                        .into();
                        run.activity = if run.approvals.is_empty() {
                            "正在继续处理…"
                        } else {
                            "等待你的审批"
                        }
                        .into();
                        approval
                    });
                    if let (Some(approval), Some(session)) = (approval, self.session_mut(id)) {
                        session.record_tool_approval(
                            &prefix,
                            &request,
                            &approval.title,
                            &approval.detail,
                            if allow { "已允许" } else { "已拒绝" },
                        );
                    }
                }
                Event::Finished { error, cancelled } => {
                    let finished_run = self.runs.remove(&id);
                    let elapsed_ms = finished_run.as_ref().map(|run| {
                        run.turn_started.elapsed().as_millis().min(u64::MAX as u128) as u64
                    });
                    let finished_usage = finished_run
                        .as_ref()
                        .filter(|run| run.usage.turn_elapsed_ms.is_some())
                        .map(|run| {
                            let mut usage = run.usage;
                            usage.turn_elapsed_ms = elapsed_ms;
                            usage
                        });
                    self.unread_completed.remove(&id);
                    terminal = true;
                    if error.is_none() && !cancelled {
                        completed_turns.push(id);
                        if self.selected_session != Some(id) {
                            self.unread_completed.insert(id);
                        }
                    }
                    if let Some(session) = self.session_mut(id) {
                        session.interrupted = false;
                        if let Some(usage) = finished_usage {
                            session.usage = usage;
                        }
                        // Scope timing to this run, including partial replies on failure/cancel.
                        let item_prefix = format!("{prefix}-");
                        for item in &mut session.messages {
                            if item.kind == ItemKind::Assistant
                                && item.key.starts_with(&item_prefix)
                            {
                                item.elapsed_ms = elapsed_ms;
                            }
                        }
                        finish_tool_cards(
                            session,
                            &prefix,
                            if cancelled {
                                "已取消"
                            } else {
                                "未返回结果"
                            },
                        );
                        // Do not duplicate a failure already surfaced by the stream.
                        let already_shown = error.as_deref().is_some_and(|error| {
                            session
                                .messages
                                .iter()
                                .rev()
                                .filter(|item| {
                                    matches!(
                                        item.kind,
                                        ItemKind::Assistant | ItemKind::Tool | ItemKind::Error
                                    )
                                })
                                .take(5)
                                .any(|item| same_failure(&item.text, error))
                        });
                        if cancelled || (error.is_some() && !already_shown) {
                            let failed = error.is_some();
                            session.messages.push(ChatItem {
                                elapsed_ms: None,
                                key: format!("{prefix}-finish"),
                                created_at: unix_now(),
                                kind: if failed {
                                    ItemKind::Error
                                } else {
                                    ItemKind::Notice
                                },
                                text: if cancelled {
                                    "已取消；不会自动重试。".into()
                                } else {
                                    format!(
                                        "执行失败：{}",
                                        friendly_failure(error.as_deref().unwrap_or_default())
                                    )
                                },
                                input: String::new(),
                                detail: error.clone().unwrap_or_default(),
                                status: String::new(),
                                approval: None,
                                model: None,
                            });
                        }
                    }
                }
                Event::NativeId(native_id) => {
                    if let Some(session) = self.session_mut(id) {
                        session.native_id = Some(native_id);
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.status = "正在执行…".into();
                        run.activity = "正在思考…".into();
                    }
                    self.dirty = true;
                    self.persist();
                }
                other => {
                    // Starting a reply also collapses earlier tool rows. Their
                    // cached heights must change before the turn finishes.
                    if Some(id) == self.selected_session
                        && matches!(&other, Event::Delta { text, .. } | Event::Text { text, .. }
                            if !text.trim().is_empty())
                        && self.session(id).is_some_and(|session| {
                            session
                                .messages
                                .iter()
                                .rposition(is_groupable_activity)
                                .is_some_and(|index| !tool_group_has_reply(session, index + 1))
                        })
                    {
                        structural_change = true;
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        let activity = match &other {
                            Event::Delta { .. } | Event::Text { .. } => "正在生成回复…".to_string(),
                            Event::ToolInput { .. } => "正在准备工具调用…".to_string(),
                            Event::Tool { title, status, .. } => match status.as_str() {
                                "运行中" if !title.is_empty() => {
                                    format!("正在调用工具：{title}")
                                }
                                "运行中" => "正在调用工具…".to_string(),
                                "输出中" => "正在接收工具输出…".to_string(),
                                _ => "正在继续处理…".to_string(),
                            },
                            _ => String::new(),
                        };
                        if !activity.is_empty() {
                            run.activity = activity;
                        }
                    }
                    let mut index = self
                        .runs
                        .get_mut(&id)
                        .map(|run| std::mem::take(&mut run.message_index))
                        .unwrap_or_default();
                    let turn_model = self.runs.get(&id).and_then(|run| run.model.clone());
                    let selected = Some(id) == self.selected_session;
                    if let Some(session) = self.session_mut(id) {
                        let old_len = session.messages.len();
                        let shape = |item: &ChatItem| {
                            (is_empty_chat_placeholder(item), is_groupable_activity(item))
                        };
                        let previous_shape = match &other {
                            Event::Delta { key, .. }
                            | Event::Text { key, .. }
                            | Event::Tool { key, .. }
                            | Event::ToolInput { key, .. } => index
                                .find(&session.messages, &format!("{prefix}-{key}"))
                                .map(|position| shape(&session.messages[position])),
                            _ => None,
                        };
                        if let Some(changed_index) =
                            apply_event_indexed(session, &prefix, other, &mut index)
                        {
                            if let Some(model) = &turn_model {
                                attach_turn_model(session, &prefix, model);
                            }
                            if selected {
                                // New tools can merge earlier rows into a group;
                                // text arriving in an empty block can split it.
                                // A new reply segment also removes the footer
                                // and its reserved height from the previous one.
                                // Invalidate those rows as well as the event's row.
                                let current_shape = shape(&session.messages[changed_index]);
                                structural_change |= previous_shape
                                    .is_some_and(|previous| previous != current_shape)
                                    || (changed_index >= old_len
                                        && (current_shape.1
                                            || (session.messages[changed_index].kind
                                                == ItemKind::Assistant
                                                && !current_shape.0)));
                                dirty_messages.push(changed_index);
                            }
                        }
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.message_index = index;
                    }
                }
            }
        }
        if self.store.ui.auto_rename && self.store.ui.naming_mode == NamingMode::Latest {
            for id in completed_turns {
                if let Some(excerpt) = self.session(id).and_then(latest_naming_excerpt) {
                    self.start_session_naming(id, &excerpt, cx);
                }
            }
        }
        if changed {
            if structural_change {
                self.scroll.remeasure();
            } else {
                dirty_messages.sort_unstable();
                dirty_messages.dedup();
                for index in dirty_messages {
                    if index < self.scroll.item_count() {
                        self.scroll.remeasure_items(index..index + 1);
                    }
                }
            }
            self.dirty = true;
            if terminal || self.last_save.elapsed() > Duration::from_secs(1) {
                self.persist();
            }
            cx.notify();
            if follow && visible && !navigating && self.message_focus.is_none() {
                self.scroll_to_end();
            }
            // A finished turn can change workspace files; keep an open Git
            // panel current without requiring the user to press 刷新.
            if terminal {
                self.refresh_branches();
                if self.details && self.detail_view == DetailView::Git && self.git_probe.is_none() {
                    self.refresh_git(cx);
                }
            }
        }
        if self.closing
            && self.runs.is_empty()
            && self.probes.is_empty()
            && self.opencode_model_probes.is_empty()
            && self.git_operation.is_none()
            && self.commit_generation.is_none()
        {
            self.persist();
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
        if let Some(run) = self.runs.get_mut(&session)
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
        if !self.closing
            && let Some(run) = self.runs.get_mut(&session)
            && !run.runtime.is_cancelled()
            && run
                .approvals
                .front()
                .is_some_and(|a| a.id == request && (!allow || a.allow_once))
            && run.approvals.begin(&request)
        {
            run.runtime.decide(request, allow);
            run.status = "正在提交审批…".into();
            self.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }
}
