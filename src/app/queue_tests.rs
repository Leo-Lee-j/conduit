use crate::app::{ActiveRun, ChatRuntime, ConduitApp, Event, ItemKind, Provider, Store, Workspace};
use crate::event_queue::Sender;
use gpui_kit::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use std::path::PathBuf;

pub(in crate::app) fn fixture<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (Entity<ConduitApp>, &'a mut VisualTestContext, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "conduit-message-queue-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let shim = root.join("offline.cmd");
    // Offline launcher: never invokes an installed model or accesses the network.
    std::fs::write(&shim, "@echo off\r\nexit /b 0\r\n").unwrap();
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut first = crate::app::tests::session();
            first.native_id = Some("queue-first".into());
            let mut second = first.clone();
            second.id = 2;
            second.native_id = Some("queue-second".into());
            let store = Store {
                version: 2,
                workspaces: vec![
                    Workspace {
                        path: root.clone(),
                        sessions: vec![first],
                    },
                    Workspace {
                        path: root.join("other"),
                        sessions: vec![second],
                    },
                ],
                cli: crate::cli::CliSettings {
                    claude_path: Some(shim.clone()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.store.ui.auto_rename = false;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.new_session_provider = Provider::Claude;
            app.queued_messages.clear();
            app.session_control.runs.clear();
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    (app, cx, root)
}

pub(in crate::app) fn running(app: &mut ConduitApp, id: u64) -> Sender {
    let (runtime, sender) = ChatRuntime::test_channel();
    let path = app
        .store
        .workspaces
        .iter()
        .find(|workspace| workspace.sessions.iter().any(|session| session.id == id))
        .unwrap()
        .path
        .clone();
    app.session_control.runs.insert(
        id,
        ActiveRun::new(
            runtime,
            path,
            "turn-0".into(),
            Default::default(),
            None,
            Default::default(),
        ),
    );
    sender
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn stop_shortcut_only_cancels_selected_run_and_retains_queue(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "stop-shortcut");
    let mut senders = Vec::new();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            senders.push(running(this, 1));
            senders.push(running(this, 2));
            this.queued_messages.insert(
                1,
                crate::store::QueuedMessage {
                    text: "later".into(),
                    references: vec![],
                    paused: None,
                },
            );
            this.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        })
    });
    draw(cx);
    cx.simulate_keystrokes("ctrl-shift-x");
    draw(cx);
    cx.update(|_, cx| {
        let this = app.read(cx);
        assert!(this.session_control.runs[&1].runtime.is_cancelled());
        assert!(!this.session_control.runs[&2].runtime.is_cancelled());
        assert_eq!(this.queued_messages[&1].text, "later");
        assert!(this.queued_messages[&1].paused.is_some());
    });
    assert!(cx.debug_bounds("stop").is_some());
    assert!(cx.debug_bounds("send").is_none());
}

#[gpui_kit::test]
fn composer_actions_remain_inside_short_and_narrow_viewports(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "composer-layout");
    let mut sender = None;
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().model =
                Some("a-very-long-model-name-for-layout-validation".into());
            app.input
                .update(cx, |input, cx| input.set_value("下一条消息", window, cx));
            cx.notify();
        })
    });
    for (width, height) in [(850., 500.), (1100., 700.), (1200., 800.), (1600., 1000.)] {
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)));
        let mut action_bounds = None;
        for (active, stopping) in [(false, false), (true, false), (true, true)] {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    if !active {
                        app.session_control.runs.remove(&1);
                    } else if stopping {
                        app.session_control.runs.get_mut(&1).unwrap().cancel();
                    } else {
                        sender = Some(running(app, 1));
                    }
                    cx.notify();
                })
            });
            draw(cx);
            let composer = cx.debug_bounds("composer").unwrap();
            let controls = cx.debug_bounds("composer-actions").unwrap();
            let (id, hidden) = if active {
                ("stop", "send")
            } else {
                ("send", "stop")
            };
            assert!(cx.debug_bounds(hidden).is_none());
            let action = cx.debug_bounds(id).unwrap();
            assert!(
                action.left() >= composer.left() && action.right() <= composer.right(),
                "{id}: {action:?} vs {composer:?}"
            );
            assert!(
                action.top() >= controls.top()
                    && action.bottom() <= controls.bottom()
                    && action.bottom() <= gpui_kit::px(height),
                "{id}: {action:?}"
            );
            assert_eq!(action.size.width, gpui_kit::px(30.));
            assert_eq!(action.size.height, gpui_kit::px(30.));
            if let Some(previous) = action_bounds {
                assert_eq!(
                    action, previous,
                    "send and stop share a stable toolbar slot"
                );
            }
            action_bounds = Some(action);
        }
    }
}

fn settle_saves(
    app: &mut ConduitApp,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<ConduitApp>,
) {
    app.persistence.flush();
    assert!(
        app.persistence.error.is_none(),
        "queue save failed: {:?}",
        app.persistence.error
    );
    app.poll_pending_sends(window, cx);
    app.persistence.flush();
}

