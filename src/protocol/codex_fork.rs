use super::*;
use crate::commands::RequestIntent;

impl Protocol {
    /// A fork is a metadata operation. Never resume/start a model turn as a fallback.
    pub(super) fn codex_fork(&mut self, value: Value) -> Step {
        let mut step = Step::new();
        let RequestIntent::Fork(request) = &self.intent else {
            return step;
        };
        if self.provider != Provider::Codex || self.native_id.is_some() || !request.valid() {
            step.done = true;
            step.error = Some(conduit::i18n::text("branching.unsupported"));
            return step;
        }
        if value.get("id").is_some()
            && let Some(error) = value.get("error")
        {
            self.codex_fork_id = None;
            step.done = true;
            step.error = Some(pretty(error));
            return step;
        }
        match value.get("id").and_then(Value::as_u64) {
            Some(1) if value.get("result").is_some() => {
                step.writes.push(json!({"method":"initialized"}));
                let mut params = json!({
                    "threadId": request.thread_id, "lastTurnId": request.last_turn_id,
                    "cwd": self.cwd, "ephemeral": false, "excludeTurns": true,
                    "deferGoalContinuation": true,
                    "approvalPolicy": self.permission.codex_approval_policy(),
                    "approvalsReviewer": "user", "sandbox": self.permission.codex_sandbox()
                });
                if let Some(model) = &self.model {
                    params["model"] = json!(model);
                }
                if !self.route.config.is_empty() {
                    params["config"] = json!(self.route.config);
                    if let Some(provider) = self.route.config.get("model_provider") {
                        params["modelProvider"] = provider.clone();
                    }
                }
                step.writes
                    .push(json!({"id":2,"method":"thread/fork","params":params}));
            }
            Some(2) if value.get("result").is_some() => {
                let thread = &value["result"]["thread"];
                let id = thread["id"].as_str().unwrap_or_default();
                let parent_matches = thread["forkedFromId"]
                    .as_str()
                    .is_none_or(|parent| parent == request.thread_id);
                if id.is_empty() || id == request.thread_id || !parent_matches {
                    step.error = Some(conduit::i18n::text("branching.invalid_response"));
                    step.done = true;
                } else {
                    self.codex_fork_id = Some(id.into());
                    // A one-turn page verifies that older CLIs did not silently
                    // ignore lastTurnId, without hydrating the whole transcript.
                    step.writes
                        .push(json!({"id":3,"method":"thread/turns/list","params":{
                            "threadId":id,"limit":1,"sortDirection":"desc","itemsView":"notLoaded"
                        }}));
                }
            }
            Some(3) if value.get("result").is_some() => {
                let last = value["result"]["data"]
                    .as_array()
                    .filter(|turns| turns.len() == 1)
                    .and_then(|turns| turns.first());
                if let Some(id) = self.codex_fork_id.take()
                    && last.is_some_and(|turn| {
                        turn["id"] == request.last_turn_id && turn["status"] == "completed"
                    })
                {
                    self.thread_id = Some(id.clone());
                    step.events.push(Event::NativeId(id));
                } else {
                    step.error = Some(conduit::i18n::text("branching.invalid_response"));
                }
                step.done = true;
            }
            _ => {
                // Unexpected server requests must not stall a zero-model operation or ask for
                // tools/permissions inherited from a source with a goal or background activity.
                if let Some(id) = value.get("id")
                    && value.get("method").is_some()
                {
                    step.writes.push(json!({"id":id,"error":{"code":-32601,"message":"Fork operation does not handle server requests"}}));
                }
            }
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> Protocol {
        Protocol::new(
            Provider::Codex,
            "C:/work".into(),
            "must never reach a model".into(),
            None,
        )
        .with_intent(RequestIntent::Fork(crate::branching::ForkRequest {
            thread_id: "source".into(),
            last_turn_id: "chosen-turn".into(),
        }))
    }

    #[test]
    fn branch_boundaries_require_the_confirmed_thread_and_successful_turn() {
        let completion = |thread: &str, turn: &str, status: &str| {
            json!({
                "method":"turn/completed", "params":{"threadId":thread,"turn":{"id":turn,"status":status}}
            })
        };
        for (event, expected) in [
            (completion("source", "selected", "completed"), true),
            (completion("other", "selected", "completed"), false),
            (completion("source", "other", "completed"), false),
            (completion("source", "selected", "failed"), false),
            (
                json!({"method":"turn/completed","params":{"turn":{"id":"selected","status":"completed"}}}),
                false,
            ),
        ] {
            let mut protocol = Protocol::new(
                Provider::Codex,
                "C:/work".into(),
                "test".into(),
                Some("source".into()),
            );
            protocol.accept(json!({"id":2,"result":{"thread":{"id":"source"}}}));
            protocol.accept(json!({"id":3,"result":{"turn":{"id":"selected"}}}));
            let step = protocol.accept(event);
            assert_eq!(
                step.events
                    .iter()
                    .any(|event| matches!(event, Event::NativeTurnCompleted { .. })),
                expected
            );
        }
    }

    #[test]
    fn fork_copies_the_requested_completed_turn_without_starting_or_resuming_a_model() {
        let mut protocol = protocol();
        protocol.prepare().unwrap();
        let step = protocol.accept(json!({"id":1,"result":{}}));
        assert_eq!(step.writes[1]["method"], "thread/fork");
        assert_eq!(step.writes[1]["params"]["lastTurnId"], "chosen-turn");
        assert_eq!(step.writes[1]["params"]["deferGoalContinuation"], true);
        assert_eq!(step.writes[1]["params"]["excludeTurns"], true);
        assert!(!step.writes[1].to_string().contains("must never"));
        let step = protocol.accept(json!({"id":2,"result":{"thread":{
            "id":"fork", "forkedFromId":"source", "turns":[]
        }}}));
        assert!(!step.done && step.events.is_empty());
        assert_eq!(step.writes[0]["method"], "thread/turns/list");
        assert_eq!(
            step.writes[0]["params"],
            json!({"threadId":"fork","limit":1,"sortDirection":"desc","itemsView":"notLoaded"})
        );
        let step = protocol
            .accept(json!({"id":3,"result":{"data":[{"id":"chosen-turn","status":"completed"}]}}));
        assert!(step.done && step.error.is_none() && step.writes.is_empty());
        assert!(matches!(&step.events[..], [Event::NativeId(id)] if id == "fork"));
    }

    #[test]
    fn invalid_or_unsupported_forks_never_bind_a_session_or_fall_back_to_a_prompt() {
        for thread in [
            json!({"id":"source"}),
            json!({"id":"fork","forkedFromId":"other","turns":[{"id":"chosen-turn","status":"completed"}]}),
            json!({}),
        ] {
            let step = protocol().accept(json!({"id":2,"result":{"thread":thread}}));
            assert!(
                step.done
                    && step.error.is_some()
                    && step.events.is_empty()
                    && step.writes.is_empty()
            );
        }
        for data in [
            json!([{ "id":"later-turn", "status":"completed" }]),
            json!([{ "id":"chosen-turn", "status":"inProgress" }]),
            json!([]),
            json!([{ "id":"chosen-turn", "status":"completed" }, { "id":"later-turn", "status":"completed" }]),
            Value::Null,
        ] {
            let mut protocol = protocol();
            protocol
                .accept(json!({"id":2,"result":{"thread":{"id":"fork","forkedFromId":"source"}}}));
            let step = protocol.accept(json!({"id":3,"result":{"data":data}}));
            assert!(
                step.done
                    && step.error.is_some()
                    && step.events.is_empty()
                    && step.writes.is_empty()
            );
            assert!(protocol.thread_id.is_none());
        }
        let step =
            protocol().accept(json!({"id":2,"error":{"code":-32601,"message":"unsupported"}}));
        assert!(step.done && step.error.is_some() && step.writes.is_empty());
        let mut wrong_engine = protocol();
        wrong_engine.provider = Provider::Claude;
        assert!(wrong_engine.prepare().is_err());
    }
}
