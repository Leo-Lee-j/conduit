//! One unsent follow-up per conversation; never injected into an active turn.

use super::*;

pub(super) use crate::store::QueuedMessage;

impl ConduitApp {
    /// Keyboard and mouse share the same capacity and concurrency rules.
    pub(super) fn composer_send_blocked(&self) -> Option<&'static str> {
        if self.composer_save_pending() {
            return Some(conduit::i18n::text_static(
                "queue.saving_message_please_wait_your_draft_will",
            ));
        }
        if let Some(id) = self.send_target(true) {
            if self.queued_messages.contains_key(&id) {
                return Some(conduit::i18n::text_static(
                    "queue.one_message_is_already_queued_send_or",
                ));
            }
            if let Some(run) = self.session_control.runs.get(&id) {
                if !run.runtime.is_cancelled() {
                    return None;
                }
                return Some(conduit::i18n::text_static(
                    "queue.stopping_the_current_response_try_sending_again",
                ));
            }
        }
        self.send_blocked(true)
    }

    pub(super) fn enqueue_message(
        &mut self,
        id: u64,
        text: String,
        references: Vec<FileReference>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.closing
            || !self.writable
            || self.queued_messages.contains_key(&id)
            || self
                .session_control
                .runs
                .get(&id)
                .is_none_or(|run| run.runtime.is_cancelled() || run.pending_save.is_some())
        {
            return false;
        }
        let prompt = file_ref::compose_prompt(&text, &references);
        if prompt.trim().is_empty() || prompt.len() > 256 * 1024 {
            self.message = Some(
                conduit::i18n::text_static("queue.messages_must_be_nonempty_and_no_larger").into(),
            );
            cx.notify();
            return false;
        }
        self.queued_messages.insert(
            id,
            QueuedMessage {
                text,
                references,
                paused: None,
            },
        );
        let receipt = match self.persist_confirmed() {
            Ok(receipt) => receipt,
            Err(error) => {
                self.queued_messages.remove(&id);
                self.dirty = true;
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
                cx.notify();
                return false;
            }
        };
        self.session_control.queue_saves.insert(
            id,
            runtime::PendingQueueSave {
                receipt,
                composer: self.sent_composer(cx),
            },
        );
        self.message = None;
        cx.notify();
        true
    }

    pub(super) fn pause_queued_message(&mut self, id: u64, reason: &str) {
        if let Some(message) = self.queued_messages.get_mut(&id) {
            message.paused = Some(reason.into());
        }
    }

    /// Called only after the previous turn's final events have been merged.
    pub(super) fn dispatch_queued_message(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.closing
            || !self.writable
            || self.session_control.queue_saves.contains_key(&id)
            || self.session_busy(id)
        {
            return;
        }
        let Some(message) = self
            .queued_messages
            .get(&id)
            .filter(|message| message.paused.is_none())
        else {
            return;
        };
        let Some(index) = self
            .store
            .workspaces
            .iter()
            .position(|workspace| workspace.sessions.iter().any(|session| session.id == id))
        else {
            self.pause_queued_message(
                id,
                conduit::i18n::text_static("queue.the_session_no_longer_exists_retrieve_the"),
            );
            cx.notify();
            return;
        };
        let paths = message
            .references
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let (_, rejected) = file_ref::resolve(&paths, &self.store.workspaces[index].path);
        if !rejected.is_empty() {
            self.pause_queued_message(
                id,
                &conduit::i18n::format(
                    "queue.attachments_are_no_longer_available_retrieve_the",
                    &[("arg0", rejection_summary(&rejected).to_string())],
                ),
            );
            cx.notify();
            return;
        }
        let prompt = file_ref::compose_prompt(&message.text, &message.references);
        let provider = self.session(id).unwrap().provider;
        // Failure in a background conversation must not replace another chat's banner.
        let banner = self.message.clone();
        let message = self.queued_messages.remove(&id).unwrap();
        match self.start_turn(index, Some(id), provider, prompt, true, cx) {
            Ok(_) => {
                if let Some(pending) = self
                    .session_control
                    .runs
                    .get_mut(&id)
                    .and_then(|run| run.pending_save.as_mut())
                {
                    pending.queued = Some(message);
                }
                if self.selected_session == Some(id) {
                    self.scroll.remeasure();
                }
            }
            Err(error) => {
                if let Some(target) = self.session_notice_target(id) {
                    self.notify_user(
                        Notice::new(
                            NoticeSource::Session,
                            NoticeLevel::Error,
                            conduit::i18n::text_static("queue.queued_message_paused"),
                            error.clone(),
                        )
                        .target(target)
                        .presentation(if self.session_is_visible(id) {
                            NoticePresentation::History
                        } else {
                            NoticePresentation::Toast
                        }),
                        cx,
                    );
                }
                self.queued_messages.insert(
                    id,
                    QueuedMessage {
                        paused: Some(error),
                        ..message
                    },
                );
                self.persist();
            }
        }
        self.message = banner;
        cx.notify();
    }

    pub(super) fn send_queued_message(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.closing || !self.writable || self.session_busy(id) {
            return;
        }
        if let Some(message) = self.queued_messages.get_mut(&id) {
            message.paused = None;
        }
        self.dispatch_queued_message(id, cx);
    }

    pub(super) fn take_back_queued_message(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing
            || !self.writable
            || self.selected_session != Some(id)
            || self.session_control.queue_saves.contains_key(&id)
        {
            return;
        }
        // Never overwrite typed text or attachments, including a pending engine switch.
        if !self.input.read(cx).value().is_empty()
            || self
                .current_files()
                .is_some_and(|files| !files.references.is_empty())
        {
            self.message = Some(
                conduit::i18n::text_static("queue.handle_the_current_draft_before_retrieving_the")
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(message) = self.queued_messages.remove(&id) else {
            return;
        };
        let Some(target) = self.composer_target() else {
            self.queued_messages.insert(id, message);
            return;
        };
        self.new_session_provider = self.session(id).unwrap().provider;
        self.drafts.insert(id, message.text.clone());
        self.composer_files.insert(
            target,
            ComposerFiles {
                references: message.references,
                feedback: None,
            },
        );
        self.input
            .update(cx, |input, cx| input.set_value(message.text, window, cx));
        self.prompt_history = None;
        self.persist();
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(super) fn render_queued_message(
        &self,
        enabled: bool,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let id = self.selected_session?;
        let message = self.queued_messages.get(&id)?;
        let paused = message.paused.is_some();
        let summary = if message.text.trim().is_empty() {
            conduit::i18n::text_static("queue.attachments_only").into()
        } else {
            message
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(180)
                .collect::<String>()
        };
        let saving = self.session_control.queue_saves.contains_key(&id);
        let can_take_back = enabled
            && !saving
            && self.input.read(cx).value().is_empty()
            && self
                .current_files()
                .is_none_or(|files| files.references.is_empty());
        let can_send = enabled && !saving && !self.session_busy(id);
        Some(
            v_flex()
                .id("queued-message")
                .debug_selector(|| "queued-message".into())
                .mx(px(4.))
                .px_2()
                .py_1()
                .gap_1()
                .rounded(px(10.))
                .bg(rgb(palette().surface_strong))
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette().text))
                                .child(if saving {
                                    conduit::i18n::text_static("queue.saving")
                                } else if paused {
                                    conduit::i18n::text_static("queue.queued_paused")
                                } else {
                                    conduit::i18n::text_static("queue.queued")
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(palette().text))
                                .when_some(self.chat_typography().body_font, |text, family| {
                                    text.font_family(family)
                                })
                                .child(summary),
                        )
                        .when(!message.references.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::format(
                                        "queue.attachments",
                                        &[("arg0", format!("{}", message.references.len()))],
                                    )),
                            )
                        })
                        .when(paused, |row| {
                            row.child(
                                Button::new("send-queued-message")
                                    .debug_selector(|| "send-queued-message".into())
                                    .ghost()
                                    .small()
                                    .compact()
                                    .label(conduit::i18n::text_static("queue.send"))
                                    .disabled(!can_send)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.send_queued_message(id, cx);
                                    })),
                            )
                        })
                        .child(
                            Button::new("take-back-queued-message")
                                .debug_selector(|| "take-back-queued-message".into())
                                .ghost()
                                .small()
                                .compact()
                                .icon(FullIconName::Pencil)
                                .accessibility_label(conduit::i18n::text_static(
                                    "queue.retrieve_to_edit",
                                ))
                                .disabled(!can_take_back)
                                .tooltip(if can_take_back {
                                    conduit::i18n::text_static(
                                        "queue.remove_from_queue_and_return_to_input",
                                    )
                                } else {
                                    conduit::i18n::text_static(
                                        "queue.handle_the_current_draft_first",
                                    )
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.take_back_queued_message(id, window, cx);
                                })),
                        )
                        .child(
                            Button::new("remove-queued-message")
                                .debug_selector(|| "remove-queued-message".into())
                                .ghost()
                                .small()
                                .compact()
                                .icon(FullIconName::X)
                                .tooltip(conduit::i18n::text_static("queue.remove_queued_message"))
                                .accessibility_label(conduit::i18n::text_static(
                                    "queue.remove_queued_message",
                                ))
                                .disabled(!enabled || saving)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.queued_messages.remove(&id);
                                    this.persist_and_cleanup_attachments();
                                    cx.notify();
                                })),
                        ),
                )
                .when_some(message.paused.clone(), |card, reason| {
                    card.child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(reason),
                    )
                })
                .into_any_element(),
        )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
#[path = "queue_tests.rs"]
pub(in crate::app) mod tests;
