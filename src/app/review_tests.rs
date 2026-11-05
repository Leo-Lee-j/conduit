//! Regression cases from the whole-repository review; no real CLI is started.
use super::{ChatItem, ConduitApp, Provider, Store, Workspace};
use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use std::path::PathBuf;

fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext, PathBuf) {
    cx.update(gpui_kit::init);
    let path = std::env::temp_dir().canonicalize().unwrap();
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.native_id = Some("review-native".into());
            session.title = "手动命名".into();
            let store = Store {
                version: 2,
                workspaces: vec![Workspace {
                    path: path.clone(),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    (app, cx, path)
}
fn item(key: &str, text: &str) -> ChatItem {
    serde_json::from_value(serde_json::json!({"key":key,"kind":"User","text":text})).unwrap()
}
fn imported(messages: Vec<ChatItem>) -> crate::history::ImportedSession {
    crate::history::ImportedSession {
        provider: Provider::Claude,
        native_id: "review-native".into(),
        created_at: 0,
        title: "原生标题".into(),
        model: None,
        messages,
    }
}

#[gpui_kit::test]
fn stale_history_cannot_replace_messages_after_a_turn_completes(cx: &mut TestAppContext) {
    let (app, cx, path) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages = vec![item("user", "first")];
            app.session_control.begin_import(&app.store.workspaces[0]);
            // A started and completed turn changes the revision even though no
            // runtime remains when the scan returns.
            app.session_mut(1)
                .unwrap()
                .messages
                .push(item("new-user", "completed turn"));
            app.finish_history_import(
                &path,
                vec![imported(vec![item("import-0", "first")])],
                false,
                true,
                window,
                cx,
            );
            assert_eq!(app.session(1).unwrap().messages.len(), 2);
            assert_eq!(app.session(1).unwrap().messages[1].text, "completed turn");
            assert_eq!(app.session(1).unwrap().title, "手动命名");
        })
    });
}

#[gpui_kit::test]
fn native_replacement_invalidates_outline_keys_even_with_same_message_count(
    cx: &mut TestAppContext,
) {
    let (app, cx, path) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages = vec![item("import-0", "same prompt")];
            app.outline
                .refresh(Some(&app.store.workspaces[0].sessions[0]));
            assert_eq!(app.outline.entries[0].key, "import-0");
            let revision = app.session_control.global_revision;
            app.finish_history_import(
                &path,
                vec![imported(vec![item("import-99", "same prompt")])],
                false,
                true,
                window,
                cx,
            );
            app.outline
                .refresh(Some(&app.store.workspaces[0].sessions[0]));
            assert_eq!(app.outline.entries[0].key, "import-99");
            assert_ne!(app.session_control.global_revision, revision);
            assert!(app.pending_message_jump.is_none());
            assert_eq!(app.session(1).unwrap().title, "手动命名");
        })
    });
}

#[gpui_kit::test]
fn search_discards_results_after_transcript_changes(cx: &mut TestAppContext) {
    let (app, cx, _) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages = vec![item("user", "old prompt")];
            app.search_query = "prompt".into();
            app.refresh_search(cx, false);
            app.session_mut(1).unwrap().messages = vec![item("user", "new prompt")];
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.search_ui.hits[0].snippet, "new prompt");
            app.refresh_search(cx, false);
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.search_ui.hits.len(), 1);
        assert_eq!(app.search_ui.hits[0].snippet, "new prompt");
    });
}

#[gpui_kit::test]
fn content_updates_do_not_cancel_an_active_search(cx: &mut TestAppContext) {
    let (app, cx, _) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.search_query = "prompt".into();
            app.refresh_search(cx, false);
            let generation = app.search_ui.generation;
            let cancel = app.search_ui.cancel.clone();
            for _ in 0..20 {
                app.session_mut(1).unwrap().messages = vec![item("user", "new prompt")];
                app.refresh_search(cx, false);
                assert_eq!(app.search_ui.generation, generation);
                assert!(!cancel.load(std::sync::atomic::Ordering::Acquire));
            }
        });
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.search_ui.running);
        assert_eq!(app.search_ui.hits[0].snippet, "new prompt");
    });
}

#[gpui_kit::test]
fn history_batches_keep_the_original_revision_until_the_import_finishes(cx: &mut TestAppContext) {
    let (app, cx, path) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages = vec![item("user", "first")];
            app.session_control.begin_import(&app.store.workspaces[0]);
            app.history_loading.insert(path.clone());
            let initial = app.session_control.imports[&path].clone();
            app.finish_history_import(
                &path,
                vec![imported(vec![item("import-0", "first")])],
                false,
                false,
                window,
                cx,
            );
            assert!(app.history_loading.contains(&path));
            assert_eq!(app.session_control.imports[&path], initial);
            app.session_mut(1)
                .unwrap()
                .messages
                .push(item("new-user", "completed turn"));
            app.finish_history_import(
                &path,
                vec![imported(vec![item("import-0", "first")])],
                false,
                true,
                window,
                cx,
            );
            assert!(!app.history_loading.contains(&path));
            assert!(!app.session_control.imports.contains_key(&path));
            assert_eq!(app.session(1).unwrap().messages.len(), 2);
            assert_eq!(app.session(1).unwrap().messages[1].text, "completed turn");
        });
    });
}
