use super::*;
use crate::persistence::SaveReceipt;
use crate::subagents::{ChildCommand, CommandOutcome, PendingChildCommand};

pub(super) struct Pending {
    saved: Option<SaveReceipt>,
    response: Option<std::sync::mpsc::Receiver<CommandOutcome>>,
    command: PendingChildCommand,
    sent: Option<SentChild>,
}

impl ConduitApp {
    pub(super) fn live_child_owner(&self, key: &TaskKey) -> Option<(u64, String, String)> {
        let id = self.saved_child(key)?;
        let binding = self.store.child_bindings.get(&id)?;
        let root = self.session(key.root)?;
        let run = self.session_control.runs.get(&key.root)?;
        let agent = self.routed_agent(key)?;
        if root.provider != Provider::Codex
            || run.prefix != key.run
            || run.runtime.is_cancelled()
            || run.pending_save.is_some()
            || !agent.status.active()
            || agent.direct_input != Some(true)
            || agent.task_id.is_empty()
            || agent.task_id.starts_with("call:")
            || binding.root != key.root
            || root.native_id.as_ref() != Some(&binding.root_native)
            || self.child_source(key.root).as_ref() != Some(&binding.source)
        {
            return None;
        }
        Some((key.root, run.prefix.clone(), agent.task_id.clone()))
    }

    pub(super) fn start_child_control(
        &mut self,
        key: TaskKey,
        text: Option<String>,
        sent: Option<SentChild>,
        cx: &mut Context<Self>,
    ) {
        if self.closing
            || !self.writable
            || self
                .store
                .child_commands
                .iter()
                .any(|pending| pending.root == key.root && pending.command.thread == key.agent)
        {
            return;
        }
        let Some((root, run, turn)) = self.live_child_owner(&key) else {
            return;
        };
        let command = PendingChildCommand {
            root,
            run,
            command: ChildCommand {
                thread: key.agent.clone(),
                turn,
                text,
            },
        };
        self.store.child_commands.push(command.clone());
        match self.persist_confirmed() {
            Ok(receipt) => {
                self.session_control.agent_navigation.commands.insert(
                    key,
                    Pending {
                        saved: Some(receipt),
                        response: None,
                        command,
                        sent,
                    },
                );
            }
            Err(error) => {
                self.store
                    .child_commands
                    .retain(|pending| pending.root != root || pending.command.thread != key.agent);
                if let Some(editor) = self
                    .session_control
                    .agent_navigation
                    .editors
                    .get_mut(&editor::EditorKey::from(&key))
                {
                    editor.error = Some(error);
                }
            }
        }
        cx.notify();
    }

    pub(super) fn stop_child(&mut self, key: &TaskKey, cx: &mut Context<Self>) {
        if self.subagent_route() != Some(key) || self.closing || !self.writable {
            return;
        }
        if let Some(id) = self.saved_child(key)
            && self.session_control.runs.contains_key(&id)
        {
            self.stop_session_run(id);
        } else {
            self.start_child_control(key.clone(), None, None, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn poll_child_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys: Vec<_> = self
            .session_control
            .agent_navigation
            .commands
            .keys()
            .cloned()
            .collect();
        for key in keys {
            let mut pending = self
                .session_control
                .agent_navigation
                .commands
                .remove(&key)
                .unwrap();
            let mut outcome = None;
            if let Some(receipt) = &pending.saved
                && let Some(saved) = receipt.poll()
            {
                pending.saved = None;
                let owner = self.live_child_owner(&key);
                match saved {
                    Err(error) => outcome = Some(CommandOutcome::Rejected(error)),
                    Ok(())
                        if self.closing
                            || owner.as_ref()
                                != Some(&(
                                    pending.command.root,
                                    pending.command.run.clone(),
                                    pending.command.command.turn.clone(),
                                )) =>
                    {
                        outcome = Some(CommandOutcome::Rejected(conduit::i18n::text(
                            "subagents.stale_target",
                        )));
                    }
                    Ok(()) => {
                        pending.response = Some(
                            self.session_control.runs[&pending.command.root]
                                .runtime
                                .child_command(pending.command.command.clone()),
                        );
                    }
                }
            }
            if let Some(response) = &pending.response {
                match response.try_recv() {
                    Ok(result) => outcome = Some(result),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        outcome = Some(CommandOutcome::Unknown(conduit::i18n::text(
                            "subagents.command_unknown",
                        )))
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
            if let Some(outcome) = outcome {
                let error = match &outcome {
                    CommandOutcome::Accepted => {
                        if let Some(sent) = &pending.sent {
                            self.clear_child_draft(sent, window, cx);
                        }
                        None
                    }
                    CommandOutcome::Rejected(error) | CommandOutcome::Unknown(error) => {
                        Some(error.clone())
                    }
                };
                if !matches!(outcome, CommandOutcome::Unknown(_)) {
                    self.store.child_commands.retain(|command| {
                        command.root != key.root || command.command.thread != key.agent
                    });
                }
                if let Some(editor) = self
                    .session_control
                    .agent_navigation
                    .editors
                    .get_mut(&editor::EditorKey::from(&key))
                {
                    editor.error = error;
                }
                self.persist();
                cx.notify();
            } else {
                self.session_control
                    .agent_navigation
                    .commands
                    .insert(key, pending);
            }
        }
    }

    pub(super) fn child_approvals(
        &self,
        key: &TaskKey,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let child = self.saved_child(key);
        let owner = child
            .filter(|id| self.session_control.runs.contains_key(id))
            .or_else(|| {
                self.session_control
                    .runs
                    .get(&key.root)
                    .filter(|run| run.prefix == key.run)
                    .map(|_| key.root)
            });
        let Some(owner) = owner else {
            return div().into_any_element();
        };
        let run = &self.session_control.runs[&owner];
        let approvals: Vec<_> = run
            .approvals
            .iter()
            .filter(|approval| {
                child == Some(owner)
                    || approval.input["threadId"].as_str() == Some(key.agent.as_str())
            })
            .cloned()
            .collect();
        let mut view = v_flex()
            .max_h(px(300.))
            .id("child-approvals")
            .overflow_y_scroll()
            .gap_2();
        for (index, approval) in approvals.into_iter().enumerate() {
            if approval.is_user_input() {
                view = view.child(
                    div()
                        .id(("child-question", index))
                        .child(self.render_user_input(owner, &approval, window, cx)),
                );
                continue;
            }
            let request = approval.id.clone();
            let deny = approval.id.clone();
            let submitted = run.approvals.is_submitted(&approval.id)
                || run.runtime.is_cancelled()
                || self.closing;
            view = view.child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .child(approval.title.clone())
                    .child(
                        div()
                            .text_sm()
                            .child(notifications::approval_message(&approval)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(("child-allow", index))
                                    .small()
                                    .disabled(submitted || !approval.allow_once)
                                    .label(conduit::i18n::text_static("subagents.allow_once"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.submit_approval(owner, &request, true, cx);
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("child-deny", index))
                                    .small()
                                    .disabled(submitted)
                                    .label(conduit::i18n::text_static("subagents.deny"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.submit_approval(owner, &deny, false, cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        view.into_any_element()
    }
}
