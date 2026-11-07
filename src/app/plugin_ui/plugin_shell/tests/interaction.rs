//! Cross-root input and modal behavior using the real desktop composition.
use super::*;
use gpui_kit::{ClipboardItem, EntityInputHandler, FocusHandle};

fn focused(cx: &mut VisualTestContext) -> FocusHandle {
    cx.update(|window, cx| window.focused(cx).expect("keyboard focus"))
}

fn open_summary(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    command(app, "example.workspace-overview.increment", cx);
    // Registered components expose their identity through native semantics,
    // while the script snapshot only prints the component type.
    let _ = text(app, true, cx);
}

fn layers(cx: &mut VisualTestContext) -> (bool, usize, bool) {
    cx.update(|window, cx| {
        let host = window.has_active_dialog(cx);
        gpui_shell::ShellRoot::update(window, cx, |root, _, _| {
            (host, root.dialog_count(), root.sheet().is_some())
        })
        .unwrap()
    })
}

#[gpui_kit::test]
fn desktop_composition_survives_plugin_command_and_commits_once(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    open_summary(&app, cx);
    let input = cx.update(|window, cx| {
        let input = app.read(cx).input.clone();
        input.read(cx).focus_handle(cx).focus(window, cx);
        input.update(cx, |input, cx| {
            input.set_value("前😀", window, cx);
            input.set_selected_range("前😀".len().."前😀".len(), cx);
            input.replace_and_mark_text_in_range(None, "n", Some(1..1), window, cx);
            input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
            assert_eq!(input.marked_text_range(window, cx), Some(3..5));
        });
        input
    });
    draw(cx);
    command(&app, "example.workspace-overview.increment", cx);
    cx.update(|window, cx| {
        assert!(input.read(cx).focus_handle(cx).is_focused(window));
        input.update(cx, |input, cx| {
            assert_eq!(input.marked_text_range(window, cx), Some(3..5));
            input.replace_text_in_range(None, "你", window, cx);
            assert_eq!(input.value().as_str(), "前😀你");
            assert_eq!(input.marked_text_range(window, cx), None);
        });
    });
    draw(cx);
    cx.simulate_keystrokes("ctrl-z");
    draw(cx);
    cx.update(|_, cx| assert_eq!(input.read(cx).value().as_str(), "前😀"));
    // A canceled composition must not swallow the next ordinary edit.
    cx.update(|window, cx| {
        input.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "shi", Some(3..3), window, cx);
            input.replace_text_in_range(None, "", window, cx);
            input.unmark_text(window, cx);
        })
    });
    draw(cx);
    cx.simulate_input("好");
    draw(cx);
    cx.update(|_, cx| assert_eq!(input.read(cx).value().as_str(), "前😀好"));
    assert!(text(&app, true, cx).contains("Command count: 2"));
}

#[gpui_kit::test]
fn desktop_input_editing_and_theme_keep_plugin_state(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    open_summary(&app, cx);
    click(cx, "plugin-summary-input");
    cx.simulate_input("中文😀 input");
    draw(cx);
    assert!(text(&app, true, cx).contains("Input: 中文😀 input"));
    let input = focused(cx);
    cx.simulate_keystrokes("ctrl-a");
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("粘贴🦀".into())));
    cx.simulate_keystrokes("ctrl-v");
    draw(cx);
    assert!(text(&app, true, cx).contains("Input: 粘贴🦀"));
    for (mode, label) in [
        (crate::store::AppearanceMode::Light, "light"),
        (crate::store::AppearanceMode::Dark, "dark"),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.appearance = mode;
                app.apply_appearance(window, cx);
            });
        });
        draw(cx);
        draw(cx);
        let snapshot = text(&app, true, cx);
        assert!(snapshot.contains(&format!("Theme: {label}")), "{snapshot}");
        assert!(snapshot.contains("Input: 粘贴🦀"));
        assert!(snapshot.contains("Command count: 1"));
        cx.update(|window, _| assert!(input.is_focused(window)));
    }
}

