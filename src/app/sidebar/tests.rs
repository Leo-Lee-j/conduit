//! Real GPUI frames exercise the viewport, cache invalidation and scroll anchors.
use super::cache;
use crate::app::{ChatItem, ConduitApp, Event, Store, Workspace};
use gpui_kit::{AppContext as _, Entity, ScrollStrategy, TestAppContext, VisualTestContext};
use std::rc::Rc;

fn fixture(cx: &mut TestAppContext, count: u64) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut sessions: Vec<_> = (1..=count)
                .map(|id| {
                    let mut session = crate::app::tests::session();
                    session.id = id;
                    session.created_at = id;
                    session.title = format!("needle session {id}");
                    session.messages.clear();
                    session
                })
                .collect();
            sessions.last_mut().unwrap().messages = vec![
                item("turn-live-user", "User", "question", count),
                item("turn-live-reply", "Assistant", "partial", count),
            ];
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions,
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.selected_session = Some(count);
            app.expanded_workspaces.insert(0);
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    (app, cx)
}
fn item(key: &str, kind: &str, text: &str, timestamp: u64) -> ChatItem {
    serde_json::from_value(serde_json::json!({
        "key": key, "kind": kind, "text": text, "created_at": timestamp
    }))
    .unwrap()
}
fn draw(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        let view = app.read(cx).sidebar_view.clone();
        view.update(cx, |_, cx| cx.notify());
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn ten_thousand_sessions_only_construct_viewport_rows(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, 10_000);
    cx.update(|_, cx| app.read(cx).sidebar.rendered_rows.set(0));
    draw(&app, cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.sidebar.rows.len(), 10_003);
        let rendered = app.sidebar.rendered_rows.get();
        assert!(
            rendered > 0 && rendered < 100,
            "constructed {rendered} rows"
        );
    });
    assert!(cx.debug_bounds("session-row-10000").is_some());
    assert!(cx.debug_bounds("session-row-1").is_none());
    cx.update(|_, cx| {
        let app = app.read(cx);
        app.sidebar.rendered_rows.set(0);
        app.sidebar.scroll.scroll_to_item(5002, ScrollStrategy::Top);
    });
    draw(&app, cx);
    assert!(cx.debug_bounds("session-row-5000").is_some());
    assert!(cx.debug_bounds("session-row-10000").is_none());
    cx.update(|_, cx| assert!(app.read(cx).sidebar.rendered_rows.get() < 100));
}

#[gpui_kit::test]
fn streaming_text_reuses_sidebar_and_order_but_title_and_archive_refresh(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, 1000);
    let (renders, sorts, rebuilds, rows, sizes) = cx.update(|_, cx| {
        let app = app.read(cx);
        (
            app.sidebar_view.read(cx).render_count,
            app.sidebar.sort_count,
            app.sidebar.rebuild_count,
            app.sidebar.rows.clone(),
            app.sidebar.sizes.clone(),
        )
    });
    for _ in 0..5 {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                crate::app::apply_event(
                    app.session_mut(1000).unwrap(),
                    "turn-live",
                    Event::Delta {
                        key: "reply".into(),
                        text: " more text".into(),
                    },
                );
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.sidebar_view.read(cx).render_count, renders);
        assert_eq!(app.sidebar.sort_count, sorts);
        assert_eq!(app.sidebar.rebuild_count, rebuilds);
        assert!(Rc::ptr_eq(&rows, &app.sidebar.rows));
        assert!(Rc::ptr_eq(&sizes, &app.sidebar.sizes));
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1000).unwrap().title = "renamed".into();
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        let app = app.read(cx);
        assert!(app.sidebar_view.read(cx).render_count > renders);
        assert_eq!(app.sidebar.sort_count, sorts);
        assert!(Rc::ptr_eq(&rows, &app.sidebar.rows));
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.toggle_archive(1000, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-row-1000").is_none());
    assert!(cx.debug_bounds("session-row-999").is_some());
}

#[gpui_kit::test]
fn reordering_activity_above_viewport_preserves_top_session_and_offset(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, 100);
    cx.update(|_, cx| {
        let app = app.read(cx);
        let index = app
            .sidebar
            .rows
            .iter()
            .position(|row| matches!(row, cache::Row::Session { id: 60, .. }))
            .unwrap();
        app.sidebar
            .scroll
            .scroll_to_item(index, ScrollStrategy::Top);
    });
    draw(&app, cx);
    let before = cx.debug_bounds("session-row-60").unwrap().origin.y;
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(2)
                .unwrap()
                .messages
                .push(item("new", "User", "new activity", 1000));
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let after = cx.debug_bounds("session-row-60").unwrap().origin.y;
    assert_eq!(
        after, before,
        "a newly active session above the viewport must not move the reader"
    );
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(matches!(
            app.sidebar.rows[2],
            cache::Row::Session { id: 2, .. }
        ));
    });
}

