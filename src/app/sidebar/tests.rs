//! Real GPUI frames exercise the viewport, cache invalidation and scroll anchors.
use super::cache;
use crate::app::{ChatItem, ConduitApp, Event, Store, Workspace};
use gpui_kit::test::TestWindowExt as _;
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

fn visible_sessions(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> Vec<u64> {
    cx.update(|_, cx| {
        app.read(cx)
            .sidebar
            .rows
            .iter()
            .filter_map(|row| match row {
                cache::Row::Session { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    })
}

#[gpui_kit::test]
fn virtual_sidebar_keyboard_opens_hundredth_session_and_restores_menu_focus(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, 120);
    cx.update(|window, cx| {
        let focus = app.read(cx).sidebar.focus.clone().unwrap();
        focus.focus(window, cx);
    });
    cx.simulate_keystrokes("home");
    for _ in 0..100 {
        cx.simulate_keystrokes("down");
    }
    draw(&app, cx);
    cx.simulate_keystrokes("enter");
    draw(&app, cx);
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_session, Some(21));
        assert!(app.sidebar.focus.as_ref().unwrap().is_focused(window));
    });
    cx.simulate_keystrokes("shift-f10");
    draw(&app, cx);
    cx.update(|_, cx| assert!(app.read(cx).sidebar.menu.is_some()));
    cx.simulate_keystrokes("escape");
    draw(&app, cx);
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert!(app.sidebar.menu.is_none());
        assert!(app.sidebar.focus.as_ref().unwrap().is_focused(window));
    });
    cx.simulate_keystrokes("home left");
    draw(&app, cx);
    assert!(visible_sessions(&app, cx).is_empty());
    cx.simulate_keystrokes("right");
    draw(&app, cx);
    assert_eq!(visible_sessions(&app, cx).len(), 120);
}

#[gpui_kit::test]
fn session_context_menu_pins_and_unpins_without_switching_the_conversation(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use gpui_kit::{Modifiers, MouseButton};
    let (app, cx) = fixture(cx, 3);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    for (label, pinned) in [("置顶会话", true), ("取消置顶", false)] {
        let row = cx.debug_bounds("session-row-1").unwrap().center();
        cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::default());
        cx.run_until_parked();
        draw(&app, cx);
        cx.update(|window, cx| {
            assert_eq!(
                window.within("popup-menu").find(0usize).label(),
                Some(label)
            );
            window.within("popup-menu").click(0usize, cx);
        });
        cx.run_until_parked();
        draw(&app, cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(app.session(1).unwrap().pinned, pinned);
            assert_eq!(app.selected_session, Some(3));
        });
        assert_eq!(
            visible_sessions(&app, cx),
            if pinned { vec![1, 3, 2] } else { vec![3, 2, 1] }
        );
    }
}

