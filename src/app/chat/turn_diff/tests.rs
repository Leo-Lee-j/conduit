use super::{ChatItem, ItemKind, SessionRecord, turn_messages};

fn message(key: &str, kind: ItemKind, text: &str, input: serde_json::Value) -> ChatItem {
    serde_json::from_value(serde_json::json!({
        "key": key, "kind": kind, "text": text,
        "input": input.to_string(), "status": "completed"
    }))
    .unwrap()
}

fn fixture() -> SessionRecord {
    let mut session = crate::app::tests::session();
    session.messages = vec![
        message(
            "old",
            ItemKind::Tool,
            "Write",
            serde_json::json!({"file_path":"src/a.rs", "content":"old turn"}),
        ),
        message(
            "turn",
            ItemKind::User,
            "修改两个文件",
            serde_json::Value::Null,
        ),
        message(
            "first",
            ItemKind::Tool,
            "Edit",
            serde_json::json!({"file_path":"src/a.rs", "old_string":"before", "new_string":"first"}),
        ),
        message(
            "other-file",
            ItemKind::Tool,
            "Write",
            serde_json::json!({"file_path":"src/z.rs", "content":"other file"}),
        ),
        message(
            "second",
            ItemKind::Tool,
            "修改文件",
            serde_json::json!([{"path":"src/a.rs", "kind":"update", "diff":"@@ -1 +1 @@\n-first\n+second\n"}]),
        ),
    ];
    session
}

#[test]
fn file_preview_keeps_the_clicked_turn_boundary() {
    let _locale = crate::test_support::chinese();
    let mut session = fixture();
    session.messages.extend([
        message(
            "later",
            ItemKind::User,
            "next turn",
            serde_json::Value::Null,
        ),
        message(
            "new-edit",
            ItemKind::Tool,
            "Write",
            serde_json::json!({"file_path":"src/a.rs", "content":"later turn"}),
        ),
    ]);
    let keys: Vec<_> = turn_messages(&session, Some("turn"))
        .iter()
        .map(|item| item.key.as_str())
        .collect();
    assert_eq!(keys, ["first", "other-file", "second"]);
    assert_eq!(turn_messages(&session, None)[0].key, "old");
    assert!(turn_messages(&session, Some("missing-turn")).is_empty());
}
