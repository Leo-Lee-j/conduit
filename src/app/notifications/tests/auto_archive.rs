use super::{draw, fixture};
use crate::{
    app::{ActiveRun, ChatRuntime, DocumentTab, unix_now},
    domain::SessionRecord,
    notifications::{Level, Source, Target},
    store::{AutoArchiveDays, AutoArchiveSettings, QueuedMessage},
};
use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext};

fn toggle_setting(cx: &mut VisualTestContext) {
    use gpui_kit::test::TestWindowExt as _;
    let at = cx.update(|window, _| {
        window
            .find("session-auto-archive-enabled")
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn old_session(id: u64, age_days: u64) -> SessionRecord {
    let mut session = crate::app::tests::session();
    session.id = id;
    session.created_at = unix_now() - age_days * 24 * 60 * 60;
    session.title = format!("会话{id}");
    session
}

#[gpui_kit::test]
fn auto_archive_settings_are_searchable_and_allow_configuration_before_enabling(
    cx: &mut TestAppContext,
) {
    use gpui_kit::test::TestWindowExt as _;
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, false);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            for query in ["自动归档", "automatic archive"] {
                assert!(
                    app.settings_search_matches(query)
                        .iter()
                        .any(|result| { result.page == 10 && result.group == Some(5) })
                );
            }
            app.open_settings(window, cx);
            app.settings_ui.page = 10;
            app.settings_ui.group = Some(5);
        });
    });
    draw(cx);
    cx.update(|window, _| {
        assert_ne!(
            window.find("session-auto-archive-enabled").disabled(),
            Some(true)
        );
        assert_ne!(
            window.find("session-auto-archive-days").disabled(),
            Some(true)
        );
    });
    super::click(cx, "session-auto-archive-days");
    let at = cx.update(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .iter()
            .find(|element| {
                element.visible()
                    && element.role() == Some(gpui_kit::Role::MenuItem)
                    && element.label().is_some_and(|label| label.contains("30 天"))
            })
            .expect("30-day archive option")
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.store.ui.auto_archive.enabled);
        assert_eq!(app.store.ui.auto_archive.after, AutoArchiveDays::Thirty);
        assert!(app.notifications.center.records().is_empty());
    });
    toggle_setting(cx);
    cx.update(|window, cx| {
        assert!(app.read(cx).store.ui.auto_archive.enabled);
        assert_ne!(
            window.find("session-auto-archive-days").disabled(),
            Some(true)
        );
    });
    toggle_setting(cx);
    cx.update(|window, cx| {
        assert!(!app.read(cx).store.ui.auto_archive.enabled);
        assert_ne!(
            window.find("session-auto-archive-days").disabled(),
            Some(true)
        );
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        });
    });
    draw(cx);
    cx.update(|window, _| {
        assert_eq!(
            window.find("session-auto-archive-enabled").disabled(),
            Some(true)
        );
        assert_eq!(
            window.find("session-auto-archive-days").disabled(),
            Some(true)
        );
    });
}

#[gpui_kit::test]
fn auto_archive_groups_notices_and_preserves_title_snapshots(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.workspaces[0].sessions = (10..17).map(|id| old_session(id, 10)).collect();
            app.store.workspaces[1].sessions = vec![old_session(20, 10)];
            app.selected_session = Some(10);
            app.open_tabs = vec![DocumentTab::Chat(10), DocumentTab::Chat(20)];
            app.set_auto_archive_enabled(true, cx);
            assert!(
                app.store
                    .workspaces
                    .iter()
                    .flat_map(|w| &w.sessions)
                    .all(|s| s.archived)
            );
            assert!(app.open_tabs.is_empty());
            assert!(app.selected_session.is_none());
            let records = app.notifications.center.records();
            assert_eq!(records.len(), 2);
            let notice = records
                .iter()
                .find(|record| {
                    record.target
                        == Some(Target::ArchivedSessions(
                            app.store.workspaces[0].path.clone(),
                        ))
                })
                .unwrap();
            assert_eq!(notice.source, Source::App);
            assert_eq!(notice.level, Level::Success);
            let message = notice.display_message();
            assert!(message.contains(&app.store.workspaces[0].name()));
            assert!(message.contains("7 个"));
            assert!(message.contains("…（另 2 个会话）"));
            for id in 10..15 {
                assert!(message.lines().any(|line| line == format!("会话{id}")));
            }
            assert!(!message.contains("会话15"));
            let notice_id = notice.id;
            app.session_mut(10).unwrap().title = "后来改的标题".into();
            assert_eq!(
                app.notifications
                    .center
                    .get(notice_id)
                    .unwrap()
                    .display_message(),
                message
            );
            app.auto_archive_sessions(false, cx);
            assert_eq!(app.notifications.center.records().len(), 2);
            // Restore gets a full new grace period, including across restarts.
            app.toggle_archive(10, cx);
            app.store.ui.auto_archive =
                serde_json::from_value(serde_json::to_value(&app.store.ui.auto_archive).unwrap())
                    .unwrap();
            app.auto_archive_sessions(true, cx);
            assert!(!app.session(10).unwrap().archived);
            assert_eq!(app.notifications.center.records().len(), 2);
            // Viewing the notice reveals archived sessions in the bound workspace.
            app.selected_workspace = Some(1);
            app.open_settings(window, cx);
            app.open_notification(notice_id, window, cx);
            assert!(!app.settings_ui.open);
            assert!(app.show_archived);
            assert_eq!(app.selected_workspace, Some(0));
            assert!(
                !app.collapsed_workspaces
                    .contains(&app.store.workspaces[0].path)
            );
        });
    });
    draw(cx);
}

