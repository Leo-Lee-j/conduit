use super::{Level, Lifecycle, Notice, Source};
use crate::app::{ActiveRun, ChatRuntime, ConduitApp, Event, Store, Workspace, unix_now};
use gpui_kit::component::Root;
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext,
    base::test_support, px, size,
};

mod approvals;

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