#[gpui_kit::test]
fn pinned_sessions_keep_the_ordinary_quota_and_follow_archive_and_search_filters(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, 8);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.expanded_workspaces.clear();
            app.store.ui.sessions_per_workspace = 2;
            app.toggle_pin(1, cx);
            app.toggle_pin(2, cx);
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    assert_eq!(visible_sessions(&app, cx), vec![2, 1, 8, 7]);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.sidebar.rows.iter().any(|row| matches!(
            row,
            cache::Row::More {
                total: 6,
                expanded: false,
                ..
            }
        )));
        let restored: Store =
            serde_json::from_slice(&serde_json::to_vec(&app.store).unwrap()).unwrap();
        assert_eq!(
            restored.workspaces[0]
                .sessions
                .iter()
                .filter(|s| s.pinned)
                .count(),
            2
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(8).unwrap().created_at = 1000;
            app.toggle_archive(2, cx);
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    assert_eq!(visible_sessions(&app, cx), vec![1, 8, 7]);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.show_archived = true;
            app.toggle_pin(1, cx);
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    assert_eq!(visible_sessions(&app, cx), vec![2, 8, 7]);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for id in [1, 3, 4, 5, 6] {
                app.toggle_pin(id, cx);
            }
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    assert_eq!(visible_sessions(&app, cx), vec![6, 5, 4, 3, 2, 1, 8, 7]);
    assert!(cx.debug_bounds("workspace-more-0").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.search_query = "needle session 1".into();
            app.refresh_search(cx, false);
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    assert_eq!(visible_sessions(&app, cx), vec![1]);
}

#[gpui_kit::test]
fn chat_new_session_uses_the_current_workspace_and_preserves_the_draft(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use gpui_kit::{Focusable as _, px, size};
    let (app, cx) = fixture(cx, 2);
    cx.simulate_resize(size(px(1200.), px(800.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.store.workspaces.insert(
                0,
                Workspace {
                    path: std::env::temp_dir().join("other-workspace"),
                    sessions: vec![],
                },
            );
            app.selected_workspace = Some(1);
            app.new_session_provider = crate::domain::Provider::Codex;
            app.input.update(cx, |input, cx| {
                input.set_value("尚未发送的草稿", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    draw(&app, cx);
    cx.update(|window, cx| {
        let new = window.find("chat-new-session").bounds();
        let permission = window.find("permission-picker-trigger").bounds();
        assert!(new.right() <= permission.left());
        window.click("chat-new-session", cx);
    });
    cx.run_until_parked();
    draw(&app, cx);
    cx.update(|window, cx| {
        let app = app.read(cx);
        let id = app.selected_session.unwrap();
        assert_ne!(id, 2);
        assert_eq!(app.selected_workspace, Some(1));
        assert!(app.store.workspaces[0].sessions.is_empty());
        assert_eq!(app.store.workspaces[1].sessions.len(), 3);
        assert_eq!(
            app.session(id).unwrap().provider,
            crate::domain::Provider::Codex
        );
        assert_eq!(app.drafts.get(&2).unwrap(), "尚未发送的草稿");
        assert!(app.input.read(cx).value().is_empty());
        assert!(app.input.read(cx).focus_handle(cx).is_focused(window));
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| app.read(cx).input.read(cx).value()),
        "尚未发送的草稿"
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        })
    });
    draw(&app, cx);
    cx.update(|window, cx| window.click("chat-new-session", cx));
    assert_eq!(
        cx.update(|_, cx| app.read(cx).store.workspaces[1].sessions.len()),
        3
    );
}

#[gpui_kit::test]
fn about_stays_centered_from_the_first_frame_and_after_resize(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use gpui_kit::{px, size};
    let (app, cx) = fixture(cx, 1);
    cx.update(|window, cx| {
        cx.set_reduce_motion(false);
        app.update(cx, |app, cx| app.open_about_dialog(window, cx));
    });
    for (width, height) in [(1200., 800.), (850., 500.)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.run_until_parked();
        draw(&app, cx);
        let bounds = cx.debug_bounds("dialog-0").unwrap();
        assert!(
            (f32::from(bounds.center().x) - width / 2.).abs() < 1.,
            "{bounds:?}"
        );
        assert!(
            (f32::from(bounds.center().y) - height / 2.).abs() < 1.,
            "{bounds:?}"
        );
        assert!(bounds.top() >= px(0.) && bounds.bottom() <= px(height));
    }
}

#[gpui_kit::test]
fn ten_thousand_sessions_only_construct_viewport_rows(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
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
    let _locale = crate::test_support::chinese();
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
fn pending_push_changes_refresh_the_cached_branch_entry(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, 1);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.git.branch_probe = None;
            app.git.repository = Some(crate::git::Repository {
                current: "main".into(),
                detached: false,
                branches: vec![],
                truncated: false,
                upstream: None,
                ahead: 0,
                context: Default::default(),
                branch_info: vec![],
                integration: None,
                conflicts: vec![],
                remotes: vec![],
            });
        });
    });
    for (ahead, detached, upstream, label) in [
        (
            2,
            false,
            Some("origin/main"),
            "Git 分支与日志 · 推送 2 个提交到 origin/main",
        ),
        (
            120,
            false,
            Some("origin/main"),
            "Git 分支与日志 · 推送 120 个提交到 origin/main",
        ),
        (
            120,
            false,
            Some("origin/release"),
            "Git 分支与日志 · 推送 120 个提交到 origin/release",
        ),
        (
            120,
            false,
            None,
            "Git 分支与日志 · 120 个本地未发布提交；尚未设置上游",
        ),
        (0, false, None, "Git 分支与日志"),
        (2, true, Some("origin/main"), "Git 分支与日志"),
    ] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let repository = app.git.repository.as_mut().unwrap();
                repository.ahead = ahead;
                repository.detached = detached;
                repository.upstream = upstream.map(str::to_owned);
                cx.notify();
            });
        });
        cx.run_until_parked();
        // Draw without explicitly invalidating SidebarView: Git status alone
        // must refresh the rail while workspace/session rows stay cached.
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            let branch =
                gpui_kit::base::test_support::find(window, &[], &"nav-branches".into()).unwrap();
            assert_eq!(branch.label(), Some(label));
            assert!(
                gpui_kit::base::test_support::find(window, &[], &"git-push-pending".into())
                    .is_none()
            );
        });
        assert_eq!(
            cx.debug_bounds("nav-branches-badge").is_some(),
            ahead > 0 && !detached
        );
    }
}

#[gpui_kit::test]
fn reordering_activity_above_viewport_preserves_top_session_and_offset(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
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
    let _locale = crate::test_support::chinese();
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
    let _locale = crate::test_support::chinese();
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
    let _locale = crate::test_support::chinese();
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
