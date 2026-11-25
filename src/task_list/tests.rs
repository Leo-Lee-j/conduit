use super::*;
use crate::{
    domain::Provider,
    protocol::{Event, Protocol},
};
use serde_json::json;

fn fixture() -> Vec<Value> {
    serde_json::from_str::<Value>(include_str!("../protocol/fixtures/claude-task-list.json"))
        .unwrap()["frames"]
        .as_array()
        .unwrap()
        .clone()
}
fn session() -> SessionRecord {
    serde_json::from_value(json!({"id":1,"provider":"Claude","created_at":0,"exit_code":null}))
        .unwrap()
}
fn input(key: &str, name: &str, input: Value) -> Value {
    json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":key,"name":name,"input":input}]}})
}
fn result(key: &str, output: Value, failed: bool) -> Value {
    json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":key,"content":"result","is_error":failed}]},"tool_use_result":output})
}

#[test]
fn native_task_fixture_flows_through_protocol_and_restores_every_snapshot() {
    let mut protocol = Protocol::new(Provider::Claude, "C:/fixture".into(), "probe".into(), None);
    let mut session = session();
    let mut snapshots = Vec::new();
    for frame in fixture() {
        let is_result = frame["type"] == "user";
        for event in protocol.accept(frame).events {
            if let Event::Tasks(change) = event {
                apply(&mut session, change);
            }
        }
        // Exercise storage after each event, including explicitly empty lists.
        session = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        if is_result {
            snapshots.push(session.task_list.as_ref().unwrap().steps.clone());
        }
    }
    assert_eq!(snapshots[0][0].id, "1");
    assert_eq!(snapshots[0][0].status, Status::Pending);
    assert_eq!(snapshots[2][0].owner, "developer");
    assert_eq!(snapshots[2][0].status, Status::InProgress);
    assert_eq!(snapshots[3][1].blocked_by, ["1"]);
    assert_eq!(snapshots[4][0].description, "Review task tracking.");
    assert_eq!(snapshots[5][1].description, "Run focused tests.");
    assert_eq!(snapshots[6][0].status, Status::Completed);
    assert_eq!(snapshots[7][1].text, "Verify the task list");
    assert_eq!(snapshots[8].len(), 1);
    assert!(snapshots[9].is_empty() && snapshots[10].is_empty() && snapshots[11].is_empty());
}

#[test]
fn failures_duplicates_unknown_ids_and_child_messages_cannot_change_tasks() {
    let mut tracker = claude::Tracker::default();
    let mut snapshot = Snapshot::default();
    let frames = fixture();
    for frame in &frames[..2] {
        for change in tracker.accept(frame) {
            snapshot.apply(change, None);
        }
    }
    let original = snapshot.clone();
    // Replayed completed call IDs are idempotent, even if the payload changes.
    let mut replay = frames[0].clone();
    replay["message"]["content"][0]["input"]["subject"] = json!("wrong");
    assert!(tracker.accept(&replay).is_empty());
    assert!(tracker.accept(&frames[1]).is_empty());
    for (key, output, failed) in [
        (
            "a",
            json!({"success":false,"taskId":"1","updatedFields":["status"]}),
            false,
        ),
        (
            "b",
            json!({"success":true,"taskId":"1","updatedFields":["status"]}),
            true,
        ),
        (
            "c",
            json!({"success":true,"taskId":"wrong","updatedFields":["status"]}),
            false,
        ),
    ] {
        tracker.accept(&input(
            key,
            "TaskUpdate",
            json!({"taskId":"1","status":"completed"}),
        ));
        assert!(tracker.accept(&result(key, output, failed)).is_empty());
    }
    let mut child = input("child", "TaskCreate", json!({"subject":"child"}));
    child["parent_tool_use_id"] = json!("agent");
    assert!(tracker.accept(&child).is_empty());
    assert!(
        tracker
            .accept(&result(
                "child",
                json!({"task":{"id":"1","subject":"child"}}),
                false
            ))
            .is_empty()
    );
    tracker.accept(&input(
        "unknown",
        "TaskUpdate",
        json!({"taskId":"404","status":"completed"}),
    ));
    for change in tracker.accept(&result(
        "unknown",
        json!({"success":true,"taskId":"404","updatedFields":["status"]}),
        false,
    )) {
        snapshot.apply(change, None);
    }
    assert_eq!(snapshot, original);
}

#[test]
fn new_runtime_updates_restored_tasks_by_id_and_only_acknowledged_fields() {
    let mut session = session();
    let mut tracker = claude::Tracker::default();
    for frame in &fixture()[..4] {
        for change in tracker.accept(frame) {
            apply(&mut session, change);
        }
    }
    session = serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
    let mut tracker = claude::Tracker::default();
    tracker.accept(&input(
        "resume",
        "TaskUpdate",
        json!({"taskId":"1","status":"completed","subject":"unconfirmed"}),
    ));
    for change in tracker.accept(&result(
        "resume",
        json!({"success":true,"taskId":"1","updatedFields":["status"]}),
        false,
    )) {
        apply(&mut session, change);
    }
    let tasks = &session.task_list.unwrap().steps;
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].text, "Inspect the code");
    assert_eq!(tasks[0].status, Status::Completed);
    assert_eq!(tasks[1].status, Status::Pending);
}

