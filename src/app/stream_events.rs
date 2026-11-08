//! Bounded stream consumption and transcript invalidation.
use super::*;
impl ConduitApp {
    pub(super) fn poll_streams(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let navigating = self.pending_message_jump.is_some();
        let follow = self.scroll.is_scrolled_to_end().unwrap_or_else(|| {
            self.scroll.logical_scroll_top().item_ix >= self.scroll.item_count()
        });
        // Re-render when the scroll position crosses the "at the end" threshold
        // so the floating arrow appears/disappears without polling every frame.
        if !navigating && follow != self.at_bottom {
            self.at_bottom = follow;
            cx.notify();
        }
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
                    let approval = self.session_control.runs.get_mut(&id).and_then(|run| {
                        run.expanded_approvals.remove(&request);
                        run.question_drafts.remove(&request);
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
                    }
                }
                Event::Finished { error, cancelled } => {
                    let finished_run = self.session_control.runs.remove(&id);
                    self.refresh_notifications(cx);
                    self.notifications.center.resolve_session(id, unix_now());
                    if finished_run.is_some()
                        && !cancelled
                        && (error.is_some() || !self.session_is_visible(id))
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
                                error.clone(),
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
                                .presentation(if self.session_is_visible(id) {
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
                    if (self.active_file.is_some() || self.active_diff.is_some())
                        && error.is_some()
                        && !cancelled
                    {
                        self.session_control.unread_completed.insert(id);
                    }
                    if error.is_none() && !cancelled {
                        completed_turns.push(id);
                        if self.selected_session != Some(id)
                            || (self.active_file.is_some() || self.active_diff.is_some())
                        {
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
                    if let Some(session) = self.session_mut(id) {
                        session.native_id = Some(native_id);
                    }
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.status = conduit::i18n::text_static("stream_events.running").into();
                        run.activity = conduit::i18n::text_static("stream_events.thinking").into();
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
                            Event::ToolInput { .. } => {
                                conduit::i18n::text_static("stream_events.preparing_tool_call")
                                    .to_string()
                            }
                            Event::Tool { title, status, .. } => match status.as_str() {
                                "运行中" if !title.is_empty() => conduit::i18n::format(
                                    "stream_events.calling_tool",
                                    &[("title", title.to_string())],
                                ),
                                "运行中" => {
                                    conduit::i18n::text_static("stream_events.calling_tool_72ae8e")
                                        .to_string()
                                }
                                "输出中" => conduit::i18n::text_static(
                                    "stream_events.receiving_tool_output",
                                )
                                .to_string(),
                                _ => conduit::i18n::text_static("stream_events.continuing")
                                    .to_string(),
                            },
                            _ => String::new(),
                        };
                        if !activity.is_empty() {
                            run.activity_message = match &other {
                                Event::Tool { title, status, .. }
                                    if !title.is_empty()
                                        && status
                                            == crate::message_status::ToolStatus::Running
                                                .as_str() =>
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
            self.sync_chat_rows();
            if structural_change {
                self.scroll.remeasure();
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
        for id in completed_turns {
            self.dispatch_queued_message(id, cx);
        }
    }
}
