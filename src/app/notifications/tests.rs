use super::{Level, Lifecycle, Notice, Source};
use crate::app::{ActiveRun, ChatRuntime, ConduitApp, Event, Store, Workspace, unix_now};
use gpui_kit::component::Root;
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, base::test_support, px, size,
};

mod approvals;

#[gpui_kit::test]
fn git_results_wait_for_toast_capacity_and_clear_progress(cx: &mut TestAppContext) {
    use crate::app::GitOperation;
    use crate::git::{OperationProbe, OperationResult};
    use crate::notifications::Target;

    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    for (message, error, disconnect) in [
        ("Merged feature into main", None, false),
        ("Updated branch main", None, false),
        ("Merge incomplete", Some("Merge conflict"), false),
        ("Worker disconnected", None, true),
    ] {
        let (sender, path, blockers) = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.notifications.center.mark_all_read(unix_now());
                app.poll_notifications(window, cx);
                let blockers: Vec<_> = (0..2)
                    .map(|index| {
                        app.notify_user(
                            Notice::new(Source::App, Level::Info, "Other task", index.to_string()),
                            cx,
                        )
                    })
                    .collect();
                app.poll_notifications(window, cx);
                assert_eq!(app.notifications.visible.len(), 2);
                let path = app.store.workspaces[0].path.clone();
                let (sender, events) = std::sync::mpsc::channel();
                let mut probe = OperationProbe::test_pending(path.clone());
                probe.events = events;
                app.git.operation = Some(GitOperation {
                    probe,
                    worktree: path.clone(),
                    draft: String::new(),
                    commit_push: None,
                    automatic_stage: false,
                    incoming_fetch: false,
                });
                // Results retain the operation's workspace after navigation elsewhere.
                app.selected_workspace = Some(1);
                app.selected_session = Some(2);
                app.input
                    .update(cx, |input, cx| input.set_value("keep draft", window, cx));
                cx.notify();
                (sender, path, blockers)
            })
        });
        draw(cx);
        assert!(cx.debug_bounds("git-operation-progress").is_some());
        if !disconnect {
            sender
                .send(Ok(OperationResult {
                    retry_completed: false,
                    retry_tag: None,
                    committed: false,
                    message: message.into(),
                    error: error.map(str::to_owned),
                    worktree_created: None,
                }))
                .unwrap();
        }
        drop(sender);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.poll_git_actions(window, cx);
                assert!(app.git.operation.is_none());
                let record = app
                    .notifications
                    .center
                    .records()
                    .iter()
                    .find(|record| record.source == Source::Git && record.read_at.is_none())
                    .unwrap();
                let id = record.id;
                assert_eq!(record.target, Some(Target::Workspace(path)));
                assert_eq!(
                    record.level,
                    if error.is_some() || disconnect {
                        Level::Error
                    } else {
                        Level::Success
                    }
                );
                if !disconnect {
                    assert!(record.message.contains(message));
                }
                app.poll_notifications(window, cx);
                assert_eq!(app.notifications.pending, vec![id]);
                assert!(!app.notifications.visible.contains_key(&id));
                let count = app.notifications.center.records().len();
                app.poll_git_actions(window, cx);
                app.poll_notifications(window, cx);
                assert_eq!(app.notifications.center.records().len(), count);
                assert_eq!(app.notifications.pending, vec![id]);
                app.notifications.center.mark_read(blockers[0], unix_now());
                app.poll_notifications(window, cx);
                assert!(app.notifications.pending.is_empty());
                assert!(app.notifications.visible.contains_key(&id));
                assert_eq!(app.selected_workspace, Some(1));
                assert_eq!(app.selected_session, Some(2));
                assert_eq!(app.input.read(cx).value(), "keep draft");
            })
        });
        draw(cx);
        assert!(cx.debug_bounds("git-operation-progress").is_none());
    }
}