#[gpui_kit::test]
fn sidebar_search_includes_matches_beyond_popup_limit(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, 100);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.search_query = "needle".into();
            app.refresh_search(cx, false);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        let app = app.read(cx);
        assert_eq!(app.search_ui.hits.len(), 40);
        assert_eq!(app.search_ui.matches.len(), 100);
        assert_eq!(
            app.sidebar
                .rows
                .iter()
                .filter(|row| matches!(row, cache::Row::Session { .. }))
                .count(),
            100
        );
        assert!(!app.search_ui.hits.iter().any(|hit| hit.session == 1));
        let index = app
            .sidebar
            .rows
            .iter()
            .position(|row| matches!(row, cache::Row::Session { id: 1, .. }))
            .unwrap();
        app.sidebar
            .scroll
            .scroll_to_item(index, ScrollStrategy::Top);
    });
    draw(&app, cx);
    assert!(cx.debug_bounds("session-row-1").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.search_query = "no matches here".into();
            app.refresh_search(cx, true);
            cx.notify();
        })
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("session-row-100").is_none(),
        "don't display results for the previous query"
    );
}

#[gpui_kit::test]
fn wheel_scroll_more_collapse_and_menu_survive_virtualization(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, point, px};
    let (app, cx) = fixture(cx, 100);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.expanded_workspaces.clear();
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-row-96").is_some());
    assert!(cx.debug_bounds("session-row-95").is_none());
    let more = cx.debug_bounds("workspace-more-0").unwrap().center();
    cx.simulate_mouse_down(more, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(more, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-row-95").is_some());
    let row = cx.debug_bounds("session-row-99").unwrap().center();
    let before = cx.update(|_, cx| {
        let app = app.read(cx);
        (
            app.sidebar.sort_count,
            app.sidebar.rebuild_count,
            app.sidebar.rows.clone(),
        )
    });
    cx.simulate_event(ScrollWheelEvent {
        position: row,
        delta: ScrollDelta::Pixels(point(px(0.), px(-340.))),
        ..Default::default()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.sidebar.scroll.offset().y < px(-300.));
        assert_eq!(app.sidebar.sort_count, before.0);
        assert_eq!(app.sidebar.rebuild_count, before.1);
        assert!(Rc::ptr_eq(&app.sidebar.rows, &before.2));
    });
    assert!(cx.debug_bounds("session-row-99").is_none());
    cx.update(|_, cx| {
        app.read(cx)
            .sidebar
            .scroll
            .scroll_to_item(0, ScrollStrategy::Top)
    });
    draw(&app, cx);
    let toggle = cx.debug_bounds("workspace-collapse-0").unwrap().center();
    cx.simulate_mouse_down(toggle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(toggle, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-row-100").is_none());
    cx.simulate_mouse_down(toggle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(toggle, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-row-100").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.request_delete_session(100, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("delete-session-confirmation").is_some());
    let row = cx.debug_bounds("session-row-99").unwrap().center();
    cx.simulate_event(ScrollWheelEvent {
        position: row,
        delta: ScrollDelta::Pixels(point(px(0.), px(-340.))),
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        assert!(app.read(cx).delete_target.is_none());
    });
}

#[gpui_kit::test]
fn running_priority_and_same_count_import_refresh_order(cx: &mut TestAppContext) {
    use crate::app::{ActiveRun, ChatRuntime};
    let (app, cx) = fixture(cx, 100);
    let sender = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let (runtime, sender) = ChatRuntime::test_channel();
            let path = app.store.workspaces[0].path.clone();
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    path,
                    "test".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            cx.notify();
            sender
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        assert!(matches!(
            app.read(cx).sidebar.rows[2],
            cache::Row::Session { id: 1, .. }
        ));
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.remove(&1);
            let path = app.store.workspaces[0].path.clone();
            let session = app.store.workspaces[0]
                .sessions
                .iter_mut()
                .find(|s| s.id == 2)
                .unwrap();
            session.created_at = 200;
            app.sidebar.invalidate_workspace(&path);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        assert!(matches!(
            app.read(cx).sidebar.rows[2],
            cache::Row::Session { id: 2, .. }
        ));
    });
    drop(sender);
}
