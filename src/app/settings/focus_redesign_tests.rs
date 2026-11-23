//! Native focus regressions for settings reflow and dialog return paths.
//! Keep input identity and keyboard behavior observable without asserting geometry.
use crate::{
    app::ConduitApp,
    domain::Provider,
    providers::{ModelChannel, ModelProvider},
    store::{SessionDefaults, Store},
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ElementId, Entity, Focusable as _, Modifiers, MouseButton, Role,
    TestAppContext, VisualTestContext,
    base::test_support::{ElementSnapshot, snapshots},
    component::WindowExt as _,
    px, size,
};
use std::time::Duration;

const SERVICE_ID: &str = "focus-qa-shared-service";

fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    let provider = ModelProvider {
        id: SERVICE_ID.into(),
        name: "Shared focus QA service".into(),
        engine: Provider::Codex,
        base_url: "https://focus-qa.invalid/v1".into(),
        api_key_env: String::new(),
        protected_api_key: None,
        models: vec!["focus-qa-model".into()],
        source: Default::default(),
        model_metadata: vec![],
        claude_models: None,
    };
    let mut store = Store::default();
    store.session_defaults.insert(
        Provider::Codex,
        SessionDefaults {
            channel: ModelChannel::Provider(provider.id.clone()),
            model: Some("focus-qa-model".into()),
        },
    );
    store.providers.push(provider);

    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.writable = true;
            app.connection_settings.engine = Provider::Codex;
            // Do not discover installed CLIs or write a real settings file.
            app.connection_settings.syncing = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(1500.), px(900.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_settings(window, cx);
            app.select_settings_page(8, window, cx);
        });
    });
    draw(cx);
    assert_layout(cx, false);
    (app, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        // Match redesign_tests: deliver callbacks after the completed reflow,
        // then draw their effects. The pinned test platform has no frame loop.
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

fn named_input(cx: &mut VisualTestContext, key: &'static str) -> ElementSnapshot {
    let label = conduit::i18n::text_static(key);
    let mut matches = elements(cx)
        .into_iter()
        .filter(|element| {
            element.visible()
                && element.role() == Some(Role::TextInput)
                && element.label() == Some(label)
        })
        .collect::<Vec<_>>();
    assert_eq!(matches.len(), 1, "expected one native input named {label}");
    matches.pop().unwrap()
}

fn service_name_input(cx: &mut VisualTestContext) -> ElementSnapshot {
    named_input(cx, "settings.config_editor.service_name")
}

fn click_snapshot(cx: &mut VisualTestContext, element: ElementSnapshot) {
    assert!(element.visible(), "the actual control must be visible");
    let center = element.bounds().center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn click_selector(cx: &mut VisualTestContext, selector: &'static str) {
    let center = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn assert_layout(cx: &mut VisualTestContext, compact: bool) {
    assert!(cx.debug_bounds("settings-workspace").is_some());
    assert_eq!(
        cx.debug_bounds("settings-category-selector").is_some(),
        compact,
        "the actual category control must match the current layout"
    );
    assert_eq!(cx.debug_bounds("settings-nav-8").is_some(), !compact);
}

fn resize(cx: &mut VisualTestContext, compact: bool) {
    let width = if compact { 850. } else { 1400. };
    cx.simulate_resize(size(px(width), px(900.)));
    draw(cx);
    assert_layout(cx, compact);
}

fn navigation_target(compact: bool) -> ElementId {
    if compact {
        "settings-category-selector".into()
    } else {
        ("settings-navigation", 8usize).into()
    }
}

fn tab_to(cx: &mut VisualTestContext, id: impl Into<ElementId>) {
    let id = id.into();
    for _ in 0..=64 {
        let button = control(cx, id.clone());
        assert_eq!(button.role(), Some(Role::Button));
        assert!(button.visible(), "current navigation must be visible");
        assert_ne!(button.disabled(), Some(true));
        if button.focused() == Some(true) {
            return;
        }
        cx.simulate_keystrokes("tab");
        draw(cx);
    }
    let focused = cx.update(|window, cx| format!("{:?}", window.focused(cx)));
    panic!("Tab could not reach {id:?} after reflow/dismissal; current focus: {focused}");
}

fn assert_shared_dialog(cx: &mut VisualTestContext) {
    cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    assert!(cx.debug_bounds("provider-editor-shared-scope").is_some());
    assert_eq!(service_name_input(cx).role(), Some(Role::TextInput));
}

fn open_shared_dialog_from_navigation(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    // Exercise the real production dialog API as a settings/global command
    // invoked while the disappearing navigation control owns keyboard focus.
    // Do not keep a strong trigger FocusHandle: Root itself stores a weak return
    // handle, and the test must not manufacture a surviving orphan.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let provider = app
                .store
                .providers
                .iter()
                .find(|provider| provider.id == SERVICE_ID)
                .unwrap()
                .clone();
            app.open_provider_editor(Some(provider), window, cx);
        });
    });
    draw(cx);
    assert_shared_dialog(cx);
}

fn close_clean_dialog(cx: &mut VisualTestContext) {
    // Use real keyboard cancellation, including ProviderEditor's dirty guard.
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    // Complete any real exit/deferred focus restoration before testing Tab.
    cx.background_executor.advance_clock(Duration::from_secs(1));
    draw(cx);
}

