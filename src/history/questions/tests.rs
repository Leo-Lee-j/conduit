use super::*;
use serde_json::json;

fn parse(values: &[Value]) -> super::super::ImportedSession {
    let text = std::iter::once(
        json!({"type":"session_meta","payload":{"id":"thread","cwd":"D:/questions"}}),
    )
    .chain(values.iter().cloned())
    .map(|value| value.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    super::super::parse_codex_any(&text, None).unwrap().1
}

fn event(item: Value) -> Value {
    json!({"type":"event_msg","payload":{"type":"item_completed","item":item}})
}

fn question() -> Value {
    event(
        json!({"type":"AgentMessage","id":"ask-1","delivery":"async",
        "content":[{"type":"Text","text":"Choose an engine"}],
        "questions":[{"title":"Which engine?","options":["A","B"]},{"title":"Project name?"}]}),
    )
}

fn user(text: &str) -> Value {
    event(json!({"type":"UserMessage","id":"user-1","content":[{"type":"text","text":text}]}))
}

fn synchronous() -> Value {
    json!({"type":"response_item","payload":{"type":"function_call","call_id":"sync-1","name":"request_user_input",
        "arguments":json!({"questions":[
            {"id":"choice","header":"Engine","question":"Which engine?","options":[{"label":"A","description":"First"},{"label":"B","description":"Second"}]},
            {"id":"secret","question":"Password?","isSecret":true}
        ]}).to_string()}})
}

fn output(answers: Value) -> Value {
    json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"sync-1","output":json!({"answers":answers}).to_string()}})
}

#[test]
fn native_async_questions_and_legacy_replies_restore_one_answered_card() {
    let replies = json!([
        {"questionItemId":json!(["request_user_input_async","ask-1",0]).to_string().replace('"',"\\\""),"answer":"B"},
        {"questionItemId":json!(["request_user_input_async","ask-1",1]).to_string(),"answer":"My project"}
    ]);
    let text = format!(
        "<send_user_message_question_reply>\n{replies}\n</send_user_message_question_reply>"
    );
    let imported = parse(&[
        question(),
        user(&text),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}),
    ]);
    assert_eq!(imported.messages.len(), 1);
    let item = &imported.messages[0];
    assert_eq!(item.kind, ItemKind::Tool);
    let approval = item.approval.as_ref().unwrap();
    assert_eq!(approval.status, ApprovalStatus::Answered);
    assert_eq!(approval.questions[0].selected, [1]);
    assert_eq!(approval.questions[0].answers, ["B"]);
    assert_eq!(approval.questions[1].answers, ["My project"]);
}

#[test]
fn synchronous_questions_survive_structured_tools_incremental_cache_and_redact_secrets() {
    let mut state = super::super::CodexState::default();
    let values = [
        json!({"type":"session_meta","payload":{"id":"thread","cwd":"D:/questions"}}),
        synchronous(),
        event(
            json!({"type":"CommandExecution","id":"tool","command":"pwd","status":"completed","exit_code":0}),
        ),
    ];
    state = super::super::extend_codex(
        values.into_iter().map(|value| value.to_string()),
        None,
        state,
    );
    let initial = state.clone().finish().unwrap().1;
    assert_eq!(initial.messages.len(), 2);
    assert_eq!(
        initial.messages[0].approval.as_ref().unwrap().status,
        ApprovalStatus::Interrupted
    );
    state = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    let result =
        output(json!({"choice":{"answers":["B"]},"secret":{"answers":["sensitive-password"]}}));
    let state = super::super::extend_codex(std::iter::once(result.to_string()), None, state);
    assert!(
        !serde_json::to_string(&state)
            .unwrap()
            .contains("sensitive-password")
    );
    let imported = state.finish().unwrap().1;
    let approval = imported.messages[0].approval.as_ref().unwrap();
    assert_eq!(approval.status, ApprovalStatus::Answered);
    assert_eq!(approval.questions[0].selected, [1]);
    assert_eq!(approval.questions[0].options[1].1, "Second");
    assert!(approval.questions[1].answers.is_empty());
    assert!(approval.questions[1].selected.is_empty());
    let mut previous = initial.messages;
    restore_imported(&mut previous, &imported.messages);
    assert_eq!(
        previous[0].approval.as_ref().unwrap().status,
        ApprovalStatus::Answered
    );
}

#[test]
fn conduit_async_answers_and_skips_are_restored_without_duplicate_question_snapshots() {
    for skipped in [false, true] {
        let replies = json!([{"requestId":"ask-1","questions":[
            {"question":"Which engine?","answers":if skipped {Value::Null} else {json!(["A"])},"skipped":skipped},
            {"question":"Project name?","answers":if skipped {Value::Null} else {json!(["Custom name"])},"skipped":skipped}
        ]}]);
        let text = format!(
            "Conduit paused execution to wait for the user's response to your questions. Here are the submitted responses (skipped questions have no answer):\n{replies}\nContinue the original task using these responses. Do not infer answers to skipped questions."
        );
        let imported = parse(&[question(), question(), user(&text)]);
        assert_eq!(imported.messages.len(), 1);
        let approval = imported.messages[0].approval.as_ref().unwrap();
        assert_eq!(
            approval.status,
            if skipped {
                ApprovalStatus::Skipped
            } else {
                ApprovalStatus::Answered
            }
        );
        assert_eq!(approval.questions[0].answers.len(), usize::from(!skipped));
    }
}

