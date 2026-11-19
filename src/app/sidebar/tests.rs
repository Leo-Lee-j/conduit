use super::{ConduitApp, TaskState, cache};
use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn project_groups_keep_nested_guides_and_keyboard_navigation_connected(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "sidebar-project-tree");
    let base = root.join("jupiter");
    let copy = root.join("jupiter-2");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.store.ui.auto_collapse_inactive_workspaces = false;
            app.store.ui.auto_sort_workspaces = false;
            for (workspace, path) in app.store.workspaces.iter_mut().zip([&base, &copy]) {
                workspace.path = path.clone();
                workspace.sessions[0].created_at = super::unix_now() - 100;
            }
            app.refresh_sidebar();
            assert!(matches!(
                &app.sidebar.rows[0],
                cache::Row::Project {
                    copies: 2,
                    count: 2,
                    ..
                }
            ));
            app.sidebar.scroll.set_offset(gpui_kit::point(px(0.), px(-4.)));
            assert!(matches!(app.sidebar.sticky_workspace(), Some((cache::Row::Project { .. }, _))));
            app.sidebar.scroll.set_offset(gpui_kit::point(px(0.), px(-44.)));
            assert!(matches!(app.sidebar.sticky_workspace(), Some((cache::Row::Workspace { path, .. }, _)) if path == base));
            app.sidebar.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
            cx.notify();
        })
    });
    cx.simulate_resize(size(px(1000.), px(800.)));
    draw(cx);
    let folder = cx.debug_bounds("project-folder-0").unwrap();
    let outer = [
        "project-guide-workspace-0",
        "project-guide-0-Day",
        "project-guide-session-1",
        "project-guide-workspace-1",
        "project-guide-1-Day",
        "project-guide-session-2",
    ]
    .map(|id| cx.debug_bounds(id).unwrap());
    for guide in outer {
        assert_eq!(guide.left(), folder.center().x);
    }
    for pair in outer.windows(2) {
        assert_eq!(pair[0].bottom(), pair[1].top());
    }
    let inner = cx.debug_bounds("session-guide-1").unwrap();
    assert_eq!(inner.left(), outer[0].left() + px(26.));
    assert_eq!(
        cx.debug_bounds("workspace-guide-0-Day").unwrap().bottom(),
        inner.top()
    );
    assert!(
        cx.debug_bounds("session-title-1").unwrap().left()
            > cx.debug_bounds("workspace-name-0").unwrap().left()
    );
    assert!(cx.debug_bounds("workspace-folder-0").is_none());

    let header = cx.debug_bounds("workspace-project-0").unwrap().center();
    cx.simulate_mouse_down(header, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(header, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("session-row-1").is_none());
    cx.update(|_, cx| assert_eq!(app.read(cx).selected_session, Some(1)));
    cx.simulate_keystrokes("right");
    draw(cx);
    cx.simulate_keystrokes("right");
    cx.update(|_, cx| assert!(matches!(&app.read(cx).sidebar.keyboard_row, Some(cache::Row::Workspace { path, .. }) if path == &base)));
    cx.simulate_keystrokes("left");
    cx.simulate_keystrokes("left");
    cx.update(|_, cx| {
        assert!(matches!(
            &app.read(cx).sidebar.keyboard_row,
            Some(cache::Row::Project { .. })
        ))
    });
    cx.simulate_keystrokes("left");
    draw(cx);
    assert!(cx.debug_bounds("workspace-item-0").is_none());
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(cx.debug_bounds("workspace-item-0").is_some());
    assert!(
        cx.debug_bounds("session-row-1").is_none(),
        "child collapse is preserved"
    );
}

#[gpui_kit::test]
fn collapsed_projects_refresh_activity_and_reveal_task_selection(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "sidebar-project-activity");
    let base = root.join("jupiter");
    let copy = root.join("jupiter-2");
    std::fs::create_dir_all(&base).unwrap();
    std::fs::create_dir_all(&copy).unwrap();
    let sender = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.store.ui.auto_collapse_inactive_workspaces = false;
            app.store.ui.auto_sort_workspaces = false;
            app.store.workspaces[0].path = base.clone();
            app.store.workspaces[1].path = copy.clone();
            app.input
                .update(cx, |input, cx| input.set_value("keep my draft", window, cx));
            app.refresh_sidebar();
            app.set_sidebar_project_expanded(&base, false, window, cx);
            app.set_workspace_collapsed(&copy, true);
            app.session_control.unread_completed.insert(2);
            assert!(app.refresh_sidebar());
            assert!(matches!(
                &app.sidebar.rows[0],
                cache::Row::Project {
                    activity: Some((2, false)),
                    ..
                }
            ));
            let sender = crate::app::queue::tests::running(app, 2);
            app.refresh_sidebar();
            assert!(matches!(
                &app.sidebar.rows[0],
                cache::Row::Project {
                    activity: Some((2, true)),
                    ..
                }
            ));
            assert_eq!(app.sidebar.rows.len(), 1);
            cx.notify();
            sender
        })
    });
    draw(cx);
    let card = cx.debug_bounds("workspace-active-task-2").unwrap().center();
    cx.simulate_mouse_down(card, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(card, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(
                app.drafts.get(&1).map(String::as_str),
                Some("keep my draft")
            );
            assert!(app.sidebar.expanded_projects.contains(&base));
            assert!(
                app.sidebar
                    .rows
                    .iter()
                    .any(|row| matches!(row, cache::Row::Session { id: 2, .. }))
            );
            app.set_sidebar_project_expanded(&base, false, window, cx);
            app.session_control.runs.clear();
            app.session_control.unread_completed.clear();
            app.refresh_sidebar();
            assert!(matches!(
                &app.sidebar.rows[0],
                cache::Row::Project { activity: None, .. }
            ));
            app.store.workspaces[0].path = root.join("unrelated");
            app.refresh_sidebar();
            assert!(
                !app.sidebar
                    .rows
                    .iter()
                    .any(|row| matches!(row, cache::Row::Project { .. }))
            );
            assert!(app.sidebar.expanded_projects.is_empty());
            assert!(app.sidebar.keyboard_row.as_ref().is_none_or(|focused| {
                app.sidebar
                    .rows
                    .iter()
                    .any(|row| row.same_identity(focused))
            }));
            cx.notify();
        })
    });
    drop(sender);
}

