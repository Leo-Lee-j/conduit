//! Native settings redesign regressions: navigation must not mutate session state,
//! migrated search results must reveal real controls, and UI reset stays scoped.
//! Bounds are used only to deliver clicks, never as layout expectations.
// Explicit imports keep the GPUI test attribute out of the unqualified `test`
// namespace: its generated #[test] must resolve to Rust's built-in attribute.
use super::{navigation, search};
use crate::{
    app::ConduitApp,
    domain::Provider,
    providers::ModelChannel,
    store::{DEFAULT_UI_FONT_SIZE, Store},
};
use gpui_kit::Focusable;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ElementId, Entity, Modifiers, MouseButton, Role, TestAppContext,
    VisualTestContext,
    base::test_support::{ElementSnapshot, snapshots},
    px, size,
};
use serde_json::Value;

fn fixture(cx: &mut TestAppContext, store: Store) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.writable = false;
            // Browsing this fixture must not discover installed CLIs or services.
            app.connection_settings.syncing = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(1400.), px(900.)));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    (app, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        // The pinned test platform has no frame loop. Deliver callbacks in
        // the same post-render order as Windows, including focus repair.
        if window.simulate_next_frame(cx) > 0 {
            let _ = window.draw(cx);
        }
    });
}

fn elements(cx: &mut VisualTestContext) -> Vec<ElementSnapshot> {
    cx.update(|window, _| snapshots(window))
}

fn control(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> ElementSnapshot {
    let id = id.into();
    cx.update(|window, _| window.find(id))
}

const NAV_SELECTORS: [&str; 12] = [
    "settings-nav-0",
    "settings-nav-1",
    "settings-nav-2",
    "settings-nav-3",
    "settings-nav-4",
    "settings-nav-5",
    "settings-nav-6",
    "settings-nav-7",
    "settings-nav-8",
    "settings-nav-9",
    "settings-nav-10",
    "settings-nav-11",
];
const DEFAULT_CHANNEL_SELECTORS: [&str; Provider::ALL.len()] = [
    "default-channel-0",
    "default-channel-1",
    "default-channel-2",
    "default-channel-3",
    "default-channel-4",
];

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let center = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn select_page(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, page: usize) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| app.select_settings_page(page, window, cx));
    });
    draw(cx);
}

fn query(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, text: &str) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_ui.search.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        });
    });
    cx.simulate_input(text);
    draw(cx);
}

