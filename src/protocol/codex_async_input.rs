//! Native async questions acknowledge delivery immediately. Pause the turn,
//! collect explicit answers, then continue in the same thread as a new turn.
use super::*;
use std::collections::BTreeMap;

pub(super) struct PendingInput {
    turn_id: String,
    stopped: bool,
    responses: BTreeMap<String, Option<Value>>,
}

impl Protocol {
    pub(super) fn accept_codex_async_input(&mut self, params: &Value, step: &mut Step) {
        let item = &params["item"];
        let key = text(item, "id");
        if self
            .codex_async_input
            .as_ref()
            .is_some_and(|pending| pending.responses.contains_key(key))
        {
            return; // started/completed can carry the same question snapshot.
        }
        let turn_id = params["turnId"]
            .as_str()
            .or(self.turn_id.as_deref())
            .unwrap_or_default()
            .to_owned();
        let questions = normalize_questions(&item["questions"]);
        if self.text_only
            || key.trim().is_empty()
            || self.thread_id.as_deref().is_none_or(str::is_empty)
            || turn_id.is_empty()
            || questions.is_none()
        {
            invalid_question(step);
            return;
        }
        self.accept_user_input(
            &item["id"],
            json!({"questions": questions.unwrap()}),
            ApprovalKind::CodexAsyncUserInput,
            step,
        );
        if step.done {
            return;
        }
        if self.codex_async_input.is_none() {
            // This is a client request, not a reply to a fabricated server RPC.
            step.writes.push(json!({
                "id": format!("conduit-question-interrupt:{turn_id}"),
                "method": "turn/interrupt",
                "params": {"threadId": self.thread_id, "turnId": turn_id}
            }));
        }
        let pending = self.codex_async_input.get_or_insert_with(|| PendingInput {
            turn_id,
            stopped: false,
            responses: BTreeMap::new(),
        });
        pending.responses.insert(key.into(), None);
    }

    pub(super) fn codex_async_input_completed(&mut self, params: &Value, step: &mut Step) -> bool {
        let Some(pending) = &mut self.codex_async_input else {
            return false;
        };
        if params["turn"]["id"] != pending.turn_id
            || !matches!(text(&params["turn"], "status"), "interrupted" | "completed")
        {
            return false;
        }
        // An interrupted native turn is a wait boundary, not task completion.
        pending.stopped = true;
        if let Some(resume) = self.resume_codex_async_input() {
            step.writes.push(resume);
        }
        true
    }