#[gpui_kit::test]
fn auto_archive_checks_once_per_day_and_setting_changes_check_immediately(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            for workspace in &mut app.store.workspaces {
                workspace.sessions.clear();
            }
            app.set_auto_archive_enabled(true, cx);
            let checked = app.store.ui.auto_archive.last_check_day.unwrap();
            assert!(app.notifications.center.records().is_empty());
            app.store.workspaces[0].sessions.push(old_session(3, 10));
            // Simulate loading persisted preferences on another launch the same day.
            let saved = serde_json::to_string(&app.store.ui).unwrap();
            app.store.ui = serde_json::from_str(&saved).unwrap();
            app.auto_archive_sessions(false, cx);
            assert!(!app.session(3).unwrap().archived);
            app.store.ui.auto_archive.last_check_day = Some(checked - 1);
            app.auto_archive_sessions(false, cx);
            assert!(app.session(3).unwrap().archived);
            assert_eq!(app.notifications.center.records().len(), 1);
            app.store.workspaces[0].sessions.push(old_session(4, 5));
            app.set_auto_archive_days(AutoArchiveDays::Three, cx);
            assert!(app.session(4).unwrap().archived);
            assert_eq!(app.notifications.center.records().len(), 2);
            app.set_auto_archive_enabled(false, cx);
            app.store.workspaces[0].sessions.push(old_session(5, 10));
            app.auto_archive_sessions(true, cx);
            assert!(!app.session(5).unwrap().archived);
            app.set_auto_archive_enabled(true, cx);
            assert!(app.session(5).unwrap().archived);
            assert_eq!(app.notifications.center.records().len(), 3);
        });
    });
}

#[gpui_kit::test]
fn auto_archive_keeps_active_work_drafts_pins_and_queued_messages(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.workspaces[0].sessions = (1..=6).map(|id| old_session(id, 10)).collect();
            app.store.workspaces[1].sessions.clear();
            app.selected_session = Some(1);
            app.selected_workspace = Some(0);
            app.input
                .update(cx, |input, cx| input.set_value("未发送内容", window, cx));
            app.session_mut(2).unwrap().pinned = true;
            app.drafts.insert(4, "另一个草稿".into());
            app.queued_messages.insert(
                5,
                QueuedMessage {
                    text: "队列消息".into(),
                    references: vec![],
                    paused: None,
                },
            );
            let (runtime, _sender) = ChatRuntime::test_channel();
            app.session_control.runs.insert(
                3,
                ActiveRun::new(
                    runtime,
                    app.store.workspaces[0].path.clone(),
                    "active".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app.set_auto_archive_enabled(true, cx);
            for id in 1..=5 {
                assert!(!app.session(id).unwrap().archived, "protected session {id}");
            }
            assert!(app.session(6).unwrap().archived);
            assert_eq!(app.input.read(cx).value(), "未发送内容");
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.notifications.center.records().len(), 1);
            // Read-only and closing instances cannot change policy or archive data.
            app.writable = false;
            app.set_auto_archive_days(AutoArchiveDays::Thirty, cx);
            assert_eq!(app.store.ui.auto_archive.after, AutoArchiveDays::Seven);
            app.writable = true;
            app.closing = true;
            app.store.ui.auto_archive = AutoArchiveSettings::default();
            app.set_auto_archive_enabled(true, cx);
            assert!(!app.store.ui.auto_archive.enabled);
        });
    });
}