#[gpui_kit::test]
fn shared_service_dialog_retains_native_editor_focus_and_draft_across_reflow(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    click_selector(cx, "service-row-0");
    click_selector(cx, "service-edit-local");
    assert_shared_dialog(cx);

    let name_input = service_name_input(cx);
    click_snapshot(cx, name_input);
    let editor_focus = cx.update(|window, cx| window.focused(cx).unwrap());
    let editor_path = service_name_input(cx).path().to_vec();
    cx.simulate_keystrokes("ctrl-a");
    let mut draft = "未保存的共享服务草稿".to_string();
    cx.simulate_input(&draft);
    draw(cx);
    assert_eq!(service_name_input(cx).value(), Some(draft.as_str()));

    for (compact, suffix) in [
        (true, " / compact"),
        (false, " / rail"),
        (true, " / retained"),
    ] {
        resize(cx, compact);
        assert_shared_dialog(cx);
        let input = service_name_input(cx);
        assert_eq!(
            input.path(),
            editor_path.as_slice(),
            "retain the real editor input"
        );
        assert_eq!(
            input.focused(),
            Some(true),
            "the dialog input must keep focus"
        );
        assert_eq!(
            input.value(),
            Some(draft.as_str()),
            "retain the unsaved native value"
        );
        cx.update(|window, cx| {
            assert_eq!(window.focused(cx).as_ref(), Some(&editor_focus));
            assert!(editor_focus.is_focused(window));
            assert!(
                !app.read(cx)
                    .settings_ui
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window),
                "settings search must not take an active dialog's editor focus"
            );
        });
        // Prove subsequent keyboard text still reaches this editor, not search.
        cx.simulate_input(suffix);
        draft.push_str(suffix);
        draw(cx);
        assert_eq!(service_name_input(cx).value(), Some(draft.as_str()));
    }
}

fn dialog_return_case(cx: &mut TestAppContext, start_compact: bool) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    if start_compact {
        resize(cx, true);
    }
    // Control: the same real dialog close and frame driver must restore Tab
    // navigation when its originating control remains in the current layout.
    tab_to(cx, navigation_target(start_compact));
    open_shared_dialog_from_navigation(&app, cx);
    let name_input = service_name_input(cx);
    click_snapshot(cx, name_input);
    assert_eq!(service_name_input(cx).focused(), Some(true));
    close_clean_dialog(cx);
    assert_layout(cx, start_compact);
    cx.simulate_keystrokes("tab");
    draw(cx);
    tab_to(cx, navigation_target(start_compact));

    // Keep only a weak handle, like Root's actual modal return target. Holding
    // the old Button alive here would manufacture an orphan instead of testing
    // what happens when the navigation control is genuinely unmounted.
    let original_focus = cx.update(|window, cx| {
        window
            .focused(cx)
            .expect("originating navigation must own focus")
            .downgrade()
    });
    open_shared_dialog_from_navigation(&app, cx);
    let name_input = service_name_input(cx);
    click_snapshot(cx, name_input);
    assert_eq!(service_name_input(cx).focused(), Some(true));
    resize(cx, !start_compact);
    assert_shared_dialog(cx);
    assert_eq!(service_name_input(cx).focused(), Some(true));
    // The temporary upgraded handle is dropped before close; diagnostics must
    // not change the lifecycle that the regression is intended to exercise.
    let origin_alive_after_reflow = original_focus.upgrade().is_some();
    close_clean_dialog(cx);
    assert_layout(cx, !start_compact);
    let focus_after_close = cx.update(|window, cx| format!("{:?}", window.focused(cx)));
    eprintln!(
        "dialog return (start_compact={start_compact}): no-reflow baseline passed; \
         origin handle alive after reflow={origin_alive_after_reflow}; \
         focus after close={focus_after_close}"
    );
    tab_to(cx, navigation_target(!start_compact));
}

#[gpui_kit::test]
fn rail_origin_dialog_close_keeps_compact_category_keyboard_reachable(cx: &mut TestAppContext) {
    dialog_return_case(cx, false);
}

#[gpui_kit::test]
fn compact_origin_dialog_close_keeps_rail_navigation_keyboard_reachable(cx: &mut TestAppContext) {
    dialog_return_case(cx, true);
}

#[gpui_kit::test]
fn compact_category_popup_dismissal_after_reflow_keeps_rail_keyboard_reachable(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (_, cx) = fixture(cx);
    resize(cx, true);
    tab_to(cx, navigation_target(true));
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(
        elements(cx).iter().any(|element| {
            element.visible()
                && element.role() == Some(Role::Menu)
                && element.focused() == Some(true)
        }),
        "the actual category popup must own keyboard focus before reflow"
    );

    resize(cx, false);
    // Removing the trigger may legitimately auto-dismiss its popup. Only send
    // Escape if it remains open; otherwise Escape would close Settings itself.
    if elements(cx)
        .iter()
        .any(|element| element.visible() && element.role() == Some(Role::Menu))
    {
        cx.simulate_keystrokes("escape");
        draw(cx);
    }
    assert!(
        !elements(cx)
            .iter()
            .any(|element| element.visible() && element.role() == Some(Role::Menu))
    );
    assert_layout(cx, false);
    tab_to(cx, navigation_target(false));
}