#[gpui_kit::test]
fn desktop_host_and_script_modals_unwind_one_layer_and_restore_focus(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    open_summary(&app, cx);
    click(cx, "plugin-summary-input");
    cx.simulate_input("保留草稿");
    draw(cx);
    let summary_input = focused(cx);
    command(&app, "example.workspace-overview.overlays", cx);
    assert_eq!(layers(cx), (false, 1, true));
    click(cx, "plugin-dialog-input");
    cx.simulate_input("弹窗草稿");
    draw(cx);
    let dialog_input = focused(cx);
    for _ in 0..4 {
        cx.simulate_keystrokes("tab");
        draw(cx);
        assert_ne!(focused(cx), summary_input);
    }
    click(cx, "plugin-dialog-input");
    cx.update(|window, cx| {
        // Use the same component Root as native forms. Settings itself may be
        // a retained workspace rather than a modal destination.
        window.open_dialog(cx, |dialog, _, _| {
            use gpui_kit::ParentElement;
            dialog
                .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                .title("Host dialog")
                .child(
                    gpui_kit::component::button::Button::new("g4-host-button").label("Host action"),
                )
        });
    });
    draw(cx);
    assert_eq!(layers(cx), (true, 1, true));
    for _ in 0..12 {
        cx.simulate_keystrokes("tab");
        draw(cx);
        assert_ne!(focused(cx), dialog_input);
        assert_ne!(focused(cx), summary_input);
    }
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(layers(cx), (false, 1, true));
    cx.update(|window, _| assert!(dialog_input.is_focused(window)));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(layers(cx), (false, 0, true));
    click(cx, "plugin-sheet-input");
    cx.simulate_input("侧栏草稿");
    draw(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(layers(cx), (false, 0, false));
    cx.update(|window, _| assert!(summary_input.is_focused(window)));
    assert!(text(&app, true, cx).contains("Input: 保留草稿"));
}

#[gpui_kit::test]
fn desktop_settings_and_narrow_dock_preserve_plugin_input(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    open_summary(&app, cx);
    click(cx, "plugin-summary-input");
    cx.simulate_input("窄窗草稿");
    draw(cx);
    let input = focused(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    assert!(cx.element_bounds("settings-workspace").is_some());
    // Reopening an already selected settings tab must not replace its return target.
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.element_bounds("settings-workspace").is_none());
    cx.update(|window, _| assert!(input.is_focused(window)));
    for width in [850., 1100., 1600., 850.] {
        cx.simulate_resize(size(px(width), px(700.)));
        draw(cx);
        draw(cx);
        let dock = cx.element_bounds("detail-panel").unwrap();
        let toolbar = cx.element_bounds("conversation-toolbar").unwrap();
        let field = cx.update(|window, _| {
            gpui_kit::base::test_support::find(window, &[], &"plugin-summary-input".into())
                .unwrap()
                .bounds()
        });
        assert!(field.left() >= dock.left() && field.right() <= dock.right());
        assert!(dock.right() <= px(width));
        if width < 1100. {
            // The narrow dock intentionally overlays the conversation, while
            // the bottom toolbar remains reachable outside that overlay.
            assert!(dock.bottom() <= toolbar.top());
        }
        assert!(text(&app, true, cx).contains("Input: 窄窗草稿"));
    }
    cx.update(|window, cx| app.update(cx, |app, cx| app.close_right_view(window, cx)));
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    let toolbar = cx.element_bounds("conversation-toolbar").unwrap();
    assert!(toolbar.right() <= px(850.));
    open_summary(&app, cx);
    assert!(text(&app, true, cx).contains("Input: 窄窗草稿"));
}

#[gpui_kit::test]
fn desktop_settings_does_not_restore_a_stopped_plugin_input(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    open_summary(&app, cx);
    click(cx, "plugin-summary-input");
    let old_focus = focused(cx).downgrade();
    let weak_runtime = runtime(&app, cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_settings(window, cx);
            app.disable_shell(window, cx);
        })
    });
    draw(cx);
    draw(cx);
    assert!(weak_runtime.upgrade().is_none());
    assert!(old_focus.upgrade().is_none());
    cx.update(|window, cx| app.update(cx, |app, cx| app.close_settings(window, cx)));
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            app.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        )
    });
}