    pub(super) fn codex_async_interrupt_acknowledged(&self, value: &Value) -> bool {
        value["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("conduit-question-interrupt:"))
            && value.get("method").is_none()
            && (value.get("error").is_none()
                || self
                    .codex_async_input
                    .as_ref()
                    .is_none_or(|pending| pending.stopped))
        // A turn can complete naturally before interrupt is processed. Once its
        // completion is observed, an idle-turn interrupt error is harmless. A
        // failure to interrupt a still-running turn must instead fail closed.
    }

    pub(super) fn codex_async_answer(
        &self,
        approval: &Approval,
        answers: Option<&BTreeMap<String, Vec<String>>>,
    ) -> Value {
        // This local payload is consumed by route_response, never sent as RPC.
        let questions: Vec<_> = approval
            .questions()
            .into_iter()
            .map(|q| {
                json!({
                    "question": q.question,
                    "answers": answers.and_then(|a| a.get(&q.id)),
                    "skipped": answers.is_none()
                })
            })
            .collect();
        json!({"requestId": approval.wire_id, "questions": questions})
    }

    pub(crate) fn route_codex_async_input(
        &mut self,
        approval: &Approval,
        response: Value,
    ) -> anyhow::Result<Option<Value>> {
        let slot = self
            .codex_async_input
            .as_mut()
            .and_then(|pending| pending.responses.get_mut(approval.wire_id.as_str()?));
        let Some(slot) = slot.filter(|slot| slot.is_none()) else {
            anyhow::bail!(conduit::i18n::text(
                "protocol.user_input.invalid_cli_question_request_stopped"
            ));
        };
        *slot = Some(response);
        Ok(self.resume_codex_async_input())
    }

    fn resume_codex_async_input(&mut self) -> Option<Value> {
        let pending = self.codex_async_input.as_ref()?;
        if !pending.stopped || pending.responses.values().any(Option::is_none) {
            return None;
        }
        let pending = self.codex_async_input.take()?;
        let answers: Vec<_> = pending.responses.into_values().flatten().collect();
        let message = format!(
            "Conduit paused execution to wait for the user's response to your questions. \
             Here are the submitted responses (skipped questions have no answer):\n{}\n\
             Continue the original task using these responses. Do not infer answers to skipped questions.",
            pretty(&json!(answers))
        );
        // turn/started may arrive before the turn/start response. Allow the new
        // native turn to bind its ID before its first streamed item.
        self.turn_id = None;
        Some(json!({
            "id": 3, "method": "turn/start",
            "params": {"threadId": self.thread_id, "input": [{
                "type": "text", "text": message, "text_elements": []
            }]}
        }))
    }
}

fn normalize_questions(value: &Value) -> Option<Vec<Value>> {
    let questions = value.as_array().filter(|questions| !questions.is_empty())?;
    questions
        .iter()
        .enumerate()
        .map(|(index, q)| {
            let title = q["title"].as_str().filter(|s| !s.trim().is_empty())?;
            let options = match q.get("options").filter(|v| !v.is_null()) {
                None => vec![],
                Some(options) => options
                    .as_array()?
                    .iter()
                    .map(|option| {
                        let label = option.as_str().filter(|s| !s.trim().is_empty())?;
                        Some(json!({"label": label, "description": ""}))
                    })
                    .collect::<Option<Vec<_>>>()?,
            };
            Some(json!({
                "id": format!("question_{}", index + 1), "question": title,
                "options": options, "isOther": true
            }))
        })
        .collect()
}

fn invalid_question(step: &mut Step) {
    step.done = true;
    step.error = Some(conduit::i18n::text(
        "protocol.user_input.invalid_cli_question_request_stopped",
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol(permission: PermissionLevel) -> Protocol {
        let mut protocol = Protocol::new(Provider::Codex, "C:/work".into(), "task".into(), None)
            .with_permission(permission);
        protocol.accept(json!({"id":2,"result":{"thread":{"id":"thread"}}}));
        protocol.accept(json!({"id":3,"result":{"turn":{"id":"turn"}}}));
        protocol
    }

    fn question(method: &str, id: &str) -> Value {
        json!({"method":method,"params":{"threadId":"thread","turnId":"turn","item":{
            "type":"agentMessage","id":id,"delivery":"async","text":"Which? A / B",
            "questions":[{"title":"Which?","options":["A","B"]},{"title":"Project name?"}]
        }}})
    }

    fn stopped(protocol: &mut Protocol, status: &str) -> Step {
        protocol.accept(json!({"method":"turn/completed","params":{
            "threadId":"thread","turn":{"id":"turn","status":status}
        }}))
    }

    fn approval(step: &Step) -> Approval {
        assert!(!step.done);
        assert!(step.error.is_none());
        let [Event::Approval(approval)] = step.events.as_slice() else {
            panic!(
                "question must appear once as a structured approval: {:?}",
                step.events
            );
        };
        approval.clone()
    }

    fn answer(protocol: &mut Protocol, approval: &Approval) -> Option<Value> {
        let answers = BTreeMap::from([
            ("question_1".into(), vec!["B".into()]),
            ("question_2".into(), vec!["Custom project name".into()]),
        ]);
        let response = protocol.user_input_response(approval, &answers).unwrap();
        protocol
            .route_codex_async_input(approval, response)
            .unwrap()
    }

    fn assert_resume(resume: &Value) {
        assert_eq!(resume["method"], "turn/start");
        assert_eq!(resume["params"]["threadId"], "thread");
        assert!(
            resume.get("result").is_none(),
            "async notifications have no server RPC to reply to"
        );
        let text = resume["params"]["input"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("Which?") && text.contains("B") && text.contains("Custom project name")
        );
    }

    #[test]
    fn async_questions_wait_for_both_the_user_and_native_turn_to_stop() {
        let _locale = crate::test_support::chinese();
        for permission in [
            PermissionLevel::ReadOnly,
            PermissionLevel::WorkspaceWrite,
            PermissionLevel::FullAccess,
        ] {
            for early_answer in [false, true] {
                for status in ["interrupted", "completed"] {
                    let mut protocol = protocol(permission);
                    let step = protocol.accept(question("item/started", "call_question"));
                    let approval = approval(&step);
                    assert_eq!(step.writes.len(), 1);
                    assert_eq!(step.writes[0]["method"], "turn/interrupt");
                    assert_eq!(
                        step.writes[0]["params"],
                        json!({"threadId":"thread","turnId":"turn"})
                    );
                    assert_eq!(protocol.automatic_approval(&approval), None);
                    protocol.enable_full_access();
                    assert_eq!(protocol.automatic_approval(&approval), None);
                    assert!(
                        protocol
                            .user_input_response(&approval, &BTreeMap::new())
                            .is_none()
                    );
                    let duplicate = protocol.accept(question("item/completed", "call_question"));
                    assert!(duplicate.events.is_empty() && duplicate.writes.is_empty());
                    let questions = approval.questions();
                    assert_eq!(
                        questions[0].options,
                        vec![("A".into(), String::new()), ("B".into(), String::new())]
                    );
                    assert!(questions.iter().all(|q| q.custom));
                    assert!(questions[1].options.is_empty());
                    if early_answer {
                        assert!(answer(&mut protocol, &approval).is_none());
                        let completed = stopped(&mut protocol, status);
                        assert!(!completed.done && completed.events.is_empty());
                        assert_eq!(completed.writes.len(), 1);
                        assert_resume(&completed.writes[0]);
                    } else {
                        let completed = stopped(&mut protocol, status);
                        assert!(
                            !completed.done
                                && completed.events.is_empty()
                                && completed.writes.is_empty()
                        );
                        assert_resume(&answer(&mut protocol, &approval).unwrap());
                    }
                    // A new turn can start before its RPC response arrives.
                    protocol.accept(json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"resumed"}}}));
                    assert_eq!(protocol.turn_id.as_deref(), Some("resumed"));
                    assert!(
                        !stopped(&mut protocol, "completed").done,
                        "old completion cannot finish the resumed turn"
                    );
                    let done = protocol.accept(json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"resumed","status":"completed"}}}));
                    assert!(done.done && done.error.is_none());
                }
            }
        }
    }

    #[test]
    fn multiple_async_requests_require_every_response_and_keep_explicit_skips() {
        let _locale = crate::test_support::chinese();
        let mut protocol = protocol(PermissionLevel::FullAccess);
        let first = approval(&protocol.accept(question("item/completed", "first")));
        let second_step = protocol.accept(question("item/completed", "second"));
        let second = approval(&second_step);
        assert!(
            second_step.writes.is_empty(),
            "interrupt only once per native turn"
        );
        assert!(!stopped(&mut protocol, "interrupted").done);
        assert!(answer(&mut protocol, &first).is_none());
        let response = protocol.approval_response(&second, false);
        let resume = protocol
            .route_codex_async_input(&second, response.clone())
            .unwrap()
            .unwrap();
        assert_resume(&resume);
        assert!(
            resume["params"]["input"][0]["text"]
                .as_str()
                .unwrap()
                .contains("\"skipped\": true")
        );
        assert!(protocol.route_codex_async_input(&second, response).is_err());
    }

    #[test]
    fn interrupt_errors_only_fail_before_native_completion_is_confirmed() {
        let _locale = crate::test_support::chinese();
        let error =
            json!({"id":"conduit-question-interrupt:turn","error":{"message":"no active turn"}});
        let mut running = protocol(PermissionLevel::FullAccess);
        approval(&running.accept(question("item/completed", "question")));
        let failed = running.accept(error.clone());
        assert!(failed.done && failed.error.is_some());

        let mut waiting = protocol(PermissionLevel::FullAccess);
        let question = approval(&waiting.accept(question("item/completed", "question")));
        assert!(!stopped(&mut waiting, "completed").done);
        let acknowledged = waiting.accept(error.clone());
        assert!(!acknowledged.done && acknowledged.error.is_none());
        assert_resume(&answer(&mut waiting, &question).unwrap());
        let late = waiting.accept(error);
        assert!(!late.done && late.error.is_none());
    }

    #[test]
    fn async_question_failures_are_not_silently_ignored() {
        let _locale = crate::test_support::chinese();
        for questions in [
            json!([]),
            json!([{"title":""}]),
            json!([{"title":"Which?","options":[5]}]),
        ] {
            let mut protocol = protocol(PermissionLevel::FullAccess);
            let mut event = question("item/completed", "question");
            event["params"]["item"]["questions"] = questions;
            let step = protocol.accept(event);
            assert!(step.done && step.error.is_some());
            assert!(step.writes.is_empty() && step.events.is_empty());
        }
        let mut protocol = protocol(PermissionLevel::FullAccess);
        approval(&protocol.accept(question("item/completed", "question")));
        let failure = stopped(&mut protocol, "failed");
        assert!(
            failure.done && failure.error.is_some(),
            "real native failures still end the task"
        );
    }
}
