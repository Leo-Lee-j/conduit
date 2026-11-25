use super::*;
use serde_json::json;

fn frames() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("../protocol/fixtures/claude-task-list.json"))
        .unwrap()["native_frames"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn native_task_import_matches_live_state_and_incremental_cache_restore() {
    let frames = frames();
    let first = extend_claude(
        frames[..2].iter().map(Value::to_string),
        ClaudeState::default(),
    );
    let first: ClaudeState = serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
    let middle = extend_claude(frames[2..13].iter().map(Value::to_string), first);
    let imported = middle.clone().finish().unwrap().1;
    let list = imported.task_list.as_ref().unwrap();
    assert_eq!(list.steps.len(), 2);
    assert_eq!(list.steps[1].blocked_by, ["1"]);
    assert_eq!(list.steps[1].description, "Run focused tests.");
    let mut workspace = crate::domain::Workspace {
        path: "C:/fixture".into(),
        sessions: vec![],
    };
    merge_session(&mut workspace, &mut 1, imported);
    assert_eq!(
        workspace.sessions[0]
            .task_list
            .as_ref()
            .unwrap()
            .steps
            .len(),
        2
    );
    let end = extend_claude(frames[13..].iter().map(Value::to_string), middle)
        .finish()
        .unwrap()
        .1;
    merge_session(&mut workspace, &mut 2, end);
    assert!(
        workspace.sessions[0]
            .task_list
            .as_ref()
            .unwrap()
            .steps
            .is_empty()
    );
}

#[test]
fn delayed_native_import_cannot_revert_locally_acknowledged_tasks() {
    let imported = extend_claude(
        frames()[..5].iter().map(Value::to_string),
        ClaudeState::default(),
    )
    .finish()
    .unwrap()
    .1;
    let mut workspace = crate::domain::Workspace {
        path: "C:/fixture".into(),
        sessions: vec![],
    };
    merge_session(&mut workspace, &mut 1, imported.clone());
    let session = &mut workspace.sessions[0];
    session.messages.push(
        serde_json::from_value(json!({"key":"local-user","kind":"User","text":"continue"}))
            .unwrap(),
    );
    crate::task_list::apply(
        session,
        crate::task_list::Change::Replace {
            steps: vec![],
            persistent: true,
        },
    );
    merge_session(&mut workspace, &mut 2, imported);
    assert!(
        workspace.sessions[0]
            .task_list
            .as_ref()
            .unwrap()
            .steps
            .is_empty()
    );
}

#[test]
fn task_list_can_follow_later_native_work_without_accepting_an_older_scan() {
    let frames = frames();
    let first = extend_claude(
        frames[..5].iter().map(Value::to_string),
        ClaudeState::default(),
    )
    .finish()
    .unwrap()
    .1;
    let mut workspace = crate::domain::Workspace {
        path: "C:/fixture".into(),
        sessions: vec![],
    };
    merge_session(&mut workspace, &mut 1, first.clone());
    // Same successful native task, recorded under a GUI turn prefix.
    let item = workspace.sessions[0].messages.last_mut().unwrap();
    assert_eq!(item.text, "TaskCreate");
    item.key = "turn-1-tool_task_1".into();
    let middle = extend_claude(
        frames[..15].iter().map(Value::to_string),
        ClaudeState::default(),
    )
    .finish()
    .unwrap()
    .1;
    merge_session(&mut workspace, &mut 2, middle);
    assert_eq!(
        workspace.sessions[0].task_list.as_ref().unwrap().steps[0].status,
        crate::task_list::Status::Completed
    );
    merge_session(&mut workspace, &mut 2, first);
    assert_eq!(
        workspace.sessions[0].task_list.as_ref().unwrap().steps[0].status,
        crate::task_list::Status::Completed
    );
}
