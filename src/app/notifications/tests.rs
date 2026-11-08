use super::{Level, Lifecycle, Notice, Source};
use crate::app::{ActiveRun, ChatRuntime, ConduitApp, Event, Store, Workspace, unix_now};
use crate::app::{NoticeLevel, NoticeSource, NoticeTarget};
use gpui_kit::component::{Root, WindowExt as _};
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, base::test_support, point, px, size,
};
use std::time::Duration;

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
fn notification_card_keeps_actions_at_corners_and_reveals_hover_controls(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let id = app.notify_user(
                Notice::new(
                    Source::Session,
                    Level::Success,
                    "检查已经完成，包含较长标题时仍与状态图标垂直居中",
                    "可以返回会话查看结果。",
                )
                .target(app.session_notice_target(1).unwrap())
                .presentation(super::Presentation::History),
                cx,
            );
            app.read_notification(id, window, cx);
            app.toggle_notifications(window, cx);
            id
        })
    });
    for width in [1280., 850.] {
        cx.simulate_resize(size(px(width), px(800.)));
        cx.simulate_mouse_move(point(px(20.), px(20.)), None, Modifiers::default());
        draw(cx);
        let card = cx
            .debug_bounds(format!("notification-record-{id}").leak())
            .unwrap();
        let icon = cx
            .debug_bounds(format!("notification-icon-{id}").leak())
            .unwrap();
        let title = cx
            .debug_bounds(format!("notification-title-{id}").leak())
            .unwrap();
        let footer = cx
            .debug_bounds(format!("notification-footer-{id}").leak())
            .unwrap();
        let time = cx
            .debug_bounds(format!("notification-time-{id}").leak())
            .unwrap();
        let open = cx
            .debug_bounds(format!("notification-open-{id}").leak())
            .unwrap();
        let close = cx
            .debug_bounds(format!("notification-remove-{id}").leak())
            .unwrap();
        assert!((icon.center().y - title.center().y).abs() <= px(0.5));
        assert_eq!(time.left(), footer.left());
        assert_eq!(open.right(), footer.right());
        assert!(close.top() < footer.top() && close.right() == footer.right());
        assert!(footer.bottom() < card.bottom());
        let tabs = cx.debug_bounds("document-tab-strip").unwrap();
        let header = cx.debug_bounds("detail-header").unwrap();
        assert_eq!(header.top(), tabs.top());
        assert_eq!(header.size.height, tabs.size.height);
        cx.update(|window, _| {
            for kind in ["notification-time", "notification-remove"] {
                assert!(
                    !test_support::find(window, &[], &(kind, id).into())
                        .unwrap()
                        .visible()
                );
            }
        });
        cx.simulate_mouse_move(card.center(), None, Modifiers::default());
        draw(cx);
        cx.update(|window, _| {
            for kind in ["notification-time", "notification-remove"] {
                assert!(
                    test_support::find(window, &[], &(kind, id).into())
                        .unwrap()
                        .visible()
                );
            }
        });
        assert_eq!(
            cx.debug_bounds(format!("notification-record-{id}").leak()),
            Some(card)
        );
        assert_eq!(
            cx.debug_bounds(format!("notification-footer-{id}").leak()),
            Some(footer)
        );
    }
    click(cx, &format!("notification-remove-{id}"));
    cx.update(|_, cx| assert!(app.read(cx).notifications.center.get(id).is_none()));
}

#[gpui_kit::test]
fn notification_close_control_preserves_unread_and_active_records(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, false);
    let (unread, active) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let unread = app.notify_user(
                Notice::new(Source::App, Level::Info, "未读通知", "先标为已读再移除")
                    .presentation(super::Presentation::History),
                cx,
            );
            let active = app.notify_user(
                Notice::new(Source::App, Level::Warning, "待处理", "仍需处理")
                    .key("hover-active", "first")
                    .active()
                    .presentation(super::Presentation::History),
                cx,
            );
            app.read_notification(active, window, cx);
            (unread, active)
        })
    });
    click(cx, "toggle-sidebar");
    assert!(cx.debug_bounds("notification-panel").is_some());
    assert!(
        cx.debug_bounds(format!("notification-remove-{unread}").leak())
            .is_none()
    );
    assert!(
        cx.debug_bounds(format!("notification-remove-{active}").leak())
            .is_none()
    );
    click(cx, &format!("notification-read-{unread}"));
    assert!(
        cx.debug_bounds(format!("notification-remove-{unread}").leak())
            .is_some()
    );
    assert!(
        cx.debug_bounds(format!("notification-remove-{active}").leak())
            .is_none()
    );
}

