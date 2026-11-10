//! Chat submission, stream polling, cancellation, and approvals.

use super::*;

pub(super) mod saves;
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
        .rev()
        .take_while(|item| !item.key.ends_with("-command"))
        .find(|item| item.kind == ItemKind::User)
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
        self.terminals
            .workspace_busy(&self.store.workspaces[index].path)
            || self.git.operation.as_ref().is_some_and(|operation| {
                git::repository_scope(&operation.worktree)
                    == git::repository_scope(&self.store.workspaces[index].path)
            })
            || self.store.workspaces[index].sessions.iter().any(|s| {
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
        if self
            .send_target(append_user)
            .is_some_and(|id| self.session_fork_pending(id))
        {
            return Some(conduit::i18n::text_static("branching.pending"));
        }
        if self.cli_installing() {
            return Some(conduit::i18n::text_static(
                "runtime.cli_update_in_progress_wait_for_it",
            ));
        }
        if self.composer_save_pending() {
            return Some(conduit::i18n::text_static(
                "queue.saving_message_please_wait_your_draft_will",
            ));
        }
        if self
            .send_target(append_user)
            .and_then(|id| self.session(id))
            .is_some_and(|session| !session.transcript.loaded)
        {
            return Some(conduit::i18n::text_static(
                "runtime.session_messages_have_not_loaded_wait_or",
            ));
        }
        if self
            .send_target(append_user)
            .is_some_and(|id| self.session_busy(id))
        {
            return Some(conduit::i18n::text_static(
                "runtime.a_task_is_already_running_in_this",
            ));
        }
        if self.git.operation.as_ref().is_some_and(|operation| {
            self.selected_workspace_path().is_some_and(|path| {
                git::repository_scope(&operation.worktree) == git::repository_scope(&path)
            })
        }) {
            return Some(conduit::i18n::text_static(
                "runtime.a_git_operation_is_running_in_this",
            ));
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
        if self.composer_is_composing(window, cx) {
            return;
        }
        if self.closing || !self.writable || self.composer_save_pending() {
            return;
        }
        if self.submit_chat_command(window, cx) {
            return;
        }
        self.send_ordinary_prompt(window, cx);
    }

    pub(super) fn send_ordinary_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .send_target(true)
            .is_some_and(|id| self.session_fork_pending(id))
        {
            self.message = Some(conduit::i18n::text("branching.pending"));
            cx.notify();
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
                    .feedback = Some(conduit::i18n::format(
                    "runtime.files_are_no_longer_available_remove_or",
                    &[("arg0", rejection_summary(&rejected).to_string())],
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
        self.send_input(prompt.into(), append_user, window, cx)
    }

    pub(super) fn send_input(
        &mut self,
        input: crate::commands::TurnInput,
        append_user: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let prompt = &input.text;
        if self.closing || !self.writable {
            return false;
        }
        let Some(index) = self.selected_workspace else {
            self.message = Some(
                conduit::i18n::text_static("composer.select_a_workspace_in_the_upper_left").into(),
            );
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
            self.message = Some(
                conduit::i18n::text_static("runtime.each_message_may_be_no_larger_than").into(),
            );
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
        let mut composer = append_user.then(|| self.sent_composer(cx)).flatten();
        if matches!(input.intent, crate::commands::RequestIntent::Discover)
            && let Some(composer) = &mut composer
        {
            composer.preserve = true;
        }
        match self.start_turn(
            index,
            target,
            self.new_session_provider,
            input,
            append_user,
            cx,
        ) {
            Ok(id) => {
                if let Some(run) = self.session_control.runs.get_mut(&id)
                    && matches!(run.intent, crate::commands::RequestIntent::Command(_))
                {
                    run.command_draft = composer.clone();
                }
                if let Some(pending) = self
                    .session_control
                    .runs
                    .get_mut(&id)
                    .and_then(|run| run.pending_save.as_mut())
                {
                    pending.composer = composer;
                }
                self.input.read(cx).focus_handle(cx).focus(window, cx);
                // Save progress belongs to the run activity, not the error banner.
                self.message = None;
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
                        conduit::i18n::text_static("runtime.unable_to_start_session"),
                        error,
                    )
                    .target(destination),
                    cx,
                );
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
        input: impl Into<crate::commands::TurnInput>,
        append_user: bool,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        let crate::commands::TurnInput {
            text: prompt,
            intent,
        } = input.into();
        if let crate::commands::RequestIntent::Command(call) = &intent
            && self.command_context_for(index, target, provider).as_ref() != Some(call.context())
        {
            return Err(conduit::i18n::text("commands.stale"));
        }
        if self.closing || !self.writable {
            return Err(
                conduit::i18n::text_static("runtime.unable_to_send_a_message_right_now").into(),
            );
        }
        if target.is_some_and(|id| self.session_busy(id)) {
            return Err(
                conduit::i18n::text_static("runtime.a_task_is_already_running_in_this").into(),
            );
        }
        if target
            .and_then(|id| self.session(id))
            .is_some_and(|session| !session.transcript.loaded)
        {
            if let Some(id) = target {
                self.request_transcript(id, cx);
            }
            return Err(conduit::i18n::text_static(
                "runtime.session_messages_have_not_loaded_wait_or",
            )
            .into());
        }
        let cwd = self
            .store
            .workspaces
            .get(index)
            .ok_or(conduit::i18n::text_static(
                "runtime.the_workspace_no_longer_exists",
            ))?
            .path
            .clone();
        if self.git.operation.as_ref().is_some_and(|operation| {
            git::repository_scope(&operation.worktree) == git::repository_scope(&cwd)
        }) {
            return Err(
                conduit::i18n::text_static("runtime.a_git_operation_is_running_in_this").into(),
            );
        }
        if prompt.trim().is_empty() || prompt.len() > 256 * 1024 {
            return Err(conduit::i18n::text_static(
                "queue.messages_must_be_nonempty_and_no_larger",
            )
            .into());
        }
        let provider = if let Some(id) = target {
            let session = self.session(id).ok_or(conduit::i18n::text_static(
                "runtime.the_session_no_longer_exists",
            ))?;
            if let crate::commands::RequestIntent::Fork(request) = &intent {
                if crate::branching::request(session).as_ref() != Ok(request) {
                    return Err(conduit::i18n::text("branching.changed"));
                }
            } else if crate::branching::pending(session) {
                return Err(conduit::i18n::text("branching.pending"));
            }
            if session.native_id.is_none()
                && !matches!(intent, crate::commands::RequestIntent::Fork(_))
                && session
                    .messages
                    .iter()
                    .any(|m| m.kind == ItemKind::Assistant)
            {
                return Err(conduit::i18n::text_static(
                    "runtime.this_session_has_no_reliable_cli_session",
                )
                .into());
            }
            session.provider
        } else {
            provider
        };
        if self.cli_installing() {
            return Err(
                conduit::i18n::text_static("runtime.cli_update_in_progress_wait_for_it").into(),
            );
        }
        let executable = cli::resolve_for_launch(provider, &self.store.cli)
            .map_err(|error| {
                conduit::i18n::format(
                    "runtime.open_settings_ai_engines_to_check_the",
                    &[
                        ("arg0", provider.label().to_string()),
                        ("error", format!("{:#}", error)),
                    ],
                )
            })?
            .path;
        if provider.descriptor().commands.is_some() {
            self.remember_command_executable(provider, &executable)?;
            if let crate::commands::RequestIntent::Command(call) = &intent
                && self.command_context_for(index, target, provider).as_ref()
                    != Some(call.context())
            {
                return Err(conduit::i18n::text("commands.stale"));
            }
        }
        if !cwd.is_dir() {
            return Err(conduit::i18n::text_static(
                "runtime.the_workspace_directory_has_moved_or_is",
            )
            .into());
        }
        let worktree = git::worktree_root(&cwd);
        let prepared = target.is_none().then(|| self.prepare_session(provider));
        let selection = prepared.as_ref().map(ComposerSelection::from_session);
        let mut route = if let Some(session) = &prepared {
            let mut route = self
                .channel_route(provider, &session.channel)
                .map_err(|error| {
                    conduit::i18n::format(
                        "runtime.unable_to_use_the_selected_provider",
                        &[("error", format!("{:#}", error))],
                    )
                })?;
            if session.channel == ModelChannel::Cli {
                route.acp_variant = session.opencode.variant.clone();
            }
            route
        } else {
            self.session_route(target.unwrap()).map_err(|error| {
                conduit::i18n::format(
                    "runtime.unable_to_use_the_selected_provider",
                    &[("error", format!("{:#}", error))],
                )
            })?
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
                    .unwrap_or(conduit::i18n::text_static("files.new_session"))
                    .trim()
                    .chars()
                    .take(26)
                    .collect();
            }
            session
                .messages
                .push(ChatItem::from(crate::domain::ChatItemData {
                    elapsed_ms: None,
                    key: format!(
                        "{prefix}-{}",
                        if matches!(intent, crate::commands::RequestIntent::Message) {
                            "user"
                        } else {
                            "command"
                        }
                    ),
                    created_at: unix_now(),
                    kind: if matches!(intent, crate::commands::RequestIntent::Message) {
                        ItemKind::User
                    } else {
                        ItemKind::Notice
                    },
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
            && matches!(intent, crate::commands::RequestIntent::Message)
            && self.store.ui.naming_mode == NamingMode::FirstMessage
            && first_turn)
            .then(|| prompt.clone());
        let acp_variant = route.acp_variant.clone();
        let fresh_session = native_id.is_none();
        let context_window_fallback = self
            .session(id)
            .map(|session| {
                self.usage_context_window_fallback(provider, &session.channel, model.as_deref())
            })
            .unwrap_or(0);
        let command_executable = matches!(
            intent,
            crate::commands::RequestIntent::Command(_) | crate::commands::RequestIntent::Fork(_)
        )
        .then(|| (provider, executable.clone()));
        let (runtime, release) = ChatRuntime::start_after_save(crate::chat_runtime::StartRequest {
            intent: intent.clone(),
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
        let usage_service = self
            .session(id)
            .map(|s| match &s.channel {
                ModelChannel::Cli => "cli".to_owned(),
                ModelChannel::Provider(id) => format!("provider:{id}"),
                ModelChannel::Ccs(id) => format!("ccs:{id}"),
            })
            .unwrap_or_default();
        self.session_control.runs.insert(
            id,
            ActiveRun::new(runtime, worktree, prefix, usage, model, effort),
        );
        let command_ticket = self
            .command_context_for(index, Some(id), provider)
            .filter(|_| provider.descriptor().commands.is_some())
            .map(|context| self.chat_commands.catalog.begin(context));
        if let Some(run) = self.session_control.runs.get_mut(&id) {
            run.usage_service = usage_service;
            run.intent = intent;
            run.command_executable = command_executable;
            run.command_ticket = command_ticket;
            run.status = conduit::i18n::text_static("runtime.saving_message").into();
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
            run.acp_variant = acp_variant;
            run.context_window_fallback = context_window_fallback;
            if fresh_session && provider == Provider::Codex {
                run.output_tokens_at_start = Some(0);
                run.usage_input_baseline = Some(0);
                run.usage_cached_baseline = Some(0);
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
        if session.messages[user_index + 1..=index]
            .iter()
            .any(|item| item.key.ends_with("-command"))
        {
            return;
        }
        if session.native_id.is_none() {
            self.message = Some(
                conduit::i18n::text_static("runtime.this_session_has_no_reliable_cli_session")
                    .into(),
            );
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
        self.poll_terminals(window, cx);
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
            && !self.search_ui.running
            && (self.search_ui.refresh_needed
                || self.search_ui.revision != self.session_control.global_revision)
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
        self.poll_automations(cx);
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
                self.message = Some(
                    conduit::i18n::text_static(
                        "runtime.history_reading_timed_out_existing_conversations_were",
                    )
                    .into(),
                );
                self.notify_user(
                    Notice::new(
                        NoticeSource::App,
                        NoticeLevel::Warning,
                        conduit::i18n::text_static("runtime.history_reading_timed_out"),
                        conduit::i18n::text_static(
                            "runtime.existing_conversations_were_preserved_import_history_again",
                        ),
                    ),
                    cx,
                );
            }
            self.startup = None;
            cx.notify();
        }
        self.poll_opencode_models(cx);
        self.poll_extensions(window, cx);
        // Refresh recent session ages and running elapsed labels once a second;
        // older session ages only need a repaint every minute.
        let label_interval = if self.session_control.runs.is_empty() {
            self.sidebar.age_refresh_interval()
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
        self.poll_cli_probes(cx);
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
            && !self.terminals.active()
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
            match self.save_exit_drafts(cx) {
                Ok(()) => {
                    self.cleanup_attachments();
                    window.remove_window();
                    cx.quit();
                }
                Err(error) => {
                    self.closing = false;
                    self.message = Some(conduit::i18n::format(
                        "runtime.draft_save_failed_exit_cancelled_keep_your",
                        &[("error", format!("{:#}", error))],
                    ));
                    cx.notify();
                }
            }
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
            run.status = conduit::i18n::text_static("runtime.submitting_approval").into();
        } else {
            run.approvals.remove(request);
            run.status = conduit::i18n::text_static(
                "runtime.approval_submission_failed_the_runtime_connection_closed",
            )
            .into();
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
