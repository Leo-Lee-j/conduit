use crate::{
    app::{ConduitApp, settings::TypographyField},
    store::{
        AppearanceMode, MAX_CHAT_FONT_SIZE, MAX_CHAT_LINE_HEIGHT, MAX_CODE_FONT_SIZE,
        MAX_UI_FONT_SIZE, UiIconSize, UiSettings,
    },
};
use gpui_kit::{Context, TestAppContext, Window, component::Theme, px};

fn customize_typography(app: &mut ConduitApp, window: &mut Window, cx: &mut Context<ConduitApp>) {
    app.persistence.flush();
    app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
    app.store.ui.appearance = AppearanceMode::Dark;
    app.store.ui.light_theme = "Catppuccin Latte".into();
    app.store.ui.dark_theme = "Tokyo Night".into();
    app.store.ui.auto_sort_workspaces = false;
    app.store.ui.auto_collapse_inactive_workspaces = true;
    app.store.ui.background.strength = 34;
    app.store.ui.ui_font_family = Some(app.default_ui_font.to_string());
    app.store.ui.ui_font_size = 19.;
    app.store.ui.chat_font_family = Some(app.default_ui_font.to_string());
    app.store.ui.chat_font_size = 17.;
    app.store.ui.chat_line_height = 1.8;
    app.store.ui.code_font_family = Some(app.default_mono_font.to_string());
    app.store.ui.code_font_size = 16.;
    app.store.ui.code_copy_button = false;
    app.store.ui.code_syntax_highlight = false;
    app.store.ui.code_line_numbers = true;
    app.input.update(cx, |input, cx| {
        input.set_value("Keep this draft while changing fonts", window, cx);
        input.set_selected_range(5..15, cx);
    });
    app.apply_appearance(window, cx);
}

fn editor_and_font_entities(app: &ConduitApp) -> [gpui_kit::EntityId; 4] {
    [
        app.input.entity_id(),
        app.ui_font_select.entity_id(),
        app.chat_font_select.entity_id(),
        app.code_font_select.entity_id(),
    ]
}

