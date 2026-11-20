use super::{TaskKey, editor};
use crate::app::{ActiveRun, ChatRuntime, ConduitApp, Event, Provider};
use crate::cli_adapter::child_history::Snapshot;
use crate::subagents::{Agent, Status};
use gpui_kit::TestAppContext;
use gpui_kit::{Context, Window};
use serde_json::json;
use std::time::Instant;

fn prepare(app: &mut ConduitApp, window: &mut Window, cx: &mut Context<ConduitApp>) -> TaskKey {
    let root = app.session_mut(1).unwrap();
    root.provider = Provider::Codex;
    root.native_id = Some("parent".into());
    root.subagents = vec![Agent {
        id: "child".into(),
        parent_id: "parent".into(),
        run: "turn-0".into(),
        task_id: "native-turn".into(),
        direct_input: Some(true),
        status: Status::Running,
        ..Default::default()
    }];
    app.open_subagent(1, "turn-0".into(), "child".into(), window, cx);
    app.subagent_route().unwrap().clone()
}
fn snapshot(text: &str) -> Snapshot {
    Snapshot {
        thread: json!({"id":"child","createdAt":1}),
        turns: vec![json!({
            "id":"native-turn","status":"completed","items":[
                {"type":"userMessage","id":"user","content":[{"type":"text","text":"task"}]},
                {"type":"agentMessage","id":"reply","text":text}
            ]
        })],
    }
}
fn install(app: &mut ConduitApp, key: &TaskKey, text: &str) -> u64 {
    let binding = app.history_binding(key).unwrap();
    app.install_child_history(key, binding, snapshot(text));
    app.session_control
        .agent_navigation
        .refreshed
        .insert(key.clone(), Instant::now());
    app.visible_subagent_transcript().unwrap()
}

#[gpui_kit::test]
fn child_history_live_merge_is_idempotent_and_keeps_parent_untouched(cx: &mut TestAppContext) {
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "child-history-merge");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let before = app.session(1).unwrap().messages.clone();
            let key = prepare(app, window, cx);
            app.apply_child_event(
                1,
                "turn-0",
                "child",
                "native-turn",
                Event::Delta {
                    key: "reply".into(),
                    text: "new complete".into(),
                },
            );
            let id = install(app, &key, "new");
            assert_eq!(app.session(id).unwrap().messages.len(), 2);
            assert_eq!(app.session(id).unwrap().messages[1].text, "new complete");
            app.apply_child_event(
                1,
                "turn-0",
                "child",
                "native-turn",
                Event::Text {
                    key: "reply".into(),
                    text: "final".into(),
                },
            );
            app.apply_child_event(
                1,
                "turn-0",
                "child",
                "native-turn",
                Event::Delta {
                    key: "reply".into(),
                    text: "late duplicate".into(),
                },
            );
            install(app, &key, "new");
            assert_eq!(app.session(id).unwrap().messages.len(), 2);
            assert_eq!(app.session(id).unwrap().messages[1].text, "final");
            assert_eq!(
                serde_json::to_value(&app.session(1).unwrap().messages).unwrap(),
                serde_json::to_value(before).unwrap()
            );
            assert!(app.session(id).unwrap().is_subagent);
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(
                app.store.workspaces[0]
                    .sessions
                    .iter()
                    .filter(|session| session.is_subagent)
                    .count(),
                1
            );
            // An existing record from a different source must never be duplicated or overwritten.
            app.store.child_bindings.get_mut(&id).unwrap().source = "different-codex-home".into();
            let binding = app.history_binding(&key).unwrap();
            let count = app.store.workspaces[0].sessions.len();
            app.install_child_history(&key, binding, snapshot("wrong source"));
            assert_eq!(app.store.workspaces[0].sessions.len(), count);
            assert_eq!(app.session(id).unwrap().messages[1].text, "final");
            assert!(
                app.session_control
                    .agent_navigation
                    .history_errors
                    .contains_key(&key)
            );
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn independent_child_editors_restore_selection_drafts_routes_and_attachments(
    cx: &mut TestAppContext,
) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "child-editors");
    let attachment = root.join("child.txt");
    std::fs::write(&attachment, "fixture").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input.update(cx, |input, cx| {
                input.set_value("root draft", window, cx);
                input.set_selected_range(1..4, cx);
            });
            let root_input = app.input.clone();
            let key = prepare(app, window, cx);
            let owner = editor::EditorKey::from(&key);
            let input = app.session_control.agent_navigation.editors[&owner]
                .input
                .clone();
            input.update(cx, |input, cx| {
                input.set_value("child draft", window, cx);
                input.set_selected_range(2..5, cx);
            });
            let mut saved = app.saved_child_editors(cx);
            // Deserialize the same disk representation used by normal exit/update recovery.
            let mut json = serde_json::to_value(&saved).unwrap();
            json[0]["paths"] = json!([attachment, root.join("missing.txt")]);
            saved = serde_json::from_value(json).unwrap();
            let routes = app.saved_child_routes();
            let pages = app.saved_child_pages();
            app.return_from_subagent(1, window, cx);
            assert_eq!(app.input.entity_id(), root_input.entity_id());
            assert_eq!(app.input.read(cx).value().as_str(), "root draft");
            assert_eq!(app.input.read(cx).selected_range(), 1..4);
            app.restore_child_navigation(saved, routes, pages, window, cx);
            assert_eq!(app.subagent_route(), Some(&key));
            let input = &app.session_control.agent_navigation.editors[&owner].input;
            assert_eq!(input.read(cx).value().as_str(), "child draft");
            assert_eq!(input.read(cx).selected_range(), 2..5);
            assert!(app.child_attachment_paths().contains(&attachment));
            assert!(
                app.child_attachment_paths()
                    .contains(&root.join("missing.txt")),
                "missing attachments stay recoverable"
            );
            assert!(app.session_control.runs.is_empty());
        })
    });
}

