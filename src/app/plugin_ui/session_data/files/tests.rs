use super::{Changes, ChatItem, DiffFile, EditFileStatus};
use serde_json::json;

fn edit(key: &str, path: &str, kind: &str, patch: &str) -> ChatItem {
    serde_json::from_value(json!({
        "key": key, "kind": "Tool", "text": "fileChange", "status": "completed",
        "input": json!([{"path": path, "kind": kind, "diff": patch}]).to_string()
    }))
    .unwrap()
}

fn messages() -> Vec<ChatItem> {
    vec![
        edit(
            "create",
            "C:/project/new.txt",
            "add",
            "@@ -0,0 +1,2 @@\n+first\n+second\n",
        ),
        serde_json::from_value(json!({"key":"next", "kind":"User", "text":"Next turn"})).unwrap(),
        edit(
            "update",
            "new.txt",
            "update",
            "@@ -1 +1 @@\n-first\n+latest\n",
        ),
        edit(
            "other",
            "C:/project/other.txt",
            "update",
            "@@ -1 +1,2 @@\n-before\n+after\n+extra\n",
        ),
        edit(
            "delete",
            "C:/project/gone.txt",
            "delete",
            "@@ -1 +0,0 @@\n-removed\n",
        ),
    ]
}

#[test]
fn session_files_merge_turns_paths_and_keep_only_the_latest_diff() {
    let _locale = crate::test_support::chinese();
    let changes = Changes::collect(&messages(), "C:/project");
    assert_eq!(
        changes.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["gone.txt", "new.txt", "other.txt"]
    );
    assert_eq!((changes.added, changes.removed), (5, 3));
    let new = &changes.files["new.txt"];
    assert_eq!(new.status, EditFileStatus::Added);
    assert_eq!((new.added, new.removed), (3, 1));
    assert!(new.latest.patch.contains("+latest"));
    assert!(!new.latest.patch.contains("+first"));
    assert_eq!(changes.files["other.txt"].status, EditFileStatus::Modified);
    assert_eq!(changes.files["gone.txt"].status, EditFileStatus::Deleted);
}

#[test]
fn failed_pending_and_denied_edits_do_not_replace_completed_changes() {
    let _locale = crate::test_support::chinese();
    let mut items = messages();
    for status in ["failed", "running", "cancelled", "interrupted"] {
        let mut item = edit(status, "new.txt", "delete", "@@ -1 +0,0 @@\n-no\n");
        item.status = status.into();
        items.push(item);
    }
    let mut denied = edit("denied", "new.txt", "delete", "@@ -1 +0,0 @@\n-no\n");
    denied.approval = Some(crate::domain::ToolApproval {
        request: "approval".into(),
        status: crate::message_status::ApprovalStatus::Denied,
        questions: Vec::new(),
    });
    items.push(denied);
    let changes = Changes::collect(&items, "C:/project");
    assert_eq!((changes.added, changes.removed), (5, 3));
    assert_eq!(changes.files["new.txt"].status, EditFileStatus::Added);
    assert!(changes.files["new.txt"].latest.patch.contains("+latest"));
}

#[test]
fn session_files_keep_diff_previews_beyond_the_inline_tool_limit() {
    let _locale = crate::test_support::chinese();
    let changes: Vec<_> = (0..12)
        .map(|index| {
            json!({
                "path": format!("file-{index}.txt"), "kind": "add",
                "diff": "@@ -0,0 +1 @@\n+content\n"
            })
        })
        .collect();
    let item: ChatItem = serde_json::from_value(json!({
        "key": "many", "kind": "Tool", "text": "fileChange", "status": "completed",
        "input": serde_json::to_string(&changes).unwrap()
    }))
    .unwrap();
    let changes = Changes::collect(&[item], "C:/project");
    assert_eq!(changes.files.len(), 12);
    assert_eq!(changes.added, 12);
    for file in changes.files.values() {
        assert_eq!(DiffFile::parse(&file.latest.patch).unwrap().len(), 1);
        assert!(file.latest.patch.contains("+content"));
    }
}

#[test]
fn session_files_and_diff_survive_restart_without_workspace_files() {
    let _locale = crate::test_support::chinese();
    let fixture = crate::transcript::tests::Fixture::new();
    let mut store = fixture.store(1);
    store.workspaces[0].sessions[0].messages = messages();
    // Save before ever building the card, as happens when its panel stays closed.
    store.save_to(&fixture.state).unwrap();
    drop(store);
    let restored = crate::store::Store::load_from(&fixture.state).unwrap();
    let cold = &restored.workspaces[0].sessions[0];
    assert!(!cold.transcript.loaded);
    assert!(cold.messages.is_empty());
    let loaded = crate::transcript::load(cold.clone()).unwrap();
    let changes = Changes::collect(&loaded.messages, "C:/project");
    assert_eq!(changes.files.len(), 3);
    assert_eq!((changes.added, changes.removed), (5, 3));
    let new = &changes.files["new.txt"];
    assert_eq!(new.status, EditFileStatus::Added);
    let files = DiffFile::parse(&new.latest.patch).unwrap();
    assert_eq!(files.len(), 1);
    assert!(new.latest.patch.contains("-first"));
    assert!(new.latest.patch.contains("+latest"));
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn session_files_double_click_opens_latest_diff_without_edit_navigation(
    cx: &mut gpui_kit::TestAppContext,
) {
    use conduit::plugins::{builtins, ids::ViewId};
    use gpui_kit::{Modifiers, MouseButton, MouseUpEvent, VisualTestContext, px, size};

    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "session-files");
    let draw = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    };
    cx.simulate_resize(size(px(1400.), px(1100.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().messages = vec![
                edit("first", "src/a.txt", "add", "@@ -0,0 +1 @@\n+first\n"),
                serde_json::from_value(json!({"key":"next", "kind":"User", "text":"Next turn"}))
                    .unwrap(),
                edit(
                    "last",
                    "src/a.txt",
                    "update",
                    "@@ -1 +1 @@\n-first\n+latest\n",
                ),
            ];
            assert!(app.open_right_view(&ViewId::new(builtins::SESSION_DATA).unwrap(), window, cx));
            cx.notify();
        });
    });
    draw(cx);
    draw(cx);
    let position = cx
        .debug_bounds("session-data-file-0")
        .expect("file row")
        .center();
    assert!(cx.debug_bounds("session-data-file-1").is_none());
    assert!(cx.debug_bounds("turn-diff-preview").is_none());
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds("turn-diff-preview").is_none(),
        "single click must not open diff"
    );
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    draw(cx);
    assert!(cx.debug_bounds("turn-diff-preview").is_some());
    assert!(cx.debug_bounds("turn-diff-edit-0").is_none());
    assert!(cx.debug_bounds("turn-diff-previous-edit").is_none());
    assert!(cx.debug_bounds("turn-diff-next-edit").is_none());
    let value = cx.update(|_, cx| {
        let app = app.read(cx);
        let state = app.plugins.session_data.values().next().unwrap().read(cx);
        state.files.value.as_ref().unwrap().clone()
    });
    assert_eq!(value.files.len(), 1);
    assert!(value.files["src/a.txt"].latest.patch.contains("+latest"));
}
