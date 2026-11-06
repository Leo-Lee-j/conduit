use super::{ConduitApp, Store};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, px, size,
};

fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(Store::default()), false, false);
            app.startup = None;
            app.writable = false;
            app.connection_settings.syncing = true;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(850.), px(500.)));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    (app, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn settings_engine_management_is_readonly_and_disabled_launches_preserve_history(
    cx: &mut TestAppContext,
) {
    use crate::domain::Provider;
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.connection_settings.engine = Provider::Codex;
            app.select_settings_page(3, window, cx);
            app.set_engine_enabled(Provider::Codex, false, cx);
            assert!(
                app.store.cli.enabled(Provider::Codex),
                "read-only mutation must be rejected"
            );
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("engine-enabled-1").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            app.store.cli.set_enabled(Provider::Codex, false);
            app.new_session_provider = Provider::Claude;
            app.select_engine(Provider::Codex, window, cx);
            assert_eq!(app.new_session_provider, Provider::Claude);
            app.store.workspaces.push(crate::domain::Workspace {
                path: std::env::temp_dir(),
                sessions: vec![],
            });
            let before = serde_json::to_value(&app.store.workspaces).unwrap();
            let error = app
                .start_turn(0, None, Provider::Codex, "offline prompt".into(), true, cx)
                .unwrap_err();
            assert!(error.contains("已停用"));
            assert_eq!(serde_json::to_value(&app.store.workspaces).unwrap(), before);
            assert!(app.session_control.runs.is_empty());
            app.writable = false;
        })
    });
}

#[gpui_kit::test]
fn settings_extension_filters_keep_the_list_visible_at_small_and_large_sizes(
    cx: &mut TestAppContext,
) {
    use crate::{
        domain::Provider,
        extensions::{Catalog, Context, Entry, Kind, management::Filter},
    };
    let (app, cx) = fixture(cx);
    for (width, height) in [(850., 500.), (1280., 900.)] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.settings_ui.page = 4;
                app.extensions_ui.initialized = true;
                app.extensions_ui.probe = None;
                let context = Context {
                    engine: Provider::Codex,
                    workspace: None,
                    cli_path: None,
                };
                app.extensions_ui.context = Some(context.clone());
                app.extensions_ui.catalogs.insert(
                    context,
                    Catalog {
                        entries: vec![Entry {
                            name: "sample".into(),
                            description: "Skill description".into(),
                            path: "C:/test/SKILL.md".into(),
                            scope: "全局".into(),
                            kind: Kind::Skill,
                            disabled: true,
                            transport: String::new(),
                            target: String::new(),
                        }],
                        ..Default::default()
                    },
                );
                app.extensions_ui.filters[0] = Filter::All;
                app.extensions_ui.lists[0].reset(0);
                cx.notify();
            })
        });
        draw(cx);
        let list = cx.debug_bounds("extensions-list").unwrap();
        assert!(
            list.size.height > px(50.),
            "filters must leave room for the list"
        );
        let filter = cx.debug_bounds("extensions-filter-trigger").unwrap();
        cx.simulate_mouse_down(filter.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(filter.center(), MouseButton::Left, Modifiers::default());
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.extensions_ui.filters[0] = Filter::Disabled;
                app.extensions_ui.lists[0].reset(0);
                cx.notify();
            })
        });
        cx.simulate_keystrokes("escape");
        draw(cx);
        assert!(app.read_with(cx, |app, _| app.extensions_ui.filters[0]
            == Filter::Disabled));
        assert!(cx.debug_bounds("extension-detail-0").is_some());
    }
}

#[gpui_kit::test]
fn settings_keyboard_search_reveals_deep_section_and_escape_clears_before_leaving(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx);
    cx.simulate_keystrokes("ctrl-f");
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            app.read(cx)
                .settings_ui
                .search
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
    cx.simulate_input("提示词");
    draw(cx);
    assert!(cx.debug_bounds("settings-search-result-0").is_some());
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.settings_ui.page, 6);
        assert_eq!(app.settings_ui.group, Some(2));
        assert!(app.settings_ui.prompt_expanded);
        assert!(app.settings_ui.search.read(cx).value().is_empty());
    });
    let body = cx.debug_bounds("settings-workspace-body").unwrap();
    let prompt = cx
        .debug_bounds("git-prompt-expand")
        .expect("search opens the prompt section");
    assert!(prompt.top() >= body.top() && prompt.bottom() <= body.bottom());
    let full = cx.debug_bounds("settings-show-full-page").unwrap();
    cx.simulate_mouse_down(full.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(full.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    assert_eq!(app.read_with(cx, |app, _| app.settings_ui.group), None);
    assert!(cx.debug_bounds("settings-show-full-page").is_none());
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("unmatched-setting-xyz");
    draw(cx);
    assert!(cx.debug_bounds("settings-search-result-0").is_none());
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|_, cx| {
        assert!(app.read(cx).settings_ui.open);
        assert!(app.read(cx).settings_ui.search.read(cx).value().is_empty());
        assert_eq!(app.read(cx).settings_ui.page, 6);
    });
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!app.read_with(cx, |app, _| app.settings_ui.open));
}

#[gpui_kit::test]
fn settings_child_dialog_escape_preserves_workspace_and_navigation(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_settings_page(1, window, cx);
            app.open_about_dialog(window, cx);
        })
    });
    draw(cx);
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|window, cx| {
        assert!(!window.has_active_dialog(cx));
        assert!(app.read(cx).settings_ui.open);
        assert_eq!(app.read(cx).settings_ui.page, 1);
    });
    assert!(cx.debug_bounds("appearance-theme-system").is_some());
}