#[test]
fn todo_requires_success_and_explicit_empty_list_clears_the_snapshot() {
    let mut tracker = claude::Tracker::default();
    assert!(
        tracker
            .accept(&input(
                "todo",
                "TodoWrite",
                json!({"todos":[{"content":"test","status":"in_progress"}]})
            ))
            .is_empty()
    );
    assert!(
        tracker
            .accept(&result("todo", Value::Null, true))
            .is_empty()
    );
    tracker.accept(&input("empty", "TodoWrite", json!({"todos":[]})));
    assert!(
        matches!(&tracker.accept(&result("empty", Value::Null, false))[..], [Change::Replace { steps, persistent: false }] if steps.is_empty())
    );
    assert!(plan(&json!({"todos":[]})).unwrap().is_empty());
    assert!(plan(&json!({"todos":[{}]})).is_none());
}

#[test]
fn task_scope_survives_new_user_turn_while_turn_plan_expires() {
    let mut session = session();
    apply(
        &mut session,
        wire_plan(vec![("plan".into(), "inProgress".into())]),
    );
    assert_eq!(
        session
            .task_list
            .as_ref()
            .unwrap()
            .visible(&session)
            .unwrap()
            .len(),
        1
    );
    session
        .messages
        .push(serde_json::from_value(json!({"key":"user-1","kind":"User","text":"next"})).unwrap());
    assert!(
        session
            .task_list
            .as_ref()
            .unwrap()
            .visible(&session)
            .is_none()
    );
    apply(
        &mut session,
        Change::Upsert(Task {
            id: "1".into(),
            text: "task".into(),
            ..Default::default()
        }),
    );
    session.messages.push(
        serde_json::from_value(json!({"key":"user-2","kind":"User","text":"continue"})).unwrap(),
    );
    assert_eq!(
        session
            .task_list
            .as_ref()
            .unwrap()
            .visible(&session)
            .unwrap()[0]
            .text,
        "task"
    );
    apply(
        &mut session,
        Change::Replace {
            steps: vec![],
            persistent: true,
        },
    );
    assert_eq!(
        session.task_list.as_ref().unwrap().visible(&session),
        Some([].as_slice())
    );
}

#[test]
fn sdk_readable_results_and_batched_results_remain_correlated() {
    let mut tracker = claude::Tracker::default();
    let frames = fixture();
    let mut snapshot = Snapshot::default();
    for frame in &frames[..10] {
        let mut frame = frame.clone();
        frame.as_object_mut().unwrap().remove("tool_use_result");
        for change in tracker.accept(&frame) {
            snapshot.apply(change, None);
        }
    }
    assert_eq!(snapshot.steps.len(), 2);
    assert_eq!(snapshot.steps[0].status, Status::InProgress);
    assert_eq!(snapshot.steps[1].blocked_by, ["1"]);
    tracker.accept(&input("batch1", "TaskCreate", json!({"subject":"three"})));
    tracker.accept(&input("batch2", "TaskCreate", json!({"subject":"four"})));
    let changes = tracker.accept(&json!({"type":"user","tool_use_result":{"task":{"id":"wrong","subject":"wrong"}},"message":{"content":[
        {"type":"tool_result","tool_use_id":"batch2","content":"Task #4 created successfully: four"},
        {"type":"tool_result","tool_use_id":"batch1","content":"Task #3 created successfully: three"}
    ]}}));
    for change in changes {
        snapshot.apply(change, None);
    }
    assert_eq!(snapshot.steps[2].id, "4");
    assert_eq!(snapshot.steps[3].id, "3");
}

#[test]
fn malformed_or_unrelated_tool_data_does_not_replace_a_list() {
    let mut tracker = claude::Tracker::default();
    tracker.accept(&input("list", "TaskList", json!({})));
    assert!(
        tracker
            .accept(&result("list", json!({"tasks":[{"id":"1"}]}), false))
            .is_empty()
    );
    tracker.accept(&input("read", "Read", json!({"todos":[]})));
    assert!(
        tracker
            .accept(&result("read", json!({"tasks":[]}), false))
            .is_empty()
    );
    let mut protocol = Protocol::new(Provider::Claude, "C:/fixture".into(), "probe".into(), None);
    protocol.accept(input(
        "fail",
        "TaskUpdate",
        json!({"taskId":"1","status":"completed"}),
    ));
    assert!(
        protocol
            .accept(result(
                "fail",
                json!({"success":false,"error":"not found"}),
                false
            ))
            .events
            .iter()
            .any(|e| matches!(
                e,
                Event::Tool {
                    status: crate::message_status::ToolStatus::Failed,
                    ..
                }
            ))
    );
}
