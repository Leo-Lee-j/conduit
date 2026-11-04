//! One unsent follow-up per conversation; never injected into an active turn.

use super::*;

pub(super) use crate::store::QueuedMessage;

impl ConduitApp {
    /// Keyboard and mouse share the same capacity and concurrency rules.
    pub(super) fn composer_send_blocked(&self) -> Option<&'static str> {
        if let Some(id) = self.send_target(true) {
            if self.queued_messages.contains_key(&id) {
                return Some("已有 1 条待发送消息，请先发送或撤回；当前草稿会保留。");
            }
            if let Some(run) = self.session_control.runs.get(&id) {
                if !run.runtime.is_cancelled() {
                    return None;
                }
                return Some("正在停止当前回复，请稍后发送。");
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
                .is_none_or(|run| run.runtime.is_cancelled())
        {
            return false;
        }
        let prompt = file_ref::compose_prompt(&text, &references);
        if prompt.trim().is_empty() || prompt.len() > 256 * 1024 {
            self.message = Some("消息不能为空，且最多 256 KiB。".into());
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
        if !self.persist() {
            self.queued_messages.remove(&id);
            cx.notify();
            return false;
        }
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
        if self.closing || !self.writable {
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
            self.pause_queued_message(id, "会话已不存在，请取回待发送消息。");
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
                &format!(
                    "附件已不可引用，请取回编辑：{}",
                    rejection_summary(&rejected)
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
                            "待发送消息已暂停",
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
        if self.closing || !self.writable || self.selected_session != Some(id) {
            return;
        }
        // Never overwrite typed text or attachments, including a pending engine switch.
        if !self.input.read(cx).value().is_empty()
            || self
                .current_files()
                .is_some_and(|files| !files.references.is_empty())
        {
            self.message = Some("请先处理输入框中的草稿，再取回待发送消息。".into());
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
            "仅附件消息".into()
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
        let can_take_back = enabled
            && self.input.read(cx).value().is_empty()
            && self
                .current_files()
                .is_none_or(|files| files.references.is_empty());
        let can_send = enabled && !self.session_busy(id);
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
                                .child(if paused {
                                    "待发送 · 已暂停"
                                } else {
                                    "排队中"
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(palette().text))
                                .child(summary),
                        )
                        .when(!message.references.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!("{} 个附件", message.references.len())),
                            )
                        })
                        .when(paused, |row| {
                            row.child(
                                Button::new("send-queued-message")
                                    .debug_selector(|| "send-queued-message".into())
                                    .ghost()
                                    .small()
                                    .compact()
                                    .label("发送")
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
                                .accessibility_label("取回编辑")
                                .disabled(!can_take_back)
                                .tooltip(if can_take_back {
                                    "撤回队列并放回输入框"
                                } else {
                                    "请先处理输入框中的草稿"
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
                                .tooltip("移除待发送消息")
                                .accessibility_label("移除待发送消息")
                                .disabled(!enabled)
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
mod tests;
