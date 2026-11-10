//! Bind structured findings to an explicitly prepared review and its immutable patch.
use super::*;
use crate::review::ai::{Rejection, Review};

impl ConduitApp {
    pub(super) fn prepare_structured_review(
        &mut self,
        mut review: Review,
        provider: Provider,
        prompt: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.store.reviews.can_capture_ai(review.patch.len())?;
        self.append_review_draft(
            &review.target.workspace,
            None,
            Some(provider),
            prompt,
            window,
            cx,
        )?;
        let session = self
            .session(self.selected_session.expect("review draft has a session"))
            .expect("review draft session exists");
        review.session = session.id;
        review.session_created_at = session.created_at;
        self.store.reviews.ai.push(review);
        self.persist();
        cx.notify();
        Ok(())
    }

    pub(in crate::app) fn finish_ai_review(
        &mut self,
        id: u64,
        prefix: &str,
        incomplete: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.writable {
            return;
        }
        let Some((workspace, session)) = self.session_control.locate(&self.store.workspaces, id)
        else {
            return;
        };
        let workspace = &self.store.workspaces[workspace];
        let session = &workspace.sessions[session];
        let Some(request) = self.store.reviews.ai.iter().find(|request| {
            request.session == id
                && request.session_created_at == session.created_at
                && request.target.workspace == workspace.path
                && !request.state.complete()
                && session
                    .messages
                    .iter()
                    .find(|m| m.kind == ItemKind::User)
                    .is_some_and(|m| m.text.contains(&request.key))
        }) else {
            return;
        };
        let key = request.key.clone();
        let turn_prefix = format!("{prefix}-");
        let reply = session
            .messages
            .iter()
            .rev()
            .find(|m| m.kind == ItemKind::Assistant && m.key.starts_with(&turn_prefix));
        let message = reply.map(|m| m.key.clone());
        let text = if incomplete {
            Err(Rejection::Incomplete)
        } else {
            reply.map(|m| m.text.as_str()).ok_or(Rejection::NoReply)
        };
        let Some(status) = self.store.reviews.accept_ai(&key, message, text) else {
            return;
        };
        if let Some(target) = self.session_notice_target(id) {
            self.notify_user(
                Notice::new(
                    NoticeSource::Session,
                    NoticeLevel::Info,
                    label("review.ai_reports"),
                    status,
                )
                .target(target)
                .key(format!("review/{key}/{prefix}"), "review")
                .presentation(NoticePresentation::Toast),
                cx,
            );
        }
        self.persist();
        cx.notify();
    }

    pub(super) fn open_ai_review_session(
        &mut self,
        workspace: &Path,
        id: u64,
        created_at: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let index = self
            .store
            .workspaces
            .iter()
            .position(|w| {
                w.path == workspace
                    && w.sessions
                        .iter()
                        .any(|s| s.id == id && s.created_at == created_at && !s.archived)
            })
            .ok_or_else(|| label("review.target_missing").to_owned())?;
        self.switch(index, Some(id), window, cx);
        if self.selected_session != Some(id) || self.selected_workspace != Some(index) {
            return Err(label("review.cannot_attach").into());
        }
        Ok(())
    }
}