#[gpui_kit::test]
fn blocked_sends_notify_without_clearing_draft_or_changing_focus(cx: &mut TestAppContext) {
    use crate::app::GitOperation;
    use crate::git::OperationProbe;

    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "blocked-send-notification");
    let attachment = root.join("reference.txt");
    std::fs::write(&attachment, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input.update(cx, |input, cx| {
                input.set_value("review current changes", window, cx)
            });
            app.attach_paths(&[attachment], window, cx);
            app.input.read(cx).focus_handle(cx).focus(window, cx);
            let references = app.current_files().unwrap().references.clone();
            let target = app.session_notice_target(1);
            let (sender, events) = std::sync::mpsc::channel();
            let mut probe = OperationProbe::test_pending(root.clone());
            probe.events = events;
            app.git.operation = Some(GitOperation {
                probe,
                worktree: root,
                draft: String::new(),
                commit_push: None,
                automatic_stage: false,
                incoming_fetch: false,
            });

            for direct in [false, true] {
                if direct {
                    assert!(!app.send_text("another prompt".into(), true, window, cx));
                } else {
                    app.send_prompt(window, cx);
                }
                assert_eq!(app.input.read(cx).value(), "review current changes");
                assert_eq!(app.current_files().unwrap().references, references);
                assert!(app.input.read(cx).focus_handle(cx).is_focused(window));
                assert_eq!(app.selected_workspace, Some(0));
                assert_eq!(app.selected_session, Some(1));
                assert!(app.session_control.runs.is_empty());
                assert!(app.session(1).unwrap().messages.is_empty());
                let notice = &app.notifications.center.records()[0];
                assert_eq!(notice.source, Source::Session);
                assert_eq!(notice.level, Level::Warning);
                assert_eq!(notice.target, target);
                assert_eq!(
                    notice.message,
                    conduit::i18n::text_static("runtime.a_git_operation_is_running_in_this")
                );
                let id = notice.id;
                app.poll_notifications(window, cx);
                assert!(app.notifications.visible.contains_key(&id));
                assert!(app.notifications.banners.is_empty());
            }

            drop(sender);
            app.poll_git_actions(window, cx);
            assert!(app.git.operation.is_none());
            assert!(app.send_blocked(true).is_none());
            assert_eq!(app.input.read(cx).value(), "review current changes");
            assert_eq!(app.current_files().unwrap().references, references);
            app.switch(1, Some(2), window, cx);
            assert!(
                app.notifications
                    .center
                    .records()
                    .iter()
                    .any(|notice| { notice.source == Source::Session && notice.target == target })
            );
        });
    });
}

#[gpui_kit::test]
fn empty_session_cleanup_records_one_notification_per_nonempty_cleanup(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.auto_cleanup_empty_sessions = true;
            let created_at = unix_now().saturating_sub(2 * 24 * 60 * 60);
            for workspace in &mut app.store.workspaces {
                workspace.sessions[0].created_at = created_at;
            }

            app.cleanup_stale_empty_sessions(cx);

            assert!(
                app.store
                    .workspaces
                    .iter()
                    .all(|workspace| workspace.sessions.is_empty())
            );
            let records = app.notifications.center.records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, Source::App);
            assert_eq!(records[0].level, Level::Success);
            assert_eq!(records[0].title, "空会话清理完成");
            assert_eq!(records[0].message, "已删除 2 个空会话。");

            app.cleanup_stale_empty_sessions(cx);
            assert_eq!(app.notifications.center.records().len(), 1);
        });
    });
}

fn fixture(
    cx: &mut TestAppContext,
    sessions: bool,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut store = Store::default();
            store.ui.performance_monitor = false;
            if sessions {
                let first = crate::app::tests::session();
                let mut second = first.clone();
                second.id = 2;
                store.workspaces = vec![
                    Workspace {
                        path: std::env::temp_dir().join("conduit-notification-first"),
                        sessions: vec![first],
                    },
                    Workspace {
                        path: std::env::temp_dir().join("conduit-notification-second"),
                        sessions: vec![second],
                    },
                ];
            }
            ConduitApp::with_store(window, cx, Ok(store), false, false)
        });
        *output.borrow_mut() = Some(app.clone());
        Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    (app, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}
fn click(cx: &mut VisualTestContext, selector: &str) {
    draw(cx);
    let at = cx
        .debug_bounds(selector.to_owned().leak())
        .unwrap_or_else(|| panic!("missing {selector}"))
        .center();
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn notification_navigation_uses_bound_target_and_handles_deletion(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.input
                .update(cx, |input, cx| input.set_value("保留当前草稿", window, cx));
            let target = app.session_notice_target(2).unwrap();
            let id = app.notify_user(
                Notice::new(Source::Session, Level::Success, "会话完成", "另一工作区")
                    .target(target),
                cx,
            );
            assert_eq!(app.selected_session, Some(1));
            app.open_settings(window, cx);
            app.open_notification(id, window, cx);
            assert!(
                !app.settings_ui.open,
                "notification navigation leaves settings"
            );
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(app.selected_workspace, Some(1));
            assert_eq!(app.drafts.get(&1).map(String::as_str), Some("保留当前草稿"));
            let target = app.session_notice_target(1).unwrap();
            let stale = app.notify_user(
                Notice::new(Source::Session, Level::Error, "已删除来源", "旧记录").target(target),
                cx,
            );
            // A reused numeric session ID after restart must not redirect old history.
            app.store.workspaces[0].sessions[0].created_at += 1;
            app.open_notification(stale, window, cx);
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(
                app.notifications.target_error.as_ref().map(|error| error.0),
                Some(stale)
            );
            app.store.workspaces.clear();
            app.selected_workspace = None;
            app.selected_session = None;
            app.open_notification(stale, window, cx);
            assert!(app.store.workspaces.is_empty());
            assert!(app.selected_session.is_none());
            assert_eq!(
                app.notifications.target_error.as_ref().map(|error| error.0),
                Some(stale)
            );
            assert!(
                app.notifications
                    .center
                    .get(stale)
                    .unwrap()
                    .read_at
                    .is_some()
            );
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("notification-panel").is_some());
}

