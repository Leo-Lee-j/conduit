use super::{ConduitApp, cache};
use crate::{domain::Workspace, persistence::Persistence};
use gpui_kit::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};
use std::path::PathBuf;

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (Entity<ConduitApp>, &'a mut VisualTestContext, PathBuf) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, name);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = true;
            app.persistence = Persistence::with_writer(|_| Ok(()));
            app.store.ui.auto_collapse_inactive_workspaces = false;
        });
    });
    (app, cx, root)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let center = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_mouse_move(center, None, Modifiers::default());
    draw(cx);
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn order(app: &ConduitApp) -> Vec<PathBuf> {
    app.sidebar
        .rows
        .iter()
        .filter_map(|row| match row {
            cache::Row::Workspace { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn stored_order(app: &ConduitApp) -> Vec<PathBuf> {
    app.store
        .workspaces
        .iter()
        .map(|workspace| workspace.path.clone())
        .collect()
}

#[gpui_kit::test]
fn workspace_pins_preserve_manual_order_and_override_activity(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "workspace-pin-sort");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let paths = [
                root.clone(),
                root.join("other"),
                root.join("third"),
                root.join("fourth"),
            ];
            app.store.workspaces = paths
                .iter()
                .zip([10, 40, 20, 30])
                .enumerate()
                .map(|(index, (path, activity))| {
                    let mut session = crate::app::tests::session();
                    session.id = index as u64 + 1;
                    session.created_at = activity;
                    Workspace {
                        path: path.clone(),
                        sessions: vec![session],
                    }
                })
                .collect();
            for path in &paths {
                app.sidebar.invalidate_workspace(path);
            }
            app.expanded_workspaces.insert(0);
            app.remove_workspace_target = Some(1);
            app.input
                .update(cx, |input, cx| input.set_value("keep draft", window, cx));
            let tabs = app.open_tabs.clone();
            for automatic in [true, false] {
                app.store.ui.auto_sort_workspaces = automatic;
                app.refresh_sidebar();
                let normal = if automatic {
                    vec![
                        paths[1].clone(),
                        paths[3].clone(),
                        paths[2].clone(),
                        paths[0].clone(),
                    ]
                } else {
                    paths.to_vec()
                };
                assert_eq!(order(app), normal);
                app.set_workspace_pinned(&paths[2], true, cx);
                app.set_workspace_pinned(&paths[0], true, cx);
                assert_eq!(
                    order(app),
                    vec![
                        paths[0].clone(),
                        paths[2].clone(),
                        paths[1].clone(),
                        paths[3].clone()
                    ]
                );
                let sender = crate::app::queue::tests::running(app, 4);
                app.store.ui.auto_sort_workspaces = !automatic;
                app.refresh_sidebar();
                assert_eq!(&order(app)[..2], &[paths[0].clone(), paths[2].clone()]);
                app.store.ui.auto_sort_workspaces = automatic;
                app.refresh_sidebar();
                assert_eq!(&order(app)[..2], &[paths[0].clone(), paths[2].clone()]);
                if automatic {
                    assert_eq!(order(app)[2], paths[3]);
                }
                app.session_control.runs.remove(&4);
                drop(sender);
                app.set_workspace_pinned(&paths[0], false, cx);
                app.set_workspace_pinned(&paths[2], false, cx);
                assert_eq!(order(app), normal);
                assert_eq!(stored_order(app), paths);
                assert_eq!(app.selected_workspace, Some(0));
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.remove_workspace_target, Some(1));
                assert!(app.expanded_workspaces.contains(&0));
                assert_eq!(app.open_tabs, tabs);
                assert_eq!(app.input.read(cx).value(), "keep draft");
            }
        });
    });
}