#[gpui_kit::test]
fn notification_bell_and_panel_work_without_a_session_in_both_layouts(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
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
            assert!(
                app.read(cx)
                    .plugins
                    .right_is(conduit::plugins::builtins::NOTIFICATIONS)
            );
        });
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(cx.debug_bounds("notification-panel").is_none());
        cx.update(|_, cx| {
            assert!(!app.read(cx).plugins.right_open());
            assert_eq!(
                app.read(cx).plugins.right().as_str(),
                conduit::plugins::builtins::NOTIFICATIONS
            );
        });
        assert!(cx.debug_bounds("notification-bell").is_some());
    }
}

#[gpui_kit::test]
fn toast_body_click_marks_read_and_opens_bound_session(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    // Slide animations use wall time; the test executor only advances toast timers.
    // Render at the final position so the body click is independent of CPU speed.
    cx.update(|_, cx| cx.set_reduce_motion(true));
    cx.simulate_resize(size(px(1280.), px(900.)));
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.switch(0, Some(1), window, cx);
            let target = app.session_notice_target(2).unwrap();
            let id = app.notify_user(
                Notice::new(
                    Source::Session,
                    Level::Success,
                    "Session complete",
                    "Open session",
                )
                .target(target),
                cx,
            );
            app.poll_notifications(window, cx);
            assert_eq!(window.notifications(cx).len(), 1);
            id
        })
    });
    draw(cx);
    // Exercise the toast's own listener, which updates its Notification entity.
    let at = cx.update(|window, _| {
        let toasts: Vec<_> = test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.visible() && element.role() == Some(gpui_kit::Role::Alert))
            .collect();
        assert_eq!(toasts.len(), 1);
        let bounds = toasts[0].bounds();
        // Stay in the body, away from the close and action buttons.
        point(bounds.left() + px(80.), bounds.top() + px(25.))
    });
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_session, Some(2));
        assert_eq!(app.selected_workspace, Some(1));
        assert!(app.notifications.center.get(id).unwrap().read_at.is_some());
        assert!(!app.notifications.visible.contains_key(&id));
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    cx.update(|window, cx| assert!(window.notifications(cx).is_empty()));
}

#[gpui_kit::test]
fn toast_expiry_preserves_history_unread_and_input_focus(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
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

#[gpui_kit::test]
fn modal_notifications_preserve_dialog_and_late_toast_close_cannot_remove_replacement(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, false);
    cx.simulate_resize(size(px(1280.), px(900.)));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_about_dialog(window, cx)));
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
    let _locale = crate::test_support::chinese();
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

#[gpui_kit::test]
fn git_notices_use_center_with_or_without_git_navigation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut store = Store::default();
            store.ui.performance_monitor = false;
            store.workspaces = vec![Workspace {
                path: std::env::temp_dir().join("conduit-git-notices"),
                sessions: vec![crate::app::tests::session()],
            }];
            ConduitApp::with_store(window, cx, Ok(store), false, false)
        });
        app.update(cx, |app, cx| {
            let path = app.selected_workspace_path().unwrap();
            app.git_result_feedback(path.clone(), "后台推送完成", false, cx);
            assert_eq!(app.notifications.pending.len(), 1);

            app.show_git_navigation(cx);
            assert!(app.git_navigation_open());
            app.git_result_feedback(path.clone(), "提交完成", false, cx);
            assert_eq!(app.notifications.pending.len(), 2);
            let notice = &app.notifications.center.records()[0];
            assert_eq!(notice.level, NoticeLevel::Success);
            assert_eq!(notice.message, "提交完成");
            assert_eq!(notice.target, Some(NoticeTarget::Workspace(path.clone())));

            // Routine progress is recorded without adding a toast.
            app.git_feedback(path.clone(), "正在暂存…", false, cx);
            assert_eq!(app.notifications.pending.len(), 2);
            assert_eq!(
                app.notifications.center.records()[0].level,
                NoticeLevel::Info
            );

            let error = "校验失败的完整说明\n".repeat(30);
            app.git_feedback(path.clone(), error.clone(), true, cx);
            assert_eq!(app.notifications.pending.len(), 3);
            let notice = &app.notifications.center.records()[0];
            assert_eq!(notice.source, NoticeSource::Git);
            assert_eq!(notice.level, NoticeLevel::Error);
            assert_eq!(notice.message, error);

            // A background result keeps its own workspace, not the current selection.
            let other = path.with_file_name("conduit-other-git-notices");
            app.git_result_feedback(other.clone(), "后台推送失败", true, cx);
            assert_eq!(app.notifications.pending.len(), 4);
            assert_eq!(
                app.notifications.center.records()[0].target,
                Some(NoticeTarget::Workspace(other))
            );
            assert_eq!(app.notifications.center.records().len(), 5);
        });
        Root::new(app, window, cx)
    });
    cx.run_until_parked();
}