#[gpui_kit::test]
fn background_completion_and_approval_publish_once_without_switching_sessions(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        cx.set_app_identity(
            crate::system_notifications::APP_USER_MODEL_ID,
            crate::system_notifications::APP_NAME,
        );
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(1);
            app.selected_session = Some(2);
            app.window_active = false;
            app.store.ui.system_notifications = true;
            let (runtime, sender) = ChatRuntime::test_channel();
            let path = app.store.workspaces[1].path.clone();
            app.session_control.runs.insert(
                2,
                ActiveRun::new(
                    runtime,
                    path,
                    "notification-turn".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            app.poll_streams(window, cx);
            assert_eq!(app.selected_session, Some(2));
            let records = app.notifications.center.records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, Source::Session);
            assert_eq!(records[0].level, Level::Success);
            assert!(app.session_is_visible(2));
            assert!(!app.session_is_notification_visible(2));
            app.poll_notifications(window, cx);
            assert!(app.notifications.visible.is_empty());
            assert_eq!(app.notifications.system_visible.len(), 1);
            assert!(app.notifications.pending.is_empty());
            app.poll_streams(window, cx);
            assert_eq!(app.notifications.center.records().len(), 1);
            let id = app.notify_user(
                Notice::new(Source::Session, Level::Warning, "等待审批", "请查看会话")
                    .target(app.session_notice_target(2).unwrap())
                    .key("approval/2/test", "waiting")
                    .active(),
                cx,
            );
            app.notifications.center.mark_all_read(unix_now());
            app.notifications.center.clear_read();
            assert_eq!(
                app.notifications.center.get(id).unwrap().lifecycle,
                Lifecycle::Active
            );
            app.notifications.center.resolve_session(2, unix_now());
            app.notifications.center.clear_read();
            assert!(app.notifications.center.records().is_empty());
        })
    });
    // System notification delivery is queued on the test platform executor.
    cx.run_until_parked();
    assert_eq!(cx.shown_system_notifications().len(), 1);
}

#[gpui_kit::test]
fn notification_text_selection_copies_without_changing_draft_or_read_state(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{ClipboardItem, Focusable as _, base::TextSelection, point};

    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    let title = "通知标题";
    let message = "第一行：通知详情\n第二行：错误原因";
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.input
                .update(cx, |input, cx| input.set_value("保留草稿", window, cx));
            let id = app.notify_user(
                Notice::new(Source::App, Level::Error, title, message)
                    .presentation(super::Presentation::History),
                cx,
            );
            app.toggle_notifications(window, cx);
            id
        })
    });
    draw(cx);

    for (kind, expected) in [("title", title), ("message", message)] {
        // The composer may retain focus while a notification is being selected.
        cx.update(|window, cx| {
            app.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.write_to_clipboard(ClipboardItem::new_string("SENTINEL".into()));
        });
        let bounds = cx
            .debug_bounds(format!("notification-{kind}-{id}").leak())
            .unwrap();
        let start = bounds.origin + point(px(1.), px(4.));
        let end = point(bounds.right() - px(1.), bounds.bottom() - px(4.));
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        draw(cx);
        assert_eq!(cx.update(TextSelection::selected_text), expected);
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.update(|window, cx| {
            assert!(app.read(cx).notifications.focus.is_focused(window));
            window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some(expected.to_owned())
        );
        // Selection survives the cached panel's next frame.
        draw(cx);
        assert_eq!(cx.update(TextSelection::selected_text), expected);
    }
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.input.read(cx).value(), "保留草稿");
        assert_eq!(app.selected_session, Some(1));
        assert!(app.notifications.center.get(id).unwrap().read_at.is_none());
    });
}