#[cfg(windows)]
#[gpui_kit::test]
fn queue_message_save_isolated_from_another_test_writer(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    // A previous test's detached writer can outlive its test thread, even when
    // the harness uses --test-threads=1. Keep its temporary file open throughout
    // this send so a shared data directory fails deterministically on Windows.
    let _other_writer = std::thread::spawn(|| {
        use std::os::windows::fs::OpenOptionsExt;

        let path = Store::path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(path.with_extension("json.tmp"))
            .unwrap()
    })
    .join()
    .unwrap();
    let (app, cx, _) = fixture(cx, "isolated-save");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let _sender = running(this, 1);
            this.input.update(cx, |input, cx| {
                input.set_value("isolated follow-up", window, cx)
            });
            this.send_prompt(window, cx);
            settle_saves(this, window, cx);
            assert_eq!(this.queued_messages[&1].text, "isolated follow-up");
            assert_eq!(
                Store::load().unwrap().queued_messages[&1].text,
                "isolated follow-up"
            );
            assert!(this.input.read(cx).value().is_empty());
        })
    });
}

#[gpui_kit::test]
fn queue_message_ui_capacity_and_takeback_preserve_drafts(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "capacity");
    let file = root.join("reference.txt");
    std::fs::write(&file, "reference").unwrap();
    let sender = cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let sender = running(this, 1);
            this.attach_paths(std::slice::from_ref(&file), window, cx);
            this.input
                .update(cx, |input, cx| input.set_value("第二条消息", window, cx));
            this.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
            sender
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("queue-message").is_none());
    assert!(cx.debug_bounds("stop").is_some());
    cx.simulate_keystrokes("enter");
    cx.update(|window, cx| app.update(cx, |this, cx| settle_saves(this, window, cx)));
    draw(cx);
    let bubble = cx.debug_bounds("queued-message").unwrap();
    let composer = cx.debug_bounds("composer").unwrap();
    assert!(bubble.bottom() <= composer.top());
    assert!(bubble.size.height <= gpui_kit::px(40.));
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert_eq!(this.queued_messages[&1].text, "第二条消息");
            assert_eq!(this.queued_messages[&1].references.len(), 1);
            assert!(this.input.read(cx).value().is_empty());
            assert!(
                this.session(1).unwrap().messages.is_empty(),
                "queued text is not sent history"
            );
            assert_eq!(
                Store::load().unwrap().queued_messages[&1].text,
                "第二条消息"
            );
            this.input
                .update(cx, |input, cx| input.set_value("第三条草稿", window, cx));
            this.input.read(cx).focus_handle(cx).focus(window, cx);
        })
    });
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert_eq!(this.queued_messages.len(), 1);
            assert_eq!(this.input.read(cx).value().as_ref(), "第三条草稿");
            this.take_back_queued_message(1, window, cx);
            assert_eq!(
                this.input.read(cx).value().as_ref(),
                "第三条草稿",
                "takeback cannot overwrite drafts"
            );
            assert!(this.queued_messages.contains_key(&1));
            this.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            this.take_back_queued_message(1, window, cx);
            assert_eq!(this.input.read(cx).value().as_ref(), "第二条消息");
            assert_eq!(
                this.current_files().unwrap().references[0].path,
                file.canonicalize().unwrap()
            );
            assert!(this.queued_messages.is_empty());
            this.persistence.flush();
            assert!(Store::load().unwrap().queued_messages.is_empty());
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("queued-message").is_none());
    drop(sender);
}

#[gpui_kit::test]
fn queue_message_background_completion_uses_bound_session_and_keeps_focus(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "background");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let sender = running(this, 1);
            this.input
                .update(cx, |input, cx| input.set_value("follow-up", window, cx));
            this.send_prompt(window, cx);
            settle_saves(this, window, cx);
            assert!(this.queued_messages.contains_key(&1));
            this.selected_workspace = Some(1);
            this.selected_session = Some(2);
            this.new_session_provider = Provider::Codex;
            this.input
                .update(cx, |input, cx| input.set_value("other draft", window, cx));
            this.drafts.insert(2, "other draft".into());
            this.message = Some("other banner".into());
            this.input.read(cx).focus_handle(cx).focus(window, cx);
            sender
                .send(Event::NativeId("latest-native-id".into()))
                .unwrap();
            sender
                .send(Event::Delta {
                    key: "reply".into(),
                    text: "first answer".into(),
                })
                .unwrap();
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            this.poll(window, cx);
            assert_eq!(this.selected_workspace, Some(1));
            assert_eq!(this.selected_session, Some(2));
            assert_eq!(this.new_session_provider, Provider::Codex);
            assert_eq!(this.input.read(cx).value().as_ref(), "other draft");
            assert_eq!(this.drafts[&2], "other draft");
            assert!(this.input.read(cx).focus_handle(cx).is_focused(window));
            assert_eq!(this.message.as_deref(), Some("other banner"));
            assert!(this.session(2).unwrap().messages.is_empty());
            let messages = &this.session(1).unwrap().messages;
            assert_eq!(messages.len(), 2);
            assert_eq!(messages[0].kind, ItemKind::Assistant);
            assert_eq!(messages[1].kind, ItemKind::User);
            assert_eq!(messages[1].text, "follow-up");
            assert_eq!(
                this.session(1).unwrap().native_id.as_deref(),
                Some("latest-native-id")
            );
            assert!(this.queued_messages.is_empty());
            this.persistence.flush();
            assert!(Store::load().unwrap().queued_messages.is_empty());
            assert!(this.session_control.runs.contains_key(&1));
            this.session_control.runs.remove(&1).unwrap().cancel();
        })
    });
}

