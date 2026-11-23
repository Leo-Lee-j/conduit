//! Bounded stream consumption and transcript invalidation.
use super::*;
use crate::usage_ledger::ApiRequestOutcome;
impl ConduitApp {
    pub(super) fn poll_streams(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.maintain_file_proposals(cx);
        self.poll_child_controls(window, cx);
        self.poll_child_histories(cx);
        let navigating = self.pending_message_jump.is_some();
        // Follow is reader intent, not a measurement of a changing document.
        // Unknown heights and parser completion must not toggle it. Only a
        // downward wheel gesture that actually landed at the end resumes it.
        if !navigating && !self.at_bottom && self.timeline.read(cx).resume_follow(self) {
            self.at_bottom = true;
            cx.notify();
        }
        let follow = self.at_bottom && !self.timeline.read(cx).wheel_pending(self);
        // A global fair budget limits both concurrent streams and large events.
        let mut incoming = Vec::new();
        let mut ids = self
            .session_control
            .runs
            .iter()
            .filter_map(|(&id, run)| run.pending_save.is_none().then_some(id))
            .collect::<Vec<_>>();
        ids.sort_unstable();
        if !ids.is_empty() {
            let offset = self.session_control.event_cursor % ids.len();
            ids.rotate_left(offset);
            self.session_control.event_cursor = (offset + 1) % ids.len();
            let started = Instant::now();
            let mut bytes = 0;
            'drain: loop {
                let mut progress = false;
                for id in &ids {
                    if incoming.len() >= 256
                        || bytes >= 512 * 1024
                        || started.elapsed() >= Duration::from_millis(3)
                    {
                        break 'drain;
                    }
                    let run = &self.session_control.runs[id];
                    if let Ok(event) = run.runtime.events.try_recv() {
                        bytes += crate::event_queue::event_bytes(&event);
                        incoming.push((*id, run.prefix.clone(), event));
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
        }
        if self.store.ui.performance_monitor {
            self.performance
                .update(cx, |monitor, _| monitor.record_events(incoming.len()));
        }
        let mut changed = false;
        let mut terminal = false;
        let mut files_changed = false;
        let mut visible = false;
        let mut dirty_messages = Vec::new();
        let mut structural_change = false;
        let mut completed_turns = Vec::new();
        let mut file_tool_events = Vec::new();
        for (id, prefix, event) in incoming {
            if self
                .session_control
                .runs
                .get(&id)
                .is_none_or(|run| run.prefix != prefix)
            {
                continue;
            }
            if self.session_control.runs.get(&id).is_some_and(|run| {
                !matches!(run.intent, crate::commands::RequestIntent::Message)
                    && run.runtime.is_cancelled()
            }) && matches!(
                event,
                Event::CommandCatalog(_) | Event::Delta { .. } | Event::Text { .. }
            ) {
                continue;
            }
            // Transient previews have their own entity invalidation and never enter history.
            if let Event::FileProposal { key, update } = event {
                if let Some(root) = self
                    .session_control
                    .runs
                    .get(&id)
                    .filter(|run| !run.runtime.is_cancelled())
                    .map(|run| run.worktree.clone())
                {
                    self.file_proposal_event(id, &format!("{prefix}-{key}"), &root, update, cx);
                }
                continue;
            }
            match &event {
                Event::Tool { key, status, .. }
                    if !matches!(
                        status,
                        crate::message_status::ToolStatus::Running
                            | crate::message_status::ToolStatus::Output
                    ) =>
                {
                    self.end_file_proposals(id, Some(&format!("{prefix}-{key}")), cx)
                }
                Event::Finished { .. } | Event::Failure(_) | Event::CleanupBlocked(_) => {
                    self.end_file_proposals(id, None, cx)
                }
                _ => (),
            }
            let first_response = match &event {
                Event::Delta { text, .. } | Event::Text { text, .. } => !text.trim().is_empty(),
                Event::ReasoningSummary { text, .. } => !text.trim().is_empty(),
                Event::Tool { .. } | Event::Approval(_) => true,
                _ => false,
            };
            if first_response
                && let Some(run) = self.session_control.runs.get_mut(&id)
                && run.api_first_response_ms.is_none()
                && let Some(started) = run.api_request_started
            {
                run.api_first_response_ms =
                    Some(started.elapsed().as_millis().min(u64::MAX as u128) as u64);
            }
            changed = true;
            visible |= Some(id) == self.selected_session;
            if let Some((workspace, session)) =
                self.session_control.locate(&self.store.workspaces, id)
                && let Some(run) = self.session_control.runs.get_mut(&id)
                && let Some(position) = run.message_index.observe_reasoning(
                    &mut self.store.workspaces[workspace].sessions[session],
                    &prefix,
                    &event,
                    Instant::now(),
                )
                && Some(id) == self.selected_session
            {
                dirty_messages.push(position);
            }
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
                Event::Activity(text) => {
                    if let Some(run) = self.session_control.runs.get_mut(&id)
                        && run.approvals.is_empty()
                        && !run.runtime.is_cancelled()
                    {
                        run.activity = text;
                        run.activity_message = None;
                    }
                }
                Event::Child {
                    thread_id,
                    turn_id,
                    event,
                } => {
                    self.apply_child_event(id, &prefix, &thread_id, &turn_id, *event);
                    cx.notify();
                }
                Event::Subagent(agent) => {
                    if let Some(session) = self.session_mut(id) {
                        crate::subagents::upsert(&mut session.subagents, agent, &prefix);
                    }
                    self.invalidate_child_body(id);
                    self.session_control.changed(id);
                }
                Event::NativeTurnCompleted { thread_id, turn_id } => {
                    if let Some(session) = self.session_mut(id) {
                        crate::branching::record(session, thread_id, turn_id);
                    }
                }
                Event::CommandCatalog(result) => {
                    if let Some(ticket) = self
                        .session_control
                        .runs
                        .get(&id)
                        .and_then(|run| run.command_ticket.clone())
                        && self.session_command_context(id).as_ref() == Some(&ticket.context)
                    {
                        self.chat_commands.catalog.publish(&ticket, result);
                        self.refresh_chat_commands(cx);
                    }
                }
                Event::AcpConfig(options) => {
                    let variant = if let Some(session) = self.session_mut(id) {
                        session.opencode.options = options;
                        session.opencode.effective_variant().map(str::to_owned)
                    } else {
                        None
                    };
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.acp_variant = variant;
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
                        self.session_control.plans.remove(&id);
                    } else {
                        self.session_control.plans.insert(id, steps);
                    }
                }
                Event::Usage(mut usage) => {
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        if usage.input_tokens_known && usage.input_tokens < run.usage.input_tokens {
                            run.usage_input_baseline = None;
                        }
                        if usage.cached_input_tokens_known
                            && usage.cached_input_tokens < run.usage.cached_input_tokens
                        {
                            run.usage_cached_baseline = None;
                        }
                        if (usage.output_tokens < run.output_tokens_at_start.unwrap_or(0)
                            || usage.output_tokens < run.usage.output_tokens)
                            && usage.turn_output_tokens.is_none()
                        {
                            // A counter reset is not a new speed measurement.
                            run.output_tokens_at_start = None;
                        }
                        let elapsed =
                            run.turn_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        usage = usage
                            .sample_turn(run.output_tokens_at_start, elapsed)
                            .with_session_speed(run.completed_usage);
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
                        .session_control
                        .runs
                        .get(&id)
                        .is_some_and(|run| !self.closing && !run.runtime.is_cancelled());
                    if !accepted {
                        continue;
                    }
                    let notification_approval = approval.clone();
                    let questions = approval.questions();
                    let drafts = questions
                        .iter()
                        .map(|q| {
                            let input = cx.new(|cx| {
                                InputState::new(window, cx)
                                    .placeholder(conduit::i18n::text_static(
                                        "stream_events.enter_your_answer",
                                    ))
                                    .masked(q.secret)
                            });
                            let subscription =
                                cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
                            super::user_input::QuestionDraft {
                                selected: Default::default(),
                                custom: input,
                                _subscription: subscription,
                                question: q.clone(),
                            }
                        })
                        .collect();
                    if let Some(session) = self.session_mut(id) {
                        session.record_tool_approval(
                            &prefix,
                            &approval.id,
                            &approval.title,
                            &approval.detail,
                            "等待中",
                        );
                        if approval.is_user_input() {
                            session.record_user_input(
                                &prefix,
                                &approval.id,
                                questions
                                    .iter()
                                    .map(super::user_input::question_record)
                                    .collect(),
                            );
                        }
                    }
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        if approval.is_user_input() {
                            run.question_drafts.insert(approval.id.clone(), drafts);
                        }
                        run.approvals.push(approval);
                        run.status = run.waiting_status().into();
                        run.activity = run.status.clone();
                    }
                    self.notify_approval(id, &prefix, &notification_approval, cx);
                }
                Event::ApprovalCancelled(request) => {
                    self.finish_notification_approval(
                        id,
                        &request,
                        crate::notifications::ApprovalOutcome::Cancelled,
                        cx,
                    );
                    let mut approval = None;
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.expanded_approvals.remove(&request);
                        run.question_drafts.remove(&request);
                        approval = run.approvals.remove(&request);
                        if run.approvals.is_empty() && !run.runtime.is_cancelled() {
                            run.status = conduit::i18n::text_static("stream_events.running").into();
                            run.activity =
                                conduit::i18n::text_static("stream_events.continuing").into();
                        } else if !run.runtime.is_cancelled() {
                            run.status = run.waiting_status().into();
                            run.activity = run.status.clone();
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
                    self.finish_notification_approval(
                        id,
                        &request,
                        if allow {
                            crate::notifications::ApprovalOutcome::Allowed
                        } else {
                            crate::notifications::ApprovalOutcome::Denied
                        },
                        cx,
                    );
                    let mut answered_questions = None;
                    let approval = self.session_control.runs.get_mut(&id).and_then(|run| {
                        run.expanded_approvals.remove(&request);
                        if let Some(drafts) = run.question_drafts.remove(&request)
                            && allow
                        {
                            answered_questions =
                                Some(drafts.iter().map(|draft| draft.record(cx)).collect());
                        }
                        let approval = run.approvals.remove(&request);
                        run.status = if run.approvals.is_empty() {
                            conduit::i18n::text_static("stream_events.running")
                        } else {
                            run.waiting_status()
                        }
                        .into();
                        run.activity = if run.approvals.is_empty() {
                            conduit::i18n::text_static("stream_events.continuing")
                        } else {
                            run.waiting_status()
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
                            if approval.is_user_input() {
                                if allow { "已回答" } else { "已跳过" }
                            } else if allow {
                                "已允许"
                            } else {
                                "已拒绝"
                            },
                        );
                        if let Some(questions) = answered_questions {
                            session.record_user_input(&prefix, &request, questions);
                        }
                    }
                }
                Event::Finished { error, cancelled } => {
                    self.finish_child_observation(id);
                    if let Some(session) = self.session_mut(id) {
                        crate::subagents::disconnect(
                            &mut session.subagents,
                            Some(&prefix),
                            cancelled,
                        );
                    }
                    let finished_run = self.session_control.runs.remove(&id);
                    if let Some(run) = &finished_run
                        && matches!(run.intent, crate::commands::RequestIntent::Message)
                        && !cancelled
                        && !run.runtime.is_cancelled()
                    {
                        cx.emit(super::encourager::PetFeedback {
                            failed: error.is_some(),
                        });
                    }
                    if let Some(run) = &finished_run
                        && matches!(run.intent, crate::commands::RequestIntent::Message)
                    {
                        self.finish_ai_review(
                            id,
                            &prefix,
                            error.is_some() || cancelled || run.runtime.is_cancelled(),
                            cx,
                        );
                    }
                    if let Some(run) = &finished_run
                        && matches!(run.intent, crate::commands::RequestIntent::Message)
                    {
                        self.completion_automations(
                            id,
                            &run.usage_key,
                            error.is_some(),
                            cancelled || run.runtime.is_cancelled(),
                            cx,
                        );
                    }
                    if let Some(run) = &finished_run
                        && !matches!(run.intent, crate::commands::RequestIntent::Fork(_))
                    {
                        self.record_turn_usage(
                            id,
                            run,
                            error.is_some() || cancelled,
                            if cancelled || run.runtime.is_cancelled() {
                                ApiRequestOutcome::Cancelled
                            } else if error.is_some() {
                                ApiRequestOutcome::Failed
                            } else {
                                ApiRequestOutcome::Succeeded
                            },
                            cx,
                        );
                    }
                    if let Some(run) = &finished_run {
                        if let Some(ticket) = &run.command_ticket {
                            self.chat_commands.catalog.finish(ticket);
                        }
                        if (error.is_some() || cancelled)
                            && let Some(sent) = &run.command_draft
                        {
                            self.restore_command_draft(id, sent, window, cx);
                        }
                        if !matches!(run.intent, crate::commands::RequestIntent::Message)
                            && error.is_none()
                            && !cancelled
                            && let Some(session) = self.session_mut(id)
                        {
                            let key =
                                if matches!(run.intent, crate::commands::RequestIntent::Discover) {
                                    "commands.connected"
                                } else if matches!(
                                    run.intent,
                                    crate::commands::RequestIntent::Fork(_)
                                ) {
                                    "branching.created"
                                } else {
                                    "commands.completed"
                                };
                            apply_event_indexed(
                                session,
                                &prefix,
                                Event::Notice(conduit::i18n::text(key)),
                                &mut MessageIndex::default(),
                            );
                        }
                    }
                    self.refresh_notifications(cx);
                    self.notifications.center.resolve_session(id, unix_now());
                    if finished_run.is_some()
                        && !cancelled
                        && (error.is_some() || !self.session_is_notification_visible(id))
                        && let Some(target) = self.session_notice_target(id)
                    {
                        let session_title = self
                            .session(id)
                            .map(|session| session.title.clone())
                            .unwrap_or_default();
                        let (level, title, message) = if let Some(error) = &error {
                            (
                                NoticeLevel::Error,
                                conduit::i18n::text_static("stream_events.session_failed"),
                                failure_summary(error),
                            )
                        } else {
                            (
                                NoticeLevel::Success,
                                conduit::i18n::text_static("stream_events.session_completed"),
                                session_title,
                            )
                        };
                        self.notify_user(
                            Notice::new(NoticeSource::Session, level, title, message)
                                .target(target)
                                .key(format!("session/{id}/{prefix}/finished"), "finished")
                                .presentation(if self.session_is_notification_visible(id) {
                                    NoticePresentation::History
                                } else {
                                    NoticePresentation::Toast
                                }),
                            cx,
                        );
                    }
                    self.refresh_notifications(cx);
                    if error.is_some() || cancelled {
                        self.pause_queued_message(
                            id,
                            if cancelled {
                                conduit::i18n::text_static("composer.the_current_response_has_stopped_queued_messages")
                            } else {
                                conduit::i18n::text_static("stream_events.the_current_response_failed_queued_messages_will")
                            },
                        );
                    }
                    let elapsed_ms = finished_run.as_ref().map(|run| {
                        run.turn_started.elapsed().as_millis().min(u64::MAX as u128) as u64
                    });
                    let finished_usage = finished_run
                        .as_ref()
                        .filter(|run| run.usage.turn_elapsed_ms.is_some())
                        .map(|run| {
                            let mut usage = run.usage;
                            usage.turn_elapsed_ms = elapsed_ms;
                            usage.with_session_speed(run.completed_usage)
                        });
                    self.session_control.unread_completed.remove(&id);
                    terminal = true;
                    // The selected chat can be hidden by a file or child page.
                    // Errors in other chats retain their error notification, not
                    // a successful-completion marker on an unrelated tab.
                    if self.selected_session == Some(id)
                        && !self.session_is_notification_visible(id)
                        && error.is_some()
                        && !cancelled
                    {
                        self.session_control.unread_completed.insert(id);
                    }
                    if error.is_none() && !cancelled {
                        if finished_run.as_ref().is_some_and(|run| {
                            !matches!(run.intent, crate::commands::RequestIntent::Fork(_))
                        }) {
                            completed_turns.push(id);
                        }
                        if !self.session_is_notification_visible(id) {
                            self.session_control.unread_completed.insert(id);
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
                        if !cancelled
                            && let Some(error) = &error
                            && (!already_shown
                                || session.messages.iter().any(|item| {
                                    item.kind == ItemKind::Error
                                        && item.key.starts_with(&format!("{prefix}-"))
                                }))
                        {
                            crate::failure::record_failure(session, &prefix, error, true);
                        }
                        if cancelled {
                            session
                                .messages
                                .push(ChatItem::from(crate::domain::ChatItemData {
                                    reasoning_summary: None,
                                    elapsed_ms: None,
                                    key: format!("{prefix}-finish"),
                                    created_at: unix_now(),
                                    kind: ItemKind::Notice,
                                    text: conduit::i18n::text_static(
                                        "stream_events.cancelled_will_not_retry_automatically",
                                    )
                                    .into(),
                                    input: String::new(),
                                    detail: error.clone().unwrap_or_default(),
                                    status: Default::default(),
                                    approval: None,
                                    model: None,
                                }));
                        }
                    }
                }
                Event::CleanupBlocked(error) => {
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.cleanup_error = Some(error.clone());
                        run.cancel();
                    }
                    self.pause_queued_message(
                        id,
                        conduit::i18n::text_static(
                            "stream_events.process_cleanup_failed_queued_messages_paused",
                        ),
                    );
                    self.closing = false;
                    self.message = Some(error.clone());
                    if let Some(session) = self.session_mut(id) {
                        crate::failure::record_failure(session, &prefix, &error, true);
                    }
                    if let Some(target) = self.session_notice_target(id) {
                        self.notify_user(
                            Notice::new(
                                NoticeSource::Session,
                                NoticeLevel::Error,
                                conduit::i18n::text_static("stream_events.process_cleanup_failed"),
                                error,
                            )
                            .target(target),
                            cx,
                        );
                    }
                    self.refresh_notifications(cx);
                }
                Event::NativeId(native_id) => {
                    if let Some(ticket) = self
                        .session_control
                        .runs
                        .get_mut(&id)
                        .and_then(|run| run.command_ticket.as_mut())
                    {
                        self.chat_commands
                            .catalog
                            .bind_native(ticket, native_id.clone());
                    }
                    if let Some(session) = self.session_mut(id) {
                        crate::branching::bound(session, &native_id);
                        session.native_id = Some(native_id);
                    }
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.status = conduit::i18n::text_static("stream_events.running").into();
                        // A thread ID only confirms the CLI handshake, not model output.
                        run.activity = conduit::i18n::text_static(
                            if matches!(run.intent, crate::commands::RequestIntent::Fork(_)) {
                                "branching.created"
                            } else {
                                "stream_events.waiting_for_model_response"
                            },
                        )
                        .into();
                        run.activity_message = None;
                    }
                    self.dirty = true;
                    self.persist();
                }
                other => {
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        let activity = match &other {
                            Event::Delta { .. } | Event::Text { .. } => {
                                conduit::i18n::text_static("stream_events.generating_response")
                                    .to_string()
                            }
                            Event::ReasoningSummary {
                                text,
                                snapshot: false,
                                summary: false,
                                ..
                            } if !text.trim().is_empty() => {
                                conduit::i18n::text_static("stream_events.thinking").to_string()
                            }
                            Event::Failure(error) => error
                                .retry_message()
                                .map(|message| message.render())
                                .unwrap_or_else(|| {
                                    conduit::i18n::text_static("stream_events.request_failed")
                                        .to_string()
                                }),
                            Event::ToolInput { .. } => {
                                conduit::i18n::text_static("stream_events.preparing_tool_call")
                                    .to_string()
                            }
                            Event::Tool { title, status, .. } => match status {
                                crate::message_status::ToolStatus::Running if !title.is_empty() => {
                                    conduit::i18n::format(
                                        "stream_events.calling_tool",
                                        &[("title", title.to_string())],
                                    )
                                }
                                crate::message_status::ToolStatus::Running => {
                                    conduit::i18n::text_static("stream_events.calling_tool_72ae8e")
                                        .to_string()
                                }
                                crate::message_status::ToolStatus::Output => {
                                    conduit::i18n::text_static(
                                        "stream_events.receiving_tool_output",
                                    )
                                    .to_string()
                                }
                                _ => conduit::i18n::text_static("stream_events.continuing")
                                    .to_string(),
                            },
                            _ => String::new(),
                        };
                        if !activity.is_empty() {
                            run.activity_message = match &other {
                                Event::Failure(error) => error
                                    .retry_message()
                                    .map(|message| (activity.clone(), message)),
                                Event::Tool { title, status, .. }
                                    if !title.is_empty()
                                        && *status
                                            == crate::message_status::ToolStatus::Running =>
                                {
                                    Some((
                                        activity.clone(),
                                        conduit::i18n::Message::new(
                                            "stream_events.calling_tool",
                                            &[("title", title.clone())],
                                        ),
                                    ))
                                }
                                _ => None,
                            };
                            run.activity = activity;
                        }
                    }
                    let mut index = self
                        .session_control
                        .runs
                        .get_mut(&id)
                        .map(|run| std::mem::take(&mut run.message_index))
                        .unwrap_or_default();
                    let turn_model = self
                        .session_control
                        .runs
                        .get(&id)
                        .and_then(|run| run.model.clone());
                    let selected = Some(id) == self.selected_session;
                    let file_root = self
                        .session_control
                        .runs
                        .get(&id)
                        .map(|run| run.worktree.clone());
                    if let Some(session) = self.session_mut(id) {
                        let old_len = session.messages.len();
                        let shape = |item: &ChatItem| {
                            (is_empty_chat_placeholder(item), is_groupable_activity(item))
                        };
                        if let Event::Tool { key, status, .. } = &other
                            && matches!(
                                status,
                                crate::message_status::ToolStatus::Completed
                                    | crate::message_status::ToolStatus::Failed
                                    | crate::message_status::ToolStatus::Cancelled
                                    | crate::message_status::ToolStatus::Interrupted
                            )
                            && index
                                .find(&session.messages, &format!("{prefix}-{key}"))
                                .is_none_or(|position| session.messages[position].status != *status)
                        {
                            files_changed = true;
                        }
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
                            if session.messages[changed_index].kind == ItemKind::Tool
                                && let Some(root) = &file_root
                            {
                                file_tool_events.push((
                                    id,
                                    root.clone(),
                                    session.messages[changed_index].clone(),
                                ));
                            }
                            if let Some(model) = &turn_model {
                                let item = &mut session.messages[changed_index];
                                if item.kind == ItemKind::Assistant && item.model.is_none() {
                                    item.model = Some(model.clone());
                                }
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
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.message_index = index;
                    }
                }
            }
        }
        if self.store.ui.auto_rename && self.store.ui.naming_mode == NamingMode::Latest {
            for &id in &completed_turns {
                if let Some(excerpt) = self.session(id).and_then(latest_naming_excerpt) {
                    self.start_session_naming(id, &excerpt, cx);
                }
            }
        }
        if changed {
            for id in &ids {
                if self.store.child_bindings.contains_key(id) {
                    self.invalidate_child_body(*id);
                    self.session_control.changed(*id);
                }
            }
            self.sync_chat_rows();
            if structural_change {
                self.scroll.remeasure_items(0..self.scroll.item_count());
            } else {
                dirty_messages.sort_unstable();
                dirty_messages.dedup();
                for index in dirty_messages {
                    self.remeasure_message(index);
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
                if (self.git_navigation_open() || self.active_diff.is_some())
                    && self.git.probe.is_none()
                {
                    self.refresh_git(cx);
                }
            }
        }
        for (session, root, item) in file_tool_events {
            self.observe_file_tool(session, &root, &item, cx);
        }
        if files_changed || terminal {
            self.refresh_visible_files(cx);
        }
        for id in completed_turns {
            self.dispatch_queued_message(id, cx);
        }
    }
}