#[gpui_kit::test]
fn conversation_times_align_to_first_line_and_guides_join_through_expanded_content(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "sidebar-time-guides");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let now = super::unix_now();
            app.startup = None;
            app.writable = false;
            app.store.ui.auto_collapse_inactive_workspaces = false;
            app.store.ui.auto_sort_workspaces = false;
            app.store.ui.sessions_per_workspace = 3;
            app.store.workspaces[0].sessions = (1..=12)
                .filter(|id| *id != 2)
                .map(|id| {
                    let mut session = crate::app::tests::session();
                    session.id = id;
                    session.title = if id == 1 {
                        "提交所有变更并推送，检查工作空间会话在两行标题下的时间对齐与长文本截断"
                            .repeat(3)
                    } else {
                        format!("会话 {id}")
                    };
                    session.created_at = if id <= 3 {
                        now - 22 * 3600 - id * 60
                    } else {
                        now - 3 * 24 * 3600 - id * 60
                    };
                    session
                })
                .collect();
            app.refresh_sidebar();
            cx.notify();
        })
    });
    for width in [850., 1280.] {
        cx.simulate_resize(size(px(width), px(750.)));
        draw(cx);
        let title = cx.debug_bounds("session-title-1").unwrap();
        let age = cx.debug_bounds("session-age-1").unwrap();
        let row = cx.debug_bounds("session-row-1").unwrap();
        assert!(
            title.size.height > age.size.height,
            "fixture must wrap the title"
        );
        assert_eq!(
            title.top(),
            age.top(),
            "time aligns with the first title line"
        );
        assert_eq!(age.size.height, px(16.), "relative time stays on one line");
        assert_eq!(age.right(), row.right() - px(8.));
        let short_title = cx.debug_bounds("session-title-3").unwrap();
        let short_age = cx.debug_bounds("session-age-3").unwrap();
        assert_eq!(short_title.top(), short_age.top());
        assert_eq!(age.right(), short_age.right());

        let folder = cx.debug_bounds("workspace-folder-0").unwrap();
        let guides = [
            "workspace-guide-0-Day",
            "session-guide-1",
            "session-guide-3",
            "workspace-guide-0-Week",
            "session-guide-4",
            "workspace-guide-0-more",
        ]
        .map(|selector| cx.debug_bounds(selector).unwrap());
        for guide in guides {
            assert_eq!(
                guide.left(),
                folder.center().x,
                "guide follows the folder axis"
            );
            assert_eq!(guide.size.width, px(1.));
        }
        for pair in guides.windows(2) {
            assert_eq!(
                pair[0].bottom(),
                pair[1].top(),
                "guide crosses row and group gaps"
            );
        }
        let more = cx.debug_bounds("workspace-more-0").unwrap();
        assert_eq!(guides.last().unwrap().bottom(), more.bottom());
        assert!(
            guides.last().unwrap().bottom() < cx.debug_bounds("workspace-item-1").unwrap().top()
        );
    }

    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.set_sidebar_sessions_expanded(&root, true, window, cx);
            let index = app
                .sidebar
                .rows
                .iter()
                .position(|row| matches!(row, cache::Row::Session { id: 9, .. }))
                .unwrap();
            let y: f32 = app.sidebar.rows[..index]
                .iter()
                .map(cache::Row::list_height)
                .sum();
            app.sidebar
                .scroll
                .set_offset(gpui_kit::point(px(0.), px(-y)));
            cx.notify();
        })
    });
    draw(cx);
    let ninth = cx.debug_bounds("session-guide-9").unwrap();
    let tenth = cx.debug_bounds("session-guide-10").unwrap();
    assert_eq!(
        ninth.bottom(),
        tenth.top(),
        "guide remains joined after virtual scrolling"
    );

    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.toggle_sidebar_workspace(&root, window, cx)
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("session-guide-9").is_none());
    assert!(cx.debug_bounds("workspace-guide-0-Day").is_none());
}

