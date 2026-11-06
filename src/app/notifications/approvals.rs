//! Live approval capabilities are separate from persisted navigation/history.
use super::*;
use crate::{notifications::ApprovalOutcome, protocol::Approval};

#[derive(Clone)]
pub(super) struct ApprovalBinding {
    session: u64,
    runtime: String,
    request: String,
}

/// Prefer the command or affected paths to protocol IDs. Keep history bounded
/// and leave the complete request in the conversation's approval details.
pub(in crate::app) fn approval_message(approval: &Approval) -> String {
    let details: serde_json::Value = serde_json::from_str(&approval.detail).unwrap_or_default();
    let values = [&approval.input, &details];
    let command = values.iter().find_map(|value| {
        [
            "/command",
            "/input/command",
            "/toolInput/command",
            "/toolCall/rawInput/command",
        ]
        .iter()
        .find_map(|path| value.pointer(path).filter(|value| !value.is_null()))
    });
    let path = values.iter().find_map(|value| {
        [
            "/file_path",
            "/path",
            "/input/file_path",
            "/toolInput/changes",
            "/changes",
            "/toolCall/locations",
        ]
        .iter()
        .find_map(|path| value.pointer(path).filter(|value| !value.is_null()))
    });
    let summary = command
        .or(path)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string())
        })
        .unwrap_or_else(|| {
            if !approval.input.is_null() {
                approval.input.to_string()
            } else if !details.is_null() {
                details.to_string()
            } else {
                approval.detail.clone()
            }
        });
    let summary = summary.trim();
    if summary.is_empty() || summary == approval.title {
        return approval.title.clone();
    }
    let mut preview: String = summary.chars().take(480).collect();
    if preview.len() < summary.len() {
        preview.push_str("…（完整请求请查看会话）");
    }
    format!("{}\n{preview}", approval.title)
}

impl ConduitApp {
    pub(in crate::app) fn notify_approval(
        &mut self,
        session: u64,
        prefix: &str,
        approval: &Approval,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.session_notice_target(session) else {
            return;
        };
        let Some(run) = self.session_control.runs.get(&session) else {
            return;
        };
        let runtime = run.runtime.identity().to_owned();
        let question = approval.is_user_input();
        // Questions also need a new notification when regeneration reuses a ticket.
        let key = format!("approval/{session}/{prefix}/{runtime}/{}", approval.id);
        let id = self.notify_user(
            Notice::new(
                Source::Session,
                Level::Warning,
                if question {
                    "会话等待你的回答"
                } else {
                    "会话等待审批"
                },
                if question {
                    approval.title.clone()
                } else {
                    approval_message(approval)
                },
            )
            .target(target)
            .key(key, "waiting")
            .active()
            .presentation(if self.session_is_visible(session) {
                Presentation::History
            } else {
                Presentation::Toast
            }),
            cx,
        );
        if !question
            && self
                .notifications
                .center
                .get(id)
                .is_some_and(|record| record.lifecycle == Lifecycle::Active)
        {
            self.notifications.approvals.insert(
                id,
                ApprovalBinding {
                    session,
                    runtime,
                    request: approval.id.clone(),
                },
            );
            self.refresh_notifications(cx);
        }
    }

    fn live_notification_approval(&self, id: u64) -> Option<(&ApprovalBinding, &ActiveRun)> {
        if self.closing {
            return None;
        }
        let record = self.notifications.center.get(id)?;
        if record.lifecycle != Lifecycle::Active {
            return None;
        }
        let binding = self.notifications.approvals.get(&id)?;
        let target = record.target.as_ref()?;
        if !matches!(target, Target::Session { session, .. } if *session == binding.session)
            || self.notification_session_index(target).is_none()
        {
            return None;
        }
        let run = self.session_control.runs.get(&binding.session)?;
        if run.runtime.identity() != binding.runtime || run.runtime.is_cancelled() {
            return None;
        }
        let approval = run.approvals.get(&binding.request)?;
        (!approval.is_user_input()).then_some((binding, run))
    }

