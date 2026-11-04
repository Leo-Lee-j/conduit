use super::{Level, Lifecycle, Notice, Source};
use crate::app::{ActiveRun, ChatRuntime, ConduitApp, Event, Store, Workspace, unix_now};
use gpui_kit::component::{Root, WindowExt as _};
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, px, size,
};
use std::time::Duration;

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
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    draw(cx);
    let at = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"))
        .center();
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn notification_bell_and_panel_work_without_a_session_in_both_layouts(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, false);
    for (width, branches) in [(1280., false), (850., true)] {
        cx.simulate_resize(size(px(width), px(800.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                if branches {
                    app.show_branch_navigation(window, cx);
                } else {
                    app.leave_branch_navigation();
                }
                app.detail_text = Some("retained-tool".into());
                cx.notify();
            })
        });
        click(cx, "notification-bell");
        let bounds = cx
            .debug_bounds("notification-panel")
            .expect("history panel visible");
        assert!(bounds.size.width >= px(250.) && bounds.right() <= px(width));
        assert!(cx.debug_bounds("notification-list").is_some());
        cx.update(|_, cx| {
            assert!(app.read(cx).details);
            assert_eq!(app.read(cx).detail_text.as_deref(), Some("retained-tool"));
        });
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(cx.debug_bounds("notification-panel").is_none());
        assert!(cx.debug_bounds("notification-bell").is_some());
    }
}

#[gpui_kit::test]
fn toast_expiry_preserves_history_unread_and_input_focus(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, true);
    cx.simulate_resize(size(px(1280.), px(900.)));
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let focus = app.input.read(cx).focus_handle(cx);
            focus.focus(window, cx);
            for index in 0..6 {
                app.notify_user(
                    Notice::new(
                        Source::App,
                        Level::Info,
                        format!("通知 {index}"),
                        "后台操作完成",
                    ),
                    cx,
                );
            }
            app.poll_notifications(window, cx);
            assert_eq!(window.focused(cx).as_ref(), Some(&focus));
            assert_eq!(app.notifications.center.unread(), 6);
            assert!(window.notifications(cx).len() <= 3);
        })
    });
    draw(cx);
    for _ in 0..9 {
        cx.executor().advance_clock(Duration::from_secs(1));
        draw(cx);
    }
    cx.update(|window, cx| {
        assert!(
            window.notifications(cx).is_empty(),
            "all transient toasts expire"
        );
        assert_eq!(app.read(cx).notifications.center.records().len(), 6);
        assert_eq!(app.read(cx).notifications.center.unread(), 6);
    });
    click(cx, "notification-bell");
    click(cx, "notifications-read-all");
    cx.update(|_, cx| assert_eq!(app.read(cx).notifications.center.unread(), 0));
    click(cx, "notifications-clear-read");
    cx.update(|_, cx| assert!(app.read(cx).notifications.center.records().is_empty()));
}

#[gpui_kit::test]
fn notification_navigation_uses_bound_target_and_handles_deletion(cx: &mut TestAppContext) {
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
            app.open_notification(id, window, cx);
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
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
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
            assert_eq!(app.selected_session, Some(1));
            let records = app.notifications.center.records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].source, Source::Session);
            assert_eq!(records[0].level, Level::Success);
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
}

#[gpui_kit::test]
fn modal_notifications_preserve_dialog_and_late_toast_close_cannot_remove_replacement(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx, false);
    cx.simulate_resize(size(px(1280.), px(900.)));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let id = app.notify_user(
                Notice::new(Source::App, Level::Info, "操作", "第一阶段")
                    .key("same-operation", "first"),
                cx,
            );
            app.poll_notifications(window, cx);
            assert!(window.has_active_dialog(cx));
            id
        })
    });
    draw(cx);
    cx.executor().advance_clock(Duration::from_secs(3));
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let next = app.notify_user(
                Notice::new(Source::App, Level::Success, "操作", "已完成")
                    .key("same-operation", "second"),
                cx,
            );
            assert_eq!(next, id);
            app.poll_notifications(window, cx);
            assert!(window.has_active_dialog(cx));
            assert_eq!(app.notifications.center.records().len(), 1);
            assert!(app.notifications.visible.contains_key(&id));
        })
    });
    cx.executor().advance_clock(Duration::from_secs(3));
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.notifications.visible.contains_key(&id));
            assert_eq!(window.notifications(cx).len(), 1);
            app.notifications.center.mark_all_read(unix_now());
            app.dismiss_read_toasts(window, cx);
            app.notify_user(
                Notice::new(Source::App, Level::Info, "后来到达", "应保持未读")
                    .presentation(super::Presentation::History),
                cx,
            );
            assert_eq!(app.notifications.center.unread(), 1);
            assert!(window.has_active_dialog(cx));
        })
    });
}

#[gpui_kit::test]
fn persistent_errors_are_recorded_once_and_reading_does_not_dismiss_the_banner(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx, false);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence.error = Some("disk full".into());
            app.message = Some("保存失败：disk full".into());
            app.poll_notifications(window, cx);
            app.poll_notifications(window, cx);
            assert_eq!(app.notifications.center.records().len(), 1);
            assert_eq!(app.notifications.banners.len(), 1);
            app.notifications.center.mark_all_read(unix_now());
            app.notifications.center.clear_read();
            assert_eq!(app.notifications.banners.len(), 1);
            assert_eq!(
                app.notifications.center.records()[0].lifecycle,
                Lifecycle::Active
            );
            app.persistence.error = None;
            app.poll_notifications(window, cx);
            assert!(app.notifications.banners.is_empty());
            assert!(app.message.is_none());
            assert_eq!(
                app.notifications.center.records()[0].lifecycle,
                Lifecycle::Resolved
            );
            app.persistence.error = Some("disk full".into());
            app.poll_notifications(window, cx);
            assert_eq!(app.notifications.center.unread(), 1);
            assert_eq!(app.notifications.banners.len(), 1);
            app.persistence.error = None;
        })
    });
}