#[gpui_kit::test]
fn activity_cards_refresh_status_switch_sessions_and_leave_room_for_projects(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "sidebar-activity");
    let senders = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.input.update(cx, |input, cx| {
                input.set_value("unfinished draft", window, cx)
            });
            let mut senders = Vec::new();
            for id in 1..=7 {
                if id > 2 {
                    let mut session = crate::app::tests::session();
                    session.id = id;
                    session.title = format!("Background conversation {id}");
                    app.store.workspaces[0].sessions.push(session);
                }
                senders.push(crate::app::queue::tests::running(app, id));
            }
            cx.notify();
            senders
        })
    });
    cx.simulate_resize(size(px(850.), px(500.)));
    draw(cx);
    let panel = cx.debug_bounds("sidebar-active-tasks").unwrap();
    let projects = cx.debug_bounds("sidebar-projects-header").unwrap();
    assert!(panel.size.height <= px(250.));
    assert!(projects.top() >= panel.bottom());
    assert!(projects.bottom() < cx.debug_bounds("sidebar").unwrap().bottom() - px(100.));

    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.retain(|id, _| *id <= 2);
            assert!(app.refresh_sidebar());
            let run = app.session_control.runs.get_mut(&2).unwrap();
            run.approvals.push(crate::protocol::Approval {
                id: "sidebar-approval".into(),
                title: "Run a command".into(),
                detail: String::new(),
                wire_id: serde_json::json!(1),
                input: serde_json::json!({}),
                allow_once: true,
                kind: crate::protocol::ApprovalKind::ClaudeTool,
            });
            assert_eq!(
                TaskState::new(&app.session_control.runs[&2]),
                TaskState::Approval
            );
            assert!(
                app.refresh_sidebar(),
                "approval changes invalidate the sidebar without transcript edits"
            );
            cx.notify();
        })
    });
    draw(cx);
    let card = cx.debug_bounds("workspace-active-task-2").unwrap();
    let click = card.center();
    cx.simulate_mouse_down(click, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(click, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_session, Some(2));
        assert_eq!(
            app.drafts.get(&1).map(String::as_str),
            Some("unfinished draft")
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.clear();
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("sidebar-active-tasks").is_none());
    assert!(cx.debug_bounds("sidebar-projects-header").is_some());
    drop(senders);
}

#[gpui_kit::test]
fn time_groups_preserve_pinned_limits_and_roll_over_without_new_messages(cx: &mut TestAppContext) {
    use cache::{Row, SessionGroup};
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "sidebar-groups");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let now = 10_000_020;
            let day = 24 * 60 * 60;
            app.startup = None;
            app.writable = false;
            app.store.ui.auto_collapse_inactive_workspaces = false;
            app.store.ui.sessions_per_workspace = 2;
            app.store.workspaces[0].sessions = [
                (1, now - 30 * day, true),
                (3, now - day + 30, false),
                (4, now - 3 * day, false),
                (5, now - 8 * day, false),
            ]
            .into_iter()
            .map(|(id, activity, pinned)| {
                let mut session = crate::app::tests::session();
                session.id = id;
                session.created_at = activity;
                session.pinned = pinned;
                session
            })
            .collect();
            let refresh = |app: &mut ConduitApp, time| {
                let inputs = cache::Inputs::new(app).at_time(time);
                app.sidebar
                    .refresh(&app.store, &app.session_control, &app.search_ui, inputs);
            };
            refresh(app, now);
            let path = app.store.workspaces[0].path.clone();
            let visible = |app: &ConduitApp| {
                app.sidebar
                    .rows
                    .iter()
                    .filter_map(|row| match row {
                        Row::Session { path: p, id } if p == &path => Some(*id),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            let groups = |app: &ConduitApp| {
                app.sidebar
                    .rows
                    .iter()
                    .filter_map(|row| match row {
                        Row::Group { path: p, group } if p == &path => Some(*group),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(visible(app), vec![1, 3, 4]);
            assert_eq!(
                groups(app),
                vec![SessionGroup::Pinned, SessionGroup::Day, SessionGroup::Week]
            );
            app.expanded_workspaces.insert(0);
            refresh(app, now);
            assert_eq!(visible(app), vec![1, 3, 4, 5]);
            assert_eq!(
                groups(app),
                vec![
                    SessionGroup::Pinned,
                    SessionGroup::Day,
                    SessionGroup::Week,
                    SessionGroup::Older
                ]
            );
            refresh(app, now + 60);
            assert_eq!(
                groups(app),
                vec![
                    SessionGroup::Pinned,
                    SessionGroup::Week,
                    SessionGroup::Older
                ]
            );
            assert_eq!(visible(app), vec![1, 3, 4, 5]);
        })
    });
}
