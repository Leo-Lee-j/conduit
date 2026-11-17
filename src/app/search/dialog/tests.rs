use super::{Category, FileKey, ResultItem, WorkspaceSearch};
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext,
    component::WindowExt as _, px, size,
};
use std::time::Duration;

fn settle(search: &Entity<WorkspaceSearch>, cx: &mut VisualTestContext) {
    for _ in 0..12 {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        if cx.update(|_, cx| {
            let search = search.read(cx);
            !search.searching && search.scanning.is_empty()
        }) {
            return;
        }
    }
    panic!("search did not settle");
}

#[gpui_kit::test]
fn unified_search_scopes_both_sources_and_keeps_sidebar_and_drafts(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "unified-search");
    let local = root.join("first");
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();
    std::fs::write(
        local.join("unified-needle.txt"),
        "preview from the first workspace",
    )
    .unwrap();
    std::fs::write(
        root.join("other/remote-needle.txt"),
        "preview from another workspace",
    )
    .unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.store.workspaces[0].path = local.clone();
            app.session_mut(1).unwrap().title = "unified-needle session".into();
            app.session_mut(2).unwrap().title = "remote-needle session".into();
            app.input.update(cx, |input, cx| {
                input.set_value("unfinished draft", window, cx)
            });
        })
    });
    let search = cx.update(|window, cx| {
        let workspaces = app
            .read(cx)
            .store
            .workspaces
            .iter()
            .map(|workspace| (workspace.path.clone(), workspace.name()))
            .collect();
        cx.new(|cx| {
            WorkspaceSearch::new(
                app.downgrade(),
                Category::All,
                workspaces,
                Some(local),
                window,
                cx,
            )
        })
    });
    cx.update(|window, cx| {
        search.update(cx, |search, cx| {
            search
                .input
                .update(cx, |input, cx| input.set_value("obsolete", window, cx));
            search
                .input
                .update(cx, |input, cx| input.set_value("needle", window, cx));
        })
    });
    settle(&search, cx);
    cx.update(|_, cx| {
        let search = search.read(cx);
        assert_eq!(search.counts, (1, 1));
        assert!(
            search
                .results
                .iter()
                .all(|item| item.title().contains("unified-needle"))
        );
        let app = app.read(cx);
        assert!(app.search_query.is_empty());
        assert_eq!(app.input.read(cx).value(), "unfinished draft");
        assert_eq!(app.selected_session, Some(1));
    });
    cx.update(|_, cx| {
        search.update(cx, |search, cx| {
            search.scope = None;
            search.refresh(cx);
        })
    });
    settle(&search, cx);
    cx.update(|_, cx| search.update(cx, |search, _| {
        assert_eq!(search.counts, (2, 2));
        search.category = Category::Files;
        search.filter_category();
        assert!(search.results.iter().all(|item| matches!(item, ResultItem::File { .. })));
        search.selected = search.results.iter().position(|item| matches!(item, ResultItem::File { key, .. } if key.workspace == root.join("other"))).unwrap();
    }));
    cx.run_until_parked();
    cx.update(|window, cx| {
        search.update(cx, |search, cx| {
            let key = FileKey {
                workspace: root.join("other"),
                relative: "remote-needle.txt".into(),
            };
            assert!(search.results.iter().any(
                |item| matches!(item, ResultItem::File { key: result, .. } if result == &key)
            ));
            search.open_selected(window, cx);
        })
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_workspace, Some(1));
        assert_eq!(
            app.current_file_key(),
            Some(FileKey {
                workspace: root.join("other"),
                relative: "remote-needle.txt".into()
            })
        );
        assert_eq!(
            app.drafts.get(&1).map(String::as_str),
            Some("unfinished draft")
        );
    });
}

#[gpui_kit::test]
fn search_entry_and_file_shortcut_share_the_dialog_and_open_result_on_click(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "unified-search-entry");
    std::fs::create_dir_all(root.join("other")).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = false;
        })
    });
    cx.simulate_resize(size(px(850.), px(500.)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("sidebar-file-search").is_none());
    let search = cx.debug_bounds("sidebar-search").unwrap();
    let add = cx.debug_bounds("add-workspace").unwrap();
    assert!(search.right() <= add.left());
    cx.simulate_mouse_down(search.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(search.center(), MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("workspace-search-skeleton-0").is_some());
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let bounds = cx.debug_bounds("workspace-search").expect("unified dialog");
    assert!(bounds.left() >= px(0.) && bounds.right() <= px(850.));
    assert!(bounds.top() >= px(0.) && bounds.bottom() <= px(500.));
    assert!(cx.debug_bounds("workspace-search-preview").is_none());
    assert!(cx.debug_bounds("workspace-search-open").is_none());
    let row = cx.debug_bounds("workspace-search-result-0").unwrap();
    cx.simulate_mouse_down(row.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(row.center(), MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(app.read(cx).selected_session, Some(1)));
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        app.update(cx, |app, cx| app.open_search_dialog(window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        app.update(cx, |app, cx| app.open_file_search(window, cx));
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("workspace-search").is_some());
    let row = cx
        .debug_bounds("workspace-search-result-0")
        .expect("file result");
    cx.simulate_mouse_down(row.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(row.center(), MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(app.read(cx).current_file_key().is_some());
    });
}