    pub(super) fn notification_approval_state(&self, id: u64) -> Option<(bool, bool)> {
        let (binding, run) = self.live_notification_approval(id)?;
        Some((
            run.approvals.get(&binding.request)?.allow_once,
            run.approvals.is_submitted(&binding.request),
        ))
    }

    pub(super) fn decide_notification(
        &mut self,
        id: u64,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Revalidate at click time; a rendered button may outlive its request.
        let binding = self
            .live_notification_approval(id)
            .map(|(binding, _)| binding.clone());
        if let Some(binding) = binding {
            self.submit_approval(binding.session, &binding.request, allow, cx);
        }
        self.dismiss_read_toasts(window, cx);
        self.refresh_notifications(cx);
    }

    pub(in crate::app) fn finish_notification_approval(
        &mut self,
        session: u64,
        request: &str,
        outcome: ApprovalOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.session_control.runs.get(&session) else {
            return;
        };
        let runtime = run.runtime.identity();
        let key = format!("approval/{session}/{}/{runtime}/{request}", run.prefix);
        let ids: Vec<_> = self
            .notifications
            .approvals
            .iter()
            .filter_map(|(id, binding)| {
                (binding.session == session
                    && binding.runtime == runtime
                    && binding.request == request)
                    .then_some(*id)
            })
            .collect();
        for id in ids {
            self.notifications.approvals.remove(&id);
            self.notifications
                .center
                .resolve_approval(id, outcome, unix_now());
        }
        // Questions have no action binding. Resolve their key after tool outcomes
        // have been recorded, since resolve_approval requires an active record.
        self.notifications.center.resolve(&key, unix_now());
        self.refresh_notifications(cx);
    }

    pub(super) fn expire_notification_approvals(&mut self) {
        let stale: Vec<_> = self
            .notifications
            .approvals
            .keys()
            .copied()
            .filter(|id| self.live_notification_approval(*id).is_none())
            .collect();
        for id in stale {
            self.notifications.approvals.remove(&id);
            self.notifications
                .center
                .resolve_approval(id, ApprovalOutcome::Expired, unix_now());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::approval_message;
    use crate::protocol::{Approval, ApprovalKind};

    fn approval(input: serde_json::Value, detail: serde_json::Value) -> Approval {
        Approval {
            id: "ticket".into(),
            title: "等待审批".into(),
            detail: detail.to_string(),
            wire_id: serde_json::Value::Null,
            input,
            allow_once: true,
            kind: ApprovalKind::Command,
        }
    }

    #[test]
    fn previews_surface_commands_and_affected_files_across_provider_shapes() {
        for (input, detail, expected) in [
            (
                serde_json::json!({"itemId":"tool"}),
                serde_json::json!({"toolInput":{"command":"cargo check"}}),
                "cargo check",
            ),
            (
                serde_json::json!({"command":"git status"}),
                serde_json::Value::Null,
                "git status",
            ),
            (
                serde_json::json!({"file_path":"src/main.rs","content":"large content"}),
                serde_json::Value::Null,
                "src/main.rs",
            ),
            (
                serde_json::Value::Null,
                serde_json::json!({"toolInput":{"changes":[{"path":"src/app.rs","kind":"update"}]}}),
                "src/app.rs",
            ),
            (
                serde_json::json!({"toolCall":{"rawInput":{"command":"cargo test"}}}),
                serde_json::Value::Null,
                "cargo test",
            ),
        ] {
            let preview = approval_message(&approval(input, detail));
            assert!(preview.starts_with("等待审批\n"));
            assert!(preview.contains(expected), "{preview}");
            assert!(!preview.contains("large content"));
        }
    }

    #[test]
    fn oversized_previews_are_bounded_and_direct_to_complete_details() {
        let request = approval(
            serde_json::json!({"command":"检查目录 ".repeat(200)}),
            serde_json::Value::Null,
        );
        let preview = approval_message(&request);
        assert!(preview.ends_with("…（完整请求请查看会话）"));
        assert!(preview.chars().count() < 520);
    }
}