#[gpui_kit::test]
fn queue_message_failure_cancel_and_manual_send(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "pause");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            for (error, cancelled) in [(Some("offline error".into()), false), (None, true)] {
                let sender = running(this, 1);
                this.input
                    .update(cx, |input, cx| input.set_value("retained", window, cx));
                this.send_prompt(window, cx);
                settle_saves(this, window, cx);
                sender.send(Event::Finished { error, cancelled }).unwrap();
                this.poll(window, cx);
                assert_eq!(this.queued_messages[&1].text, "retained");
                assert!(this.queued_messages[&1].paused.is_some());
                assert!(!this.session_control.runs.contains_key(&1));
                this.dispatch_queued_message(1, cx);
                assert!(
                    !this.session_control.runs.contains_key(&1),
                    "paused queue cannot automatically resume"
                );
                this.queued_messages.remove(&1);
                this.persist();
                this.persistence.flush();
            }
            let sender = running(this, 1);
            this.input.update(cx, |input, cx| {
                input.set_value("manual follow-up", window, cx)
            });
            this.send_prompt(window, cx);
            settle_saves(this, window, cx);
            this.pause_queued_message(1, "stopped");
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: true,
                })
                .unwrap();
            this.poll(window, cx);
            this.input
                .update(cx, |input, cx| input.set_value("keep my draft", window, cx));
            this.send_queued_message(1, cx);
            assert!(this.queued_messages.is_empty());
            assert!(this.session_control.runs.contains_key(&1));
            assert_eq!(this.input.read(cx).value().as_ref(), "keep my draft");
            this.session_control.runs.remove(&1).unwrap().cancel();
            this.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn queue_message_invalid_attachment_launch_failure_and_close_keep_content(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "invalid");
    let file = root.join("vanishing.txt");
    std::fs::write(&file, "temporary").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let sender = running(this, 1);
            this.attach_paths(std::slice::from_ref(&file), window, cx);
            this.input.update(cx, |input, cx| {
                input.set_value("with attachment", window, cx)
            });
            this.send_prompt(window, cx);
            settle_saves(this, window, cx);
            std::fs::remove_file(&file).unwrap();
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            this.poll(window, cx);
            assert!(
                this.queued_messages[&1]
                    .paused
                    .as_deref()
                    .unwrap()
                    .contains("附件")
            );
            assert_eq!(this.queued_messages[&1].references.len(), 1);
            assert!(this.session(1).unwrap().messages.is_empty());
            this.queued_messages.get_mut(&1).unwrap().references.clear();
            this.store.cli.claude_path = Some(root.join("missing.exe"));
            this.send_queued_message(1, cx);
            assert!(this.queued_messages[&1].paused.is_some());
            assert_eq!(this.queued_messages[&1].text, "with attachment");
            assert!(this.session(1).unwrap().messages.is_empty());
            this.queued_messages.get_mut(&1).unwrap().paused = None;
            this.closing = true;
            this.dispatch_queued_message(1, cx);
            assert!(this.queued_messages.contains_key(&1));
            assert!(this.session_control.runs.is_empty());
            this.closing = false;
            this.delete_session(1, cx);
            assert!(!this.queued_messages.contains_key(&1));
            this.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn queue_message_reopen_restores_paused_and_never_auto_sends(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "restore");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let _sender = running(this, 1);
            this.input.update(cx, |input, cx| {
                input.set_value("survive restart", window, cx)
            });
            this.send_prompt(window, cx);
            settle_saves(this, window, cx);
            let loaded = Store::load().unwrap();
            let restored =
                cx.new(|cx| ConduitApp::with_store(window, cx, Ok(loaded), false, false));
            restored.update(cx, |restored, cx| {
                assert_eq!(restored.queued_messages[&1].text, "survive restart");
                assert!(restored.queued_messages[&1].paused.is_some());
                restored.dispatch_queued_message(1, cx);
                assert!(restored.session_control.runs.is_empty());
                restored.remove_workspace(0, cx);
                assert!(restored.queued_messages.is_empty());
                restored.persistence.flush();
            });
        })
    });
}
