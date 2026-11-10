//! Durable send barriers. The UI keeps drafts until the exact snapshot succeeds.
use super::*;
use crate::persistence::SaveReceipt;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

#[derive(Clone)]
pub(in crate::app) struct SentComposer {
    pub(in crate::app) preserve: bool,
    target: ComposerTarget,
    text: String,
    references: Vec<FileReference>,
}

pub(in crate::app) struct PendingTurn {
    pub(super) receipt: SaveReceipt,
    pub(super) release: std::sync::mpsc::Sender<()>,
    pub(super) before: Option<SessionRecord>,
    pub(in crate::app) composer: Option<SentComposer>,
    pub(in crate::app) queued: Option<queue::QueuedMessage>,
    pub(super) naming_snippet: Option<String>,
    pub(super) selection: Option<ComposerSelection>,
}

pub(in crate::app) struct PendingQueueSave {
    pub(in crate::app) receipt: SaveReceipt,
    pub(in crate::app) composer: Option<SentComposer>,
}

impl ConduitApp {
    pub(in crate::app) fn sent_composer(&self, cx: &Context<Self>) -> Option<SentComposer> {
        Some(SentComposer {
            preserve: false,
            target: self.composer_target()?,
            text: self.input.read(cx).value().to_string(),
            references: self
                .current_files()
                .map(|files| files.references.clone())
                .unwrap_or_default(),
        })
    }

    pub(in crate::app) fn composer_save_pending(&self) -> bool {
        let target = self.composer_target();
        let matches = |composer: &SentComposer| target.as_ref() == Some(&composer.target);
        self.session_control.runs.values().any(|run| {
            run.pending_save
                .as_ref()
                .and_then(|pending| pending.composer.as_ref())
                .is_some_and(matches)
        }) || self
            .session_control
            .queue_saves
            .values()
            .any(|pending| pending.composer.as_ref().is_some_and(matches))
    }

    pub(in crate::app) fn pending_new_session(&self, id: u64) -> bool {
        self.session_control
            .runs
            .get(&id)
            .and_then(|run| run.pending_save.as_ref())
            .is_some_and(|pending| pending.before.is_none())
    }

    pub(in crate::app) fn session_draft_save_pending(&self, id: u64) -> bool {
        self.session_control.runs.values().any(|run| {
            run.pending_save
                .as_ref()
                .and_then(|pending| pending.composer.as_ref())
                .is_some_and(|sent| sent.target.session == Some(id))
        })
    }

    pub(in crate::app) fn clear_saved_composer(
        &mut self,
        sent: &SentComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.composer_target().as_ref() == Some(&sent.target);
        // Edits and navigation during a slow save belong to the user. Never
        // clear another draft, or new text/attachments entered while waiting.
        let text_matches = if current {
            self.input.read(cx).value().as_ref() == sent.text
        } else if let Some(id) = sent.target.session {
            self.drafts.get(&id) == Some(&sent.text)
        } else {
            self.workspace_drafts.get(&sent.target.workspace) == Some(&sent.text)
        };
        let files_match = self
            .composer_files
            .get(&sent.target)
            .map(|files| files.references.as_slice())
            .unwrap_or_default()
            == sent.references;
        if !text_matches || !files_match {
            return false;
        }
        if sent.preserve {
            return true;
        }
        if current {
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.prompt_history = None;
        }
        self.composer_files.remove(&sent.target);
        if let Some(id) = sent.target.session {
            self.drafts.remove(&id);
        } else {
            self.workspace_drafts.remove(&sent.target.workspace);
        }
        true
    }

    pub(in crate::app) fn restore_command_draft(
        &mut self,
        id: u64,
        sent: &SentComposer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_session == Some(id)
            && self.composer_target().as_ref() == Some(&sent.target)
            && self.input.read(cx).value().is_empty()
            && self
                .current_files()
                .is_none_or(|files| files.references.is_empty())
        {
            self.input.update(cx, |input, cx| {
                input.set_value(sent.text.clone(), window, cx)
            });
        } else if self.selected_session != Some(id)
            && self.drafts.get(&id).is_none_or(String::is_empty)
        {
            self.drafts.insert(id, sent.text.clone());
        }
    }