#[gpui_kit::test]
fn workspace_pins_keep_project_groups_whole_and_obey_search(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "workspace-pin-project");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let first = root.join("project");
            let pinned = root.join("project-2");
            let ordinary = root.join("other");
            app.store.workspaces[0].path = ordinary.clone();
            app.store.workspaces[1].path = first.clone();
            let mut session = crate::app::tests::session();
            session.id = 3;
            app.store.workspaces.push(Workspace {
                path: pinned.clone(),
                sessions: vec![session],
            });
            for path in [&first, &pinned, &ordinary] {
                app.sidebar.invalidate_workspace(path);
            }
            app.store.ui.auto_sort_workspaces = false;
            app.selected_workspace = Some(1);
            app.selected_session = Some(2);
            app.refresh_sidebar();
            let key = app.sidebar.projects.parent(&first).unwrap().clone();
            app.sidebar.expanded_projects.insert(key.clone());
            app.set_workspace_pinned(&pinned, true, cx);
            assert_eq!(
                order(app),
                [pinned.clone(), first.clone(), ordinary.clone()]
            );
            assert!(matches!(
                &app.sidebar.rows[0],
                cache::Row::Project {
                    copies: 2,
                    count: 2,
                    ..
                }
            ));
            assert!(!app.store.ui.pinned_workspaces.contains(&first));
            app.sidebar.expanded_projects.clear();
            app.refresh_sidebar();
            assert_eq!(order(app), [ordinary.clone()]);
            assert!(matches!(&app.sidebar.rows[0], cache::Row::Project { .. }));
            app.search_query = "only ordinary".into();
            app.search_ui.result_query = app.search_query.clone();
            app.search_ui.matches = [1].into_iter().collect();
            app.search_ui.result_generation += 1;
            app.refresh_sidebar();
            assert_eq!(order(app), [ordinary.clone()]);
            assert!(
                !app.sidebar
                    .rows
                    .iter()
                    .any(|row| matches!(row, cache::Row::Project { .. }))
            );
            app.search_query = "ordinary and project".into();
            app.search_ui.result_query = app.search_query.clone();
            app.search_ui.matches = [1, 2].into_iter().collect();
            app.search_ui.result_generation += 1;
            app.refresh_sidebar();
            assert_eq!(order(app), [first.clone(), ordinary.clone()]);
            assert!(matches!(
                &app.sidebar.rows[0],
                cache::Row::Project { count: 1, .. }
            ));
            app.search_query.clear();
            app.sidebar.expanded_projects.insert(key);
            app.set_workspace_pinned(&first, true, cx);
            assert_eq!(
                order(app),
                [first.clone(), pinned.clone(), ordinary.clone()]
            );
            app.set_workspace_pinned(&pinned, false, cx);
            assert_eq!(
                order(app),
                [first.clone(), pinned.clone(), ordinary.clone()]
            );
            assert!(!app.store.ui.pinned_workspaces.contains(&pinned));
            assert!(app.store.ui.pinned_workspaces.contains(&first));
            app.set_workspace_pinned(&first, false, cx);
            assert_eq!(order(app), [ordinary, first, pinned]);
        });
    });
}

#[gpui_kit::test]
fn workspace_pin_drag_guards_keep_pin_state_and_remap_navigation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "workspace-pin-drag");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let paths = stored_order(app);
            app.store.ui.auto_sort_workspaces = true;
            app.reorder_workspace(&paths[0], &paths[1], cx);
            assert_eq!(stored_order(app), paths);
            app.set_workspace_pinned(&paths[1], true, cx);
            for automatic in [true, false] {
                app.store.ui.auto_sort_workspaces = automatic;
                app.reorder_workspace(&paths[0], &paths[1], cx);
                app.reorder_workspace(&paths[1], &paths[0], cx);
                assert_eq!(stored_order(app), paths);
                assert_eq!(app.store.ui.pinned_workspaces.len(), 1);
            }
            app.set_workspace_pinned(&paths[0], true, cx);
            app.store.ui.auto_sort_workspaces = true;
            app.expanded_workspaces.insert(0);
            app.remove_workspace_target = Some(1);
            app.reorder_workspace(&paths[0], &paths[1], cx);
            assert_eq!(stored_order(app), [paths[1].clone(), paths[0].clone()]);
            assert_eq!(app.selected_workspace, Some(1));
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.remove_workspace_target, Some(0));
            assert!(app.expanded_workspaces.contains(&1));
            let pins = app.store.ui.pinned_workspaces.clone();
            app.writable = false;
            app.set_workspace_pinned(&paths[0], false, cx);
            app.reorder_workspace(&paths[0], &paths[1], cx);
            assert_eq!(app.store.ui.pinned_workspaces, pins);
            app.writable = true;
            app.closing = true;
            app.set_workspace_pinned(&paths[0], false, cx);
            assert_eq!(app.store.ui.pinned_workspaces, pins);
            app.closing = false;
            app.set_workspace_pinned(&root.join("missing"), true, cx);
            assert_eq!(app.store.ui.pinned_workspaces, pins);
            app.set_workspace_pinned(&paths[0], false, cx);
            app.set_workspace_pinned(&paths[1], false, cx);
            app.store.ui.auto_sort_workspaces = false;
            app.reorder_workspace(&paths[1], &paths[0], cx);
            assert_eq!(stored_order(app), paths);
        });
    });
}