fn business_state(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> Value {
    cx.update(|_, cx| {
        let app = app.read(cx);
        serde_json::json!({
            "store": serde_json::to_value(&app.store).unwrap(),
            "management_engine": app.connection_settings.engine,
            "new_session_engine": app.new_session_provider,
            "selected_workspace": app.selected_workspace,
            "selected_session": app.selected_session,
            "current_engine": app.selected_session
                .and_then(|id| app.session(id)).map(|session| session.provider),
            "pending_permission": app.pending_permission,
            "composer_draft": app.input.read(cx).value().to_string(),
        })
    })
}

/// Exercise real tab traversal, Button activation and the native popup's arrow
/// navigation. No SelectState entity or menu-item ordering is assumed.
pub(in crate::app) fn keyboard_category(cx: &mut VisualTestContext, page: usize) {
    assert!(cx.debug_bounds("settings-category-selector").is_some());
    let mut focused = false;
    for _ in 0..64 {
        let button = control(cx, "settings-category-selector");
        assert!(
            button.visible(),
            "the text category selector must be available"
        );
        assert_ne!(
            button.disabled(),
            Some(true),
            "read-only mode may not disable browsing"
        );
        assert!(button.label().is_some_and(|label| !label.trim().is_empty()));
        if button.focused() == Some(true) {
            focused = true;
            break;
        }
        cx.simulate_keystrokes("tab");
        draw(cx);
    }
    assert!(
        focused,
        "the compact text category selector must be keyboard reachable"
    );
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(
        elements(cx)
            .iter()
            .any(|element| { element.visible() && element.role() == Some(Role::Menu) }),
        "Enter must open the native category menu"
    );

    let title = conduit::i18n::source(navigation::PAGES[page].1);
    for _ in 0..=navigation::PAGES.len() {
        if elements(cx).iter().any(|element| {
            element.visible()
                && element.role() == Some(Role::MenuItem)
                && element.selected() == Some(true)
                && element.label().is_some_and(|label| label.contains(title))
        }) {
            cx.simulate_keystrokes("enter");
            draw(cx);
            return;
        }
        cx.simulate_keystrokes("down");
        draw(cx);
    }
    panic!("keyboard navigation did not reach settings category {title}");
}

fn assert_target(cx: &mut VisualTestContext, page: usize, group: usize) {
    match (page, group) {
        (10, 1) => {
            let permission = control(cx, "default-session-permission");
            assert!(
                permission.visible(),
                "permission search must reveal its actual picker"
            );
        }
        (10, 2) => {
            for key in [
                "settings.general.collapse_reasoning",
                "settings.general.auto_collapse_question_history",
            ] {
                let label = conduit::i18n::text_static(key);
                assert!(
                    elements(cx).iter().any(|element| {
                        element.visible()
                            && element.role() == Some(Role::Switch)
                            && element.label() == Some(label)
                    }),
                    "display search must reveal the actual switch: {label}"
                );
            }
        }
        (10, 3) => {
            // An expanded native picker must exist. Its initial viewport position
            // depends on font size and is intentionally not a layout assertion.
            let model = control(cx, "naming-model-trigger");
            assert_eq!(model.role(), Some(Role::Button));
            assert_eq!(
                model.disabled(),
                Some(true),
                "naming search must retain read-only guards"
            );
        }

        (1, 1) => {
            assert!(
                cx.debug_bounds("appearance-ui-reset").is_some(),
                "font search must reveal the merged typography controls"
            );
        }
        _ => panic!("unrecognized regression target {page}/{group}"),
    }
}

#[gpui_kit::test]
fn settings_redesign_compact_readonly_categories_and_search_work_with_keyboard(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, Store::default());
    let before = business_state(&app, cx);
    cx.simulate_resize(size(px(850.), px(500.)));
    draw(cx);
    keyboard_category(cx, 1);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.settings_ui.page, 1);
        assert!(!app.writable);
    });

    query(&app, cx, "自动命名");
    let result = control(cx, ("settings-search-result", 0_usize));
    assert_eq!(result.role(), Some(Role::Button));
    assert!(
        result
            .label()
            .is_some_and(|label| label.contains("自动命名")),
        "a search result with custom content must expose its localized target name"
    );
    assert!(cx.debug_bounds("settings-search-result-0").is_some());
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.settings_ui.page, 10);
        assert_eq!(app.settings_ui.group, Some(3));
        assert!(app.settings_ui.naming_expanded);
        assert!(app.settings_ui.search.read(cx).value().is_empty());
        assert!(app.settings_ui.open && !app.writable);
    });
    assert_target(cx, 10, 3);
    assert_eq!(
        business_state(&app, cx),
        before,
        "read-only category/search navigation must not mutate preferences or sessions"
    );
}