    pub(super) fn restore_unsent_turn(&mut self, id: u64, before: Option<SessionRecord>) {
        if let Some(before) = before {
            if let Some(session) = self.session_mut(id) {
                session.messages = before.messages;
                session.title = before.title;
                session.interrupted = before.interrupted;
            }
        } else {
            for workspace in &mut self.store.workspaces {
                workspace.sessions.retain(|session| session.id != id);
            }
            self.open_tabs.retain(|tab| *tab != DocumentTab::Chat(id));
            if self.selected_session == Some(id) {
                self.selected_session = None;
            }
        }
        self.session_control.changed(id);
        self.dirty = true;
    }

    pub(in crate::app) fn poll_pending_sends(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready: Vec<_> = self
            .session_control
            .runs
            .iter()
            .filter_map(|(&id, run)| {
                let pending = run.pending_save.as_ref()?;
                let result = if self.closing || run.runtime.is_cancelled() {
                    Some(Err(conduit::i18n::text_static(
                        "runtime.saves.send_cancelled_the_message_was_not_sent",
                    )
                    .into()))
                } else {
                    pending.receipt.poll()
                };
                result.map(|result| (id, result))
            })
            .collect();
        for (id, result) in ready {
            let mut pending = self
                .session_control
                .runs
                .get_mut(&id)
                .unwrap()
                .pending_save
                .take()
                .unwrap();
            let result = result.and_then(|()| {
                if let Some((provider, path)) = self
                    .session_control
                    .runs
                    .get(&id)
                    .and_then(|run| run.command_executable.clone())
                {
                    self.remember_command_executable(provider, &path)?;
                }
                if let Some(run) = self.session_control.runs.get(&id)
                    && let crate::commands::RequestIntent::Command(call) = &run.intent
                    && self.session_command_context(id).as_ref() != Some(call.context())
                {
                    return Err(conduit::i18n::text("commands.stale"));
                }
                if let Some(run) = self.session_control.runs.get(&id)
                    && let crate::commands::RequestIntent::Fork(request) = &run.intent
                {
                    let session = self
                        .session(id)
                        .ok_or_else(|| conduit::i18n::text("branching.changed"))?;
                    if crate::branching::request(session).as_ref() != Ok(request)
                        || run.command_ticket.as_ref().map(|ticket| &ticket.context)
                            != self.session_command_context(id).as_ref()
                    {
                        return Err(conduit::i18n::text("branching.changed"));
                    }
                }
                pending.release.send(()).map_err(|_| {
                    conduit::i18n::text_static(
                        "runtime.saves.cli_wait_thread_exited_the_message_was",
                    )
                    .into()
                })
            });
            match result {
                Ok(()) => {
                    self.session_control.plans.remove(&id);
                    self.session_control.unread_completed.remove(&id);
                    let composer_cleared = pending
                        .composer
                        .as_ref()
                        .is_some_and(|sent| self.clear_saved_composer(sent, window, cx));
                    if let Some(selection) = pending.selection.take() {
                        let current = composer_cleared
                            && pending.composer.as_ref().is_some_and(|sent| {
                                self.composer_target().as_ref() == Some(&sent.target)
                                    && (sent.preserve || self.input.read(cx).value().is_empty())
                                    && self.new_session_provider == selection.engine
                                    && self.active_file.is_none()
                                    && self.active_diff.is_none()
                                    && self.pending_model_selection.as_ref().is_none_or(|choice| {
                                        choice.engine == selection.engine
                                            && choice.channel == selection.channel
                                            && choice.model == selection.model
                                            && choice.effort == selection.effort
                                            && choice.opencode.variant == selection.opencode.variant
                                            && choice.opencode.options == selection.opencode.options
                                    })
                            });
                        if current
                            && let Some(index) =
                                self.store.workspaces.iter().position(|workspace| {
                                    workspace.sessions.iter().any(|session| session.id == id)
                                })
                        {
                            if let Some(sent) =
                                pending.composer.as_ref().filter(|sent| sent.preserve)
                            {
                                if let Some(files) = self.composer_files.remove(&sent.target) {
                                    self.composer_files.insert(
                                        ComposerTarget {
                                            workspace: sent.target.workspace.clone(),
                                            session: Some(id),
                                        },
                                        files,
                                    );
                                }
                                if let Some(old) = sent.target.session {
                                    self.drafts.remove(&old);
                                } else {
                                    self.workspace_drafts.remove(&sent.target.workspace);
                                }
                            }
                            self.select_created_session(index, id, selection.engine);
                            self.scroll_to_end();
                        }
                    }
                    if let Some(run) = self.session_control.runs.get_mut(&id) {
                        run.status =
                            conduit::i18n::text_static("runtime.saves.connecting_to_cli").into();
                        run.activity =
                            conduit::i18n::text_static("runtime.saves.starting_cli").into();
                    }
                    if let Some(snippet) = pending.naming_snippet {
                        self.start_session_naming(id, &snippet, cx);
                    }
                }
                Err(error) => {
                    if let Some(ticket) = self
                        .session_control
                        .runs
                        .get(&id)
                        .and_then(|run| run.command_ticket.as_ref())
                    {
                        self.chat_commands.catalog.finish(ticket);
                    }
                    let target = if pending.before.is_some() {
                        self.session_notice_target(id)
                    } else {
                        // A failed provisional session will be removed. Point to
                        // the source draft when it belongs to an existing chat.
                        pending
                            .composer
                            .as_ref()
                            .and_then(|sent| sent.target.session)
                            .and_then(|source| self.session_notice_target(source))
                    };
                    self.session_control.runs.remove(&id);
                    self.restore_unsent_turn(id, pending.before);
                    if let Some(mut queued) = pending.queued {
                        queued.paused = Some(error.clone());
                        self.queued_messages.insert(id, queued);
                    }
                    self.persist();
                    if self.closing {
                        // Stay open so the original unsent composer can be recovered.
                        self.closing = false;
                    }
                    let mut notice = Notice::new(
                        NoticeSource::Session,
                        NoticeLevel::Error,
                        conduit::i18n::text_static("runtime.saves.message_not_sent"),
                        conduit::i18n::format(
                            "runtime.saves.your_draft_has_been_preserved",
                            &[("error", error.to_string())],
                        ),
                    );
                    if let Some(target) = target {
                        notice = notice.target(target);
                    }
                    self.notify_user(notice, cx);
                }
            }
            self.scroll.remeasure();
            cx.notify();
        }
        let ready: Vec<_> = self
            .session_control
            .queue_saves
            .iter()
            .filter_map(|(&id, pending)| pending.receipt.poll().map(|result| (id, result)))
            .collect();
        for (id, result) in ready {
            let pending = self.session_control.queue_saves.remove(&id).unwrap();
            match result {
                Ok(()) => {
                    if let Some(sent) = &pending.composer {
                        self.clear_saved_composer(sent, window, cx);
                    }
                    if !self.session_busy(id) {
                        self.dispatch_queued_message(id, cx);
                    }
                }
                Err(error) => {
                    self.queued_messages.remove(&id);
                    self.dirty = true;
                    self.persist();
                    self.closing = false;
                    let mut notice = Notice::new(
                        NoticeSource::Session,
                        NoticeLevel::Error,
                        conduit::i18n::text_static("queue.message_not_queued"),
                        conduit::i18n::format(
                            "queue.save_failed_your_draft_has_been_preserved",
                            &[("error", error.to_string())],
                        ),
                    );
                    if let Some(target) = self.session_notice_target(id) {
                        notice = notice.target(target);
                    }
                    self.notify_user(notice, cx);
                }
            }
            cx.notify();
        }
    }
}