#[gpui_kit::test]
fn workspace_pin_icon_unpins_without_switching_chat_or_collapsing(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "workspace-pin-menu");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.auto_sort_workspaces = false;
            app.input
                .update(cx, |input, cx| input.set_value("menu draft", window, cx));
            cx.notify();
        });
    });
    cx.simulate_resize(size(px(1100.), px(800.)));
    draw(cx);
    let manual = cx.update(|_, cx| stored_order(app.read(cx)));
    let collapsed = cx.update(|_, cx| {
        let app = app.read(cx);
        (
            app.collapsed_workspaces.clone(),
            app.sidebar.auto_collapsed.clone(),
            app.expanded_workspaces.clone(),
        )
    });
    click(cx, "workspace-menu-trigger-1");
    assert!(cx.debug_bounds("workspace-pin-1").is_some());
    click(cx, "workspace-pin-1");
    assert!(cx.debug_bounds("workspace-pinned-1").is_some());
    assert!(cx.debug_bounds("workspace-pin-1").is_none());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(order(app), [manual[1].clone(), manual[0].clone()]);
        assert_eq!(app.selected_session, Some(1));
        assert_eq!(app.input.read(cx).value(), "menu draft");
    });
    click(cx, "workspace-pinned-1");
    assert!(cx.debug_bounds("workspace-pinned-1").is_none());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(order(app), manual);
        assert_eq!(stored_order(app), manual);
        assert_eq!(app.selected_workspace, Some(0));
        assert_eq!(app.selected_session, Some(1));
        assert_eq!(app.input.read(cx).value(), "menu draft");
        assert_eq!(app.collapsed_workspaces, collapsed.0);
        assert_eq!(app.sidebar.auto_collapsed, collapsed.1);
        assert_eq!(app.expanded_workspaces, collapsed.2);
        assert!(app.store.ui.pinned_workspaces.is_empty());
    });

    click(cx, "workspace-menu-trigger-1");
    click(cx, "workspace-pin-1");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            app.sidebar_view.update(cx, |_, cx| cx.notify());
            cx.notify();
        });
    });
    draw(cx);
    click(cx, "workspace-pinned-1");
    assert!(cx.debug_bounds("workspace-pinned-1").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.store.ui.pinned_workspaces.contains(&manual[1]));
        assert_eq!(app.selected_workspace, Some(0));
        assert_eq!(app.selected_session, Some(1));
        assert_eq!(app.input.read(cx).value(), "menu draft");
        assert_eq!(app.collapsed_workspaces, collapsed.0);
        assert_eq!(app.expanded_workspaces, collapsed.2);
    });
}

#[gpui_kit::test]
fn workspace_pin_is_cleared_only_after_successful_removal(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "workspace-pin-removal");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let path = app.store.workspaces[1].path.clone();
            app.set_workspace_pinned(&path, true, cx);
            let sender = crate::app::queue::tests::running(app, 2);
            app.remove_workspace(1, cx);
            assert!(app.store.ui.pinned_workspaces.contains(&path));
            app.session_control.runs.remove(&2);
            drop(sender);
            app.remove_workspace(1, cx);
            assert!(!app.store.ui.pinned_workspaces.contains(&path));
            assert_eq!(app.store.workspaces.len(), 1);
        });
    });
}