#[gpui_kit::test]
fn settings_redesign_legacy_naming_route_and_migrated_search_reveal_real_targets(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, Store::default());
    let before = business_state(&app, cx);
    // Numeric identities and visible-category mapping are separate contracts.
    // Button selection styling is not a pixel/snapshot expectation.
    for index in 0..12 {
        let route = navigation::SettingsRoute::from_index(index).unwrap();
        assert_eq!(route.index(), index);
        assert_eq!(
            route.navigation_index(),
            if index == 5 { 10 } else { index }
        );
    }
    assert!(navigation::SettingsRoute::from_index(12).is_none());
    select_page(&app, cx, 5);
    assert!(
        cx.debug_bounds("settings-nav-5").is_none(),
        "legacy route 5 is not a visible category"
    );
    assert!(cx.debug_bounds("settings-nav-10").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.settings_ui.page, 5,
            "numeric route identity must stay compatible"
        );
        assert_eq!(
            navigation::SettingsRoute::from_index(app.settings_ui.page)
                .unwrap()
                .navigation_index(),
            10
        );
        assert!(app.settings_ui.group.is_none());
    });
    assert!(
        elements(cx).iter().any(|element| {
            element.visible()
                && element.role() == Some(Role::Switch)
                && element.label()
                    == Some(conduit::i18n::text_static(
                        "settings.defaults.automatic_naming",
                    ))
        }),
        "legacy route 5 must still render automatic naming, not session defaults"
    );

    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.settings_ui.naming_expanded = false;
            app.open_settings_result(
                search::SettingsResult {
                    page: 5,
                    group: Some(1),
                    title: "legacy naming model link".into(),
                    description: String::new(),
                    engine: None,
                    target: None,
                },
                window,
                cx,
            );
            assert_eq!((app.settings_ui.page, app.settings_ui.group), (10, Some(3)));
            assert!(
                app.settings_ui.naming_expanded,
                "a retained legacy naming search target must open the real disclosure"
            );
        });
    });
    draw(cx);
    assert_target(cx, 10, 3);

    for (text, page, group) in [
        ("默认会话权限", 10, 1),
        ("思考", 10, 2),
        ("问答", 10, 2),
        ("自动命名", 10, 3),
        ("重命名 标题", 10, 3),
        ("font", 1, 1),
    ] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                // A prior disclosure may not hide a newly opened search target.
                app.settings_ui.naming_expanded = false;
                cx.notify();
            });
        });
        query(&app, cx, text);
        cx.update(|_, cx| {
            let app = app.read(cx);
            let result = app
                .settings_search_matches(text)
                .into_iter()
                .next()
                .unwrap_or_else(|| panic!("legacy search alias {text:?} stopped matching"));
            assert_eq!(
                (result.page, result.group),
                (page, Some(group)),
                "the first result for {text:?} must point at the migrated group"
            );
        });
        click(cx, "settings-search-result-0");
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                (app.settings_ui.page, app.settings_ui.group),
                (page, Some(group))
            );
            assert!(app.settings_ui.search.read(cx).value().is_empty());
        });
        assert_target(cx, page, group);
    }
    assert_eq!(
        business_state(&app, cx),
        before,
        "compatibility routes and unqualified search may change navigation, never business state"
    );

    // Qualified default-engine search intentionally changes only the management
    // browsing engine. It must not rewrite defaults or the current session engine.
    for engine in Provider::ALL {
        let text = cx.update(|_, cx| {
            app.read(cx)
                .settings_search_matches(engine.label())
                .into_iter()
                .find(|result| {
                    result.page == 10 && result.group == Some(0) && result.engine == Some(engine)
                })
                .unwrap_or_else(|| panic!("missing explicit {engine:?} defaults search target"))
                .title
        });
        query(&app, cx, &text);
        cx.update(|_, cx| {
            let result = app
                .read(cx)
                .settings_search_matches(&text)
                .into_iter()
                .next()
                .unwrap();
            assert_eq!(
                (result.page, result.group, result.engine),
                (10, Some(0), Some(engine))
            );
        });
        click(cx, "settings-search-result-0");
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!((app.settings_ui.page, app.settings_ui.group), (10, Some(0)));
            assert_eq!(app.connection_settings.engine, engine);
        });
        let channel = control(cx, ("default-channel", engine as usize));
        assert!(
            channel.visible(),
            "engine search must reveal the actual default-service picker"
        );
        for other in Provider::ALL {
            let present = cx
                .debug_bounds(DEFAULT_CHANNEL_SELECTORS[other as usize])
                .is_some();
            assert_eq!(
                present,
                other == engine,
                "group 0 shows only the selected engine's defaults"
            );
        }
        let mut expected = before.clone();
        expected["management_engine"] = serde_json::json!(engine);
        assert_eq!(
            business_state(&app, cx),
            expected,
            "explicit engine search is browsing, not a default/current-session mutation"
        );
    }
}