#[gpui_kit::test]
fn child_control_save_gate_binds_owner_and_ack_does_not_clear_new_draft(cx: &mut TestAppContext) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "child-control-save");
    let (runtime, next) = ChatRuntime::test_child_channel();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let key = prepare(app, window, cx);
            let id = install(app, &key, "saved");
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    root.clone(),
                    "turn-0".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            let owner = editor::EditorKey::from(&key);
            let input = app.session_control.agent_navigation.editors[&owner]
                .input
                .clone();
            input.update(cx, |input, cx| input.set_value("send to child", window, cx));
            app.send_child(window, cx);
            assert_eq!(app.store.child_commands.len(), 1);
            assert!(next().is_none(), "must wait for durable command snapshot");
            app.persistence.flush();
            app.poll_child_controls(window, cx);
            let (command, response) = next().expect("command released after save");
            assert_eq!(command.thread, "child");
            assert_eq!(command.turn, "native-turn");
            assert_eq!(command.text.as_deref(), Some("send to child"));
            input.update(cx, |input, cx| input.set_value("new draft", window, cx));
            app.return_from_subagent(1, window, cx);
            response
                .send(crate::subagents::CommandOutcome::Accepted)
                .unwrap();
            app.poll_child_controls(window, cx);
            assert_eq!(input.read(cx).value().as_str(), "new draft");
            assert!(app.store.child_commands.is_empty());
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
            assert!(app.child_resume_blocked(id).is_some());
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn child_control_revalidates_turn_after_save_and_retains_uncertain_commands(
    cx: &mut TestAppContext,
) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "child-control-stale");
    let (runtime, next) = ChatRuntime::test_child_channel();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let key = prepare(app, window, cx);
            install(app, &key, "saved");
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    root,
                    "turn-0".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app.start_child_control(key.clone(), None, None, cx);
            app.session_mut(1).unwrap().subagents[0].task_id = "new-turn".into();
            app.persistence.flush();
            app.poll_child_controls(window, cx);
            assert!(next().is_none());
            assert!(app.store.child_commands.is_empty());
            app.start_child_control(key.clone(), None, None, cx);
            app.persistence.flush();
            app.poll_child_controls(window, cx);
            let (command, response) = next().unwrap();
            assert_eq!(command.turn, "new-turn");
            response
                .send(crate::subagents::CommandOutcome::Unknown(
                    "disconnected".into(),
                ))
                .unwrap();
            app.poll_child_controls(window, cx);
            assert_eq!(app.store.child_commands.len(), 1);
            app.start_child_control(key, None, None, cx);
            assert!(
                next().is_none(),
                "unconfirmed commands must never be resent"
            );
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn restored_child_drafts_require_original_identity_and_valid_attachments(cx: &mut TestAppContext) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "child-draft-identity");
    let (runtime, next) = ChatRuntime::test_child_channel();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let key = prepare(app, window, cx);
            install(app, &key, "saved");
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    root.clone(),
                    "turn-0".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            let owner = editor::EditorKey::from(&key);
            app.session_control.agent_navigation.editors[&owner]
                .input
                .clone()
                .update(cx, |input, cx| {
                    input.set_value("recovered child draft", window, cx)
                });
            let baseline = serde_json::to_value(app.saved_child_editors(cx)).unwrap();
            for (field, value) in [
                ("root_native", json!("previous-parent")),
                ("source", json!(root.join("previous-codex-home"))),
                ("paths", json!([root.join("missing.txt")])),
            ] {
                let mut changed = baseline.clone();
                changed[0][field] = value;
                app.restore_child_navigation(
                    serde_json::from_value(changed).unwrap(),
                    vec![key.clone()],
                    vec![],
                    window,
                    cx,
                );
                app.send_child(window, cx);
                assert!(
                    app.store.child_commands.is_empty(),
                    "blocked draft: {field}"
                );
                assert!(next().is_none(), "must not send restored draft: {field}");
                assert_eq!(
                    app.session_control.agent_navigation.editors[&owner]
                        .input
                        .read(cx)
                        .value()
                        .as_str(),
                    "recovered child draft"
                );
            }
            // Restoring the exact binding and removing the unavailable file unblocks the same draft.
            app.restore_child_navigation(
                serde_json::from_value(baseline).unwrap(),
                vec![key],
                vec![],
                window,
                cx,
            );
            app.send_child(window, cx);
            assert_eq!(app.store.child_commands.len(), 1);
            assert!(next().is_none(), "valid draft still waits for durable save");
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn root_deletion_waits_for_child_run_then_releases_navigation(cx: &mut TestAppContext) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "child-root-delete");
    let (runtime, _) = ChatRuntime::test_child_channel();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let key = prepare(app, window, cx);
            let id = install(app, &key, "saved");
            app.session_control.runs.insert(
                id,
                ActiveRun::new(
                    runtime,
                    root,
                    "independent-turn".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app.delete_session(1, cx);
            assert!(app.session(1).is_some());
            assert!(!app.session_control.runs[&id].runtime.is_cancelled());
            assert_eq!(app.subagent_route(), Some(&key));
            app.session_control.runs.remove(&id);
            let owner = editor::EditorKey::from(&key);
            let input = app.session_control.agent_navigation.editors[&owner]
                .input
                .clone();
            input.update(cx, |input, cx| {
                input.set_value("unsent child draft", window, cx)
            });
            app.delete_session(1, cx);
            assert!(
                app.session(1).is_some(),
                "unsent child draft protects its root"
            );
            assert_eq!(input.read(cx).value().as_str(), "unsent child draft");
            input.update(cx, |input, cx| input.set_value("", window, cx));
            app.delete_session(1, cx);
            assert!(app.session(1).is_none());
            assert!(app.saved_child_editors(cx).is_empty());
            assert!(app.saved_child_routes().is_empty());
            assert!(app.saved_child_pages().is_empty());
            assert!(!app.store.child_bindings.contains_key(&id));
            app.persistence.flush();
        })
    });
}
