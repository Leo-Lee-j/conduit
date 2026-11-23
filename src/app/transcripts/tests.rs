use super::{ActiveRun, ChatRuntime, ConduitApp, MAX_LOADED, Store, queue};
use crate::transcript::tests::Fixture;
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext};

fn fixture(
    cx: &mut TestAppContext,
    count: usize,
    cold: bool,
) -> (Entity<ConduitApp>, &mut VisualTestContext, Fixture) {
    let disk = Fixture::new();
    let store = disk.store(count);
    store.save_to(&disk.state).unwrap();
    let store = if cold {
        Store::load_from(&disk.state).unwrap()
    } else {
        store
    };
    fixture_with_store(cx, store, disk)
}

fn fixture_with_store(
    cx: &mut TestAppContext,
    store: Store,
    disk: Fixture,
) -> (Entity<ConduitApp>, &mut VisualTestContext, Fixture) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.persistence = crate::persistence::Persistence::new(disk.state.clone());
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    (app, cx, disk)
}

#[gpui_kit::test]
fn answered_codex_questions_remain_visible_after_navigation_and_restart(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let disk = Fixture::new();
    let mut store = disk.store(12);
    let session = &mut store.workspaces[0].sessions[0];
    session.provider = crate::domain::Provider::Codex;
    session.messages.insert(
        1,
        serde_json::from_value(serde_json::json!({
            "key":"turn-0-permission-approval-1", "kind":"Tool", "text":"需要你的回答",
            "approval": {"request":"approval-1", "status":"已回答", "questions":[{
                "id":"question_1", "header":"", "question":"使用哪个引擎？",
                "options":[["Codex",""],["Claude Code",""]], "multiple":false,
                "secret":false, "selected":[0], "answers":["Codex"]
            }]}
        }))
        .unwrap(),
    );
    store.save_to(&disk.state).unwrap();
    let (app, cx, disk) = fixture_with_store(cx, store, disk);
    let selector = "tool-1-turn-0-permission-approval-1-question-history";
    for restart in [false, true] {
        if restart {
            let restored = Store::load_from(&disk.state).unwrap();
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.store = restored;
                    app.timeline_session = None;
                })
            });
        }
        for id in [1, 2, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 1] {
            cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(id), window, cx)));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
                let _ = window.draw(cx);
            });
            if id == 1 {
                cx.update(|_, cx| {
                    let app = app.read(cx);
                    let question = &app.session(1).unwrap().messages[1];
                    let history = super::super::user_input::history(question).unwrap();
                    assert_eq!(
                        history.status,
                        crate::message_status::ApprovalStatus::Answered
                    );
                    assert_eq!(history.questions[0].answers, ["Codex"]);
                    assert!(!super::super::tools::is_groupable_activity(question));
                });
                assert!(
                    cx.debug_bounds(selector).is_some(),
                    "question history missing after restart={restart}"
                );
            }
        }
    }
}

#[gpui_kit::test]
fn startup_only_loads_the_selected_body_and_navigation_bounds_the_cache(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _disk) = fixture(cx, 100, true);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_session, Some(100));
        assert_eq!(
            app.store.workspaces[0]
                .sessions
                .iter()
                .filter(|session| session.transcript.loaded)
                .count(),
            1
        );
        assert_eq!(app.session(100).unwrap().messages[1].text, "original reply");
        assert_eq!(app.session(1).unwrap().message_count(), 2);
    });
    for id in 1..=12 {
        cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(id), window, cx)));
        cx.run_until_parked();
    }
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_session, Some(12));
        assert!(app.session(12).unwrap().transcript.loaded);
        assert!(!app.session(1).unwrap().transcript.loaded);
        assert_eq!(
            app.store.workspaces[0]
                .sessions
                .iter()
                .filter(|session| session.transcript.loaded)
                .count(),
            MAX_LOADED
        );
    });
}

#[gpui_kit::test]
fn running_queued_current_and_unsaved_bodies_survive_eviction(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, disk) = fixture(cx, 12, false);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let (runtime, _events) = ChatRuntime::test_channel();
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    disk.root.clone(),
                    "turn-1".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app.queued_messages.insert(
                2,
                queue::QueuedMessage {
                    text: "next".into(),
                    references: vec![],
                    paused: Some("paused".into()),
                },
            );
            app.session_mut(3).unwrap().messages[1].text = "not yet saved".into();
            app.trim_transcripts();
            for id in [1, 2, 3, 12] {
                assert!(app.session(id).unwrap().transcript.loaded);
            }
            assert_eq!(app.session(3).unwrap().messages[1].text, "not yet saved");
            assert_eq!(
                app.store.workspaces[0]
                    .sessions
                    .iter()
                    .filter(|session| session.transcript.loaded)
                    .count(),
                MAX_LOADED
            );
        })
    });
}

#[gpui_kit::test]
fn load_completion_preserves_metadata_and_never_resurrects_deleted_session(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _disk) = fixture(cx, 3, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(3).unwrap().title = "renamed while loading".into();
            app.request_transcript(1, cx);
            app.delete_session(1, cx);
            app.switch(0, Some(2), window, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.session(1).is_none());
        assert_eq!(app.selected_session, Some(2));
        assert_eq!(app.session(3).unwrap().title, "renamed while loading");
        assert_eq!(app.session(2).unwrap().messages.len(), 2);
    });
}

#[gpui_kit::test]
fn missing_body_has_error_retry_and_blocks_send_without_looking_empty(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let disk = Fixture::new();
    disk.store(2).save_to(&disk.state).unwrap();
    let store = Store::load_from(&disk.state).unwrap();
    let reference = store.workspaces[0]
        .sessions
        .iter()
        .find(|session| session.id == 2)
        .unwrap()
        .transcript
        .reference
        .as_ref()
        .unwrap();
    let body = reference.directory.join(format!("{}.json", reference.hash));
    let bytes = std::fs::read(&body).unwrap();
    // Remove the body before startup can schedule a read of the selected session.
    std::fs::remove_file(&body).unwrap();
    let (app, cx, disk) = fixture_with_store(cx, store, disk);
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(2), window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.transcripts.errors.contains_key(&2));
            assert!(!app.session(2).unwrap().transcript.loaded);
            assert!(app.send_blocked(true).is_some());
            app.request_delete_session(2, cx);
            assert_eq!(app.delete_target, Some(2));
            assert!(app.session(2).is_some());
            app.delete_target = None;
        });
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("transcript-loading").is_some());
    std::fs::write(&body, bytes).unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.retry_transcript(2, cx)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.transcripts.errors.contains_key(&2));
        assert!(app.send_blocked(true).is_none());
        assert_eq!(app.session(2).unwrap().messages.len(), 2);
        assert!(disk.state.exists());
    });
}

#[gpui_kit::test]
fn stale_load_cannot_replace_a_new_imported_reference(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, disk) = fixture(cx, 1, true);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut imported = crate::transcript::load(app.session(1).unwrap().clone()).unwrap();
            imported.messages[1].text = "newer imported reply".into();
            crate::transcript::save(&imported, &crate::transcript::directory(&disk.state)).unwrap();
            assert!(crate::transcript::unload(&mut imported));
            app.store.workspaces[0].sessions[0] = imported;
            app.ensure_selected_transcript(cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session(1).unwrap().messages[1].text,
            "newer imported reply"
        );
        assert!(app.transcripts.tasks.is_empty());
    });
}