#[gpui_kit::test]
fn settings_redesign_resize_and_categories_preserve_engines_and_pending_draft_entities(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let mut store = Store::default();
    let mut session = crate::app::tests::session();
    session.provider = Provider::Claude;
    session.model = Some("current-session-model".into());
    store.workspaces.push(crate::domain::Workspace {
        path: std::env::temp_dir(),
        sessions: vec![session],
    });
    for engine in Provider::ALL {
        store.session_defaults.insert(
            engine,
            crate::store::SessionDefaults {
                channel: ModelChannel::Cli,
                model: Some(format!("engine-{}-new-session-default", engine as usize)),
            },
        );
    }
    let (app, cx) = fixture(cx, store);
    let (input, search) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.connection_settings.engine = Provider::Codex;
            app.new_session_provider = Provider::OpenCode;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            assert_eq!(app.session(1).unwrap().provider, Provider::Claude);
            app.input.update(cx, |input, cx| {
                input.set_value("unsent settings QA draft", window, cx)
            });
            cx.notify();
            (app.input.clone(), app.settings_ui.search.clone())
        })
    });
    draw(cx);
    let before = business_state(&app, cx);

    for (width, height, page, compact) in [
        (850., 500., 10, true),
        (1400., 900., 1, false),
        (850., 500., 8, true),
        (1400., 900., 10, false),
    ] {
        cx.simulate_resize(size(px(width), px(height)));
        draw(cx);
        if compact {
            keyboard_category(cx, page);
        } else {
            click(cx, NAV_SELECTORS[page]);
        }
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(app.settings_ui.page, page);
            assert_eq!(
                app.input.entity_id(),
                input.entity_id(),
                "responsive navigation may not replace an input with an in-progress draft"
            );
            assert_eq!(
                app.settings_ui.search.entity_id(),
                search.entity_id(),
                "responsive navigation must retain the search input entity"
            );
        });
        assert_eq!(
            business_state(&app, cx),
            before,
            "resizing and category navigation must preserve current/default engines and drafts"
        );
    }

    query(&app, cx, "自动命名");
    let search_range = cx.update(|_, cx| app.read(cx).settings_ui.search.read(cx).selected_range());
    for (width, height) in [(1400., 900.), (850., 500.), (1400., 900.)] {
        cx.simulate_resize(size(px(width), px(height)));
        draw(cx);
        cx.update(|window, cx| {
            let app = app.read(cx);
            assert_eq!(app.settings_ui.search.entity_id(), search.entity_id());
            assert!(
                app.settings_ui
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window),
                "a retained search editor may not lose focus during reflow"
            );
            assert_eq!(
                app.settings_ui.search.read(cx).selected_range(),
                search_range
            );
            assert_eq!(
                app.settings_ui.search.read(cx).value().as_str(),
                "自动命名",
                "a layout change must not discard a search in progress"
            );
        });
        assert!(cx.debug_bounds("settings-search-result-0").is_some());
        assert_eq!(business_state(&app, cx), before);
    }
}

#[gpui_kit::test]
fn settings_redesign_ui_font_reset_does_not_reset_chat_or_code_preferences(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, Store::default());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.ui_font_family = Some("Segoe UI".into());
            app.store.ui.ui_font_size = DEFAULT_UI_FONT_SIZE + 4.;
            app.store.ui.chat_font_family = Some("Segoe UI".into());
            app.store.ui.chat_font_size = 18.;
            app.store.ui.chat_line_height = 1.8;
            app.store.ui.code_font_family = Some("Consolas".into());
            app.store.ui.code_font_size = 16.;
            app.store.ui.code_copy_button = false;
            app.store.ui.code_syntax_highlight = false;
            app.store.ui.code_line_numbers = true;
            app.select_settings_page(1, window, cx);
            app.settings_ui.group = Some(1);
            cx.notify();
        });
    });
    draw(cx);
    let readonly = business_state(&app, cx);
    click(cx, "appearance-ui-reset");
    assert_eq!(
        business_state(&app, cx),
        readonly,
        "read-only UI reset must remain guarded"
    );
    let before = cx.update(|_, cx| serde_json::to_value(&app.read(cx).store.ui).unwrap());
    let mut expected = before;
    expected["ui_font_family"] = Value::Null;
    expected["ui_font_size"] = serde_json::json!(DEFAULT_UI_FONT_SIZE);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "appearance-ui-reset");
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            serde_json::to_value(&app.store.ui).unwrap(),
            expected,
            "UI reset may only reset the interface font family/size, not chat/code typography"
        );
    });
}
#[gpui_kit::test]
fn settings_redesign_font_family_controls_fit_the_actual_pane_at_large_ui_sizes(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    for font_size in [12., 16., 20.] {
        let mut store = Store::default();
        store.ui.ui_font_size = font_size;
        let (app, cx) = fixture(cx, store);
        select_page(&app, cx, 1);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.settings_ui.group = Some(1);
                cx.notify();
            });
        });
        for width in [850., 1000., 1400.] {
            cx.simulate_resize(size(px(width), px(1800.)));
            draw(cx);
            let pane = cx.debug_bounds("settings-content").unwrap();
            for selector in [
                "appearance-ui-font",
                "appearance-chat-font",
                "appearance-code-font",
            ] {
                let field = cx.debug_bounds(selector).expect(selector);
                assert!(field.size.width > px(0.));
                assert!(
                    field.left() >= pane.left() && field.right() <= pane.right(),
                    "{selector} may not clip outside its real pane at {width}px / {font_size}px UI font: {field:?} in {pane:?}"
                );
            }
        }
    }
}