#[test]
fn reimport_repairs_old_bubbles_and_missing_cards_without_replacing_local_answers() {
    let imported = parse(&[
        user("task"),
        question(),
        event(
            json!({"type":"AgentMessage","id":"reply","content":[{"type":"Text","text":"done"}]}),
        ),
    ]);
    let mut previous = imported.messages.clone();
    previous[1].kind = ItemKind::Assistant;
    previous[1].approval = None;
    previous[1].model = Some("model".into());
    restore_imported(&mut previous, &imported.messages);
    assert_eq!(previous[1].kind, ItemKind::Tool);
    assert!(previous[1].approval.is_some());
    previous.remove(1);
    for _ in 0..2 {
        restore_imported(&mut previous, &imported.messages);
    }
    assert_eq!(previous.len(), 3);
    assert_eq!(previous[1].key, imported.messages[1].key);
    previous[1].key = "turn-7-ask-1".into();
    previous[1].kind = ItemKind::Assistant;
    previous[1].approval = None;
    restore_imported(&mut previous, &imported.messages);
    assert_eq!(previous[1].key, "turn-7-ask-1");
    assert_eq!(previous[1].kind, ItemKind::Tool);
    assert!(previous[1].approval.is_some());
    previous[1].key = "turn-0-permission-approval-1".into();
    previous[1].approval.as_mut().unwrap().status = ApprovalStatus::Answered;
    previous[1].approval.as_mut().unwrap().questions[0].answers = vec!["Local answer".into()];
    restore_imported(&mut previous, &imported.messages);
    let merged = super::super::merge_transcript(&previous, imported.messages);
    assert_eq!(merged.len(), 3);
    assert_eq!(
        merged[1].approval.as_ref().unwrap().questions[0].answers,
        ["Local answer"]
    );
}

#[test]
fn new_turn_questions_are_appended_once_after_their_prompt() {
    let first = [
        user("first task"),
        event(
            json!({"type":"AgentMessage","id":"first-reply","content":[{"type":"Text","text":"first answer"}]}),
        ),
    ];
    let mut previous = parse(&first).messages;
    let incoming = parse(&[
        first[0].clone(), first[1].clone(),
        event(json!({"type":"UserMessage","id":"second-user","content":[{"type":"Text","text":"second task"}]})),
        question(),
    ]).messages;
    restore_imported(&mut previous, &incoming);
    assert_eq!(previous.len(), 2);
    let merged = super::super::merge_transcript(&previous, incoming);
    assert_eq!(merged.len(), 4);
    assert_eq!(merged[2].text, "second task");
    assert!(merged[3].approval.is_some());
}

#[test]
fn reimport_repairs_cold_history_and_persists_answers_across_restart() {
    let _locale = crate::test_support::chinese();
    let replies = json!([{"questionItemId":json!(["request_user_input_async","ask-1",0]).to_string(),"answer":"B"}]);
    let text = format!(
        "<send_user_message_question_reply>\n{replies}\n</send_user_message_question_reply>"
    );
    let imported = parse(&[
        user("task"),
        question(),
        user(&text),
        event(json!({
            "type":"AgentMessage","id":"reply","content":[{"type":"Text","text":"done"}]
        })),
    ]);
    let disk = crate::transcript::tests::Fixture::new();
    let mut store = disk.store(1);
    let session = &mut store.workspaces[0].sessions[0];
    session.provider = Provider::Codex;
    session.native_id = Some("thread".into());
    session.messages = imported.messages.clone();
    session.messages[1].kind = ItemKind::Assistant;
    session.messages[1].approval = None;
    session.messages.insert(
        2,
        serde_json::from_value(json!({
            "key":"import-3-user-1", "kind":"User", "text":text
        }))
        .unwrap(),
    );
    store.save_to(&disk.state).unwrap();
    for _ in 0..2 {
        let mut restored = crate::store::Store::load_from(&disk.state).unwrap();
        assert!(!restored.workspaces[0].sessions[0].transcript.loaded);
        let (records, warnings) = super::super::prepare_imported(
            &restored.workspaces[0],
            vec![imported.clone()],
            &crate::transcript::directory(&disk.state),
        );
        assert!(warnings.is_empty());
        assert_eq!(records.len(), 1);
        restored.workspaces[0].sessions = records;
        restored.save_to(&disk.state).unwrap();
        let reloaded = crate::transcript::load(restored.workspaces[0].sessions[0].clone()).unwrap();
        assert_eq!(reloaded.messages.len(), 3);
        let question = &reloaded.messages[1];
        assert_eq!(question.kind, ItemKind::Tool);
        let approval = question.approval.as_ref().unwrap();
        assert_eq!(approval.status, ApprovalStatus::Answered);
        assert_eq!(approval.questions[0].answers, ["B"]);
        assert_eq!(approval.questions[0].selected, [1]);
    }
}

#[test]
fn unrelated_and_malformed_reply_text_stays_visible() {
    for text in [
        "<send_user_message_question_reply>not json</send_user_message_question_reply>",
        "<send_user_message_question_reply>[{\"questionItemId\":\"[\\\"request_user_input_async\\\",\\\"other\\\",0]\",\"answer\":\"B\"}]</send_user_message_question_reply>",
        "Please explain <send_user_message_question_reply>.",
    ] {
        let imported = parse(&[question(), user(text)]);
        assert_eq!(imported.messages.len(), 2);
        assert_eq!(imported.messages[1].text, text);
        assert_eq!(
            imported.messages[0].approval.as_ref().unwrap().status,
            ApprovalStatus::Interrupted
        );
    }
}