#[gpui_kit::test]
fn icon_size_updates_live_and_is_restored_after_theme_changes(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-icon-size");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.set_ui_icon_size(UiIconSize::Large, cx);
            assert_eq!(app.store.ui.icon_size, UiIconSize::Large);
            assert_eq!(Theme::global(cx).icon_scale, 1.2);

            app.store.ui.appearance = AppearanceMode::Light;
            app.apply_appearance(window, cx);
            assert_eq!(Theme::global(cx).icon_scale, 1.2);

            app.set_ui_icon_size(UiIconSize::Compact, cx);
            assert_eq!(Theme::global(cx).icon_scale, 0.8);
            app.persistence.flush();
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn interface_reset_changes_only_interface_preferences_and_keeps_editor_state(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-interface-reset");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            customize_typography(app, window, cx);
            let entities = editor_and_font_entities(app);
            let draft = app.input.read(cx).value().to_string();
            let selection = app.input.read(cx).selected_range();
            let mut expected = serde_json::to_value(&app.store).unwrap();
            let defaults = serde_json::to_value(UiSettings::default()).unwrap();
            for key in ["ui_font_family", "ui_font_size"] {
                expected["ui"][key] = defaults[key].clone();
            }

            app.reset_ui_typography(window, cx);

            // Compare the entire store, including themes, workspace switches,
            // background, sessions and every chat/code preference.
            assert_eq!(serde_json::to_value(&app.store).unwrap(), expected);
            assert_eq!(editor_and_font_entities(app), entities);
            assert_eq!(app.input.read(cx).value().as_ref(), draft);
            assert_eq!(app.input.read(cx).selected_range(), selection);
            assert_eq!(Theme::global(cx).font_family, app.default_ui_font);
            assert_eq!(
                Theme::global(cx).font_size,
                px(UiSettings::default().ui_font_size)
            );
            assert_eq!(Theme::global(cx).mono_font_size, px(16.));

            // This is a navigation/search contract, not a screenshot assertion.
            let page = app.settings_appearance_page(false, cx.entity());
            assert_eq!(page.groups.len(), 3);
            app.persistence.flush();
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn chat_and_code_reset_preserves_interface_theme_background_and_editor_state(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-chat-code-reset");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            customize_typography(app, window, cx);
            let entities = editor_and_font_entities(app);
            let selection = app.input.read(cx).selected_range();
            let mut expected = serde_json::to_value(&app.store).unwrap();
            let defaults = serde_json::to_value(UiSettings::default()).unwrap();
            for key in [
                "chat_font_family",
                "chat_font_size",
                "chat_line_height",
                "code_font_family",
                "code_font_size",
                "code_copy_button",
                "code_syntax_highlight",
                "code_line_numbers",
            ] {
                expected["ui"][key] = defaults[key].clone();
            }

            app.reset_typography(window, cx);

            assert_eq!(serde_json::to_value(&app.store).unwrap(), expected);
            assert_eq!(editor_and_font_entities(app), entities);
            assert_eq!(app.input.read(cx).selected_range(), selection);
            assert_eq!(
                app.input.read(cx).value().as_ref(),
                "Keep this draft while changing fonts"
            );
            assert_eq!(Theme::global(cx).font_size, px(19.));
            assert_eq!(Theme::global(cx).mono_font_family, app.default_mono_font);
            assert_eq!(
                Theme::global(cx).mono_font_size,
                px(UiSettings::default().code_font_size)
            );
            app.persistence.flush();
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn readonly_or_closing_typography_actions_do_not_mutate_preferences_or_editor_state(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-disabled-actions");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            customize_typography(app, window, cx);
            let expected = serde_json::to_value(&app.store).unwrap();
            let entities = editor_and_font_entities(app);
            let selection = app.input.read(cx).selected_range();
            let ui_size = Theme::global(cx).font_size;
            let code_size = Theme::global(cx).mono_font_size;

            for (writable, closing) in [(false, false), (true, true)] {
                app.writable = writable;
                app.closing = closing;
                app.reset_ui_typography(window, cx);
                app.reset_typography(window, cx);
                for field in [
                    TypographyField::UiSize,
                    TypographyField::ChatSize,
                    TypographyField::ChatLineHeight,
                    TypographyField::CodeSize,
                ] {
                    app.adjust_typography(field, 1., cx);
                }
                assert_eq!(serde_json::to_value(&app.store).unwrap(), expected);
                assert_eq!(editor_and_font_entities(app), entities);
                assert_eq!(app.input.read(cx).selected_range(), selection);
                assert_eq!(Theme::global(cx).font_size, ui_size);
                assert_eq!(Theme::global(cx).mono_font_size, code_size);
            }
            app.writable = true;
            app.closing = false;
            app.persistence.flush();
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn typography_steps_keep_interface_chat_and_code_independent_and_bounded(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "appearance-independent-steps");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            customize_typography(app, window, cx);
            let entities = editor_and_font_entities(app);
            let selection = app.input.read(cx).selected_range();

            app.adjust_typography(TypographyField::UiSize, 100., cx);
            assert_eq!(app.store.ui.ui_font_size, MAX_UI_FONT_SIZE);
            assert_eq!(app.store.ui.chat_font_size, 17.);
            assert_eq!(app.store.ui.chat_line_height, 1.8);
            assert_eq!(app.store.ui.code_font_size, 16.);

            app.adjust_typography(TypographyField::CodeSize, 100., cx);
            assert_eq!(app.store.ui.code_font_size, MAX_CODE_FONT_SIZE);
            assert_eq!(app.store.ui.ui_font_size, MAX_UI_FONT_SIZE);
            assert_eq!(app.store.ui.chat_font_size, 17.);

            app.adjust_typography(TypographyField::ChatSize, 100., cx);
            assert_eq!(app.store.ui.chat_font_size, MAX_CHAT_FONT_SIZE);
            assert_eq!(app.store.ui.chat_line_height, 1.8);
            assert_eq!(app.store.ui.ui_font_size, MAX_UI_FONT_SIZE);
            assert_eq!(app.store.ui.code_font_size, MAX_CODE_FONT_SIZE);

            app.adjust_typography(TypographyField::ChatLineHeight, 100., cx);
            assert_eq!(app.store.ui.chat_line_height, MAX_CHAT_LINE_HEIGHT);
            assert_eq!(app.store.ui.chat_font_size, MAX_CHAT_FONT_SIZE);
            assert_eq!(app.store.ui.ui_font_size, MAX_UI_FONT_SIZE);
            assert_eq!(app.store.ui.code_font_size, MAX_CODE_FONT_SIZE);
            assert_eq!(Theme::global(cx).font_size, px(MAX_UI_FONT_SIZE));
            assert_eq!(Theme::global(cx).mono_font_size, px(MAX_CODE_FONT_SIZE));
            assert_eq!(editor_and_font_entities(app), entities);
            assert_eq!(app.input.read(cx).selected_range(), selection);
            app.persistence.flush();
        });
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}
