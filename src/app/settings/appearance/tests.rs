use crate::{
    app::{
        ConduitApp,
        theme::{ThemeChoice, palette, theme_preview},
    },
    store::{AppearanceMode, Store},
};
use gpui_kit::{
    AppContext as _, SharedString, TestAppContext,
    component::{Theme, ThemeMode, searchable_list::SearchableVec, select::SelectEvent},
    px,
};

#[gpui_kit::test]
fn theme_selection_preserves_mode_fonts_drafts_and_restores_saved_choices(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "theme-selection");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence.flush();
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.ui.appearance = AppearanceMode::Dark;
            app.store.ui.ui_font_size = 18.;
            app.store.ui.code_font_size = 14.;
            app.input.update(cx, |input, cx| {
                input.set_value("retain this draft", window, cx)
            });
            app.apply_appearance(window, cx);
        });
    });
    // Drive the real select subscriptions, including a choice for the inactive mode.
    for (mode, name) in [
        (ThemeMode::Light, "Catppuccin Latte"),
        (ThemeMode::Dark, "Tokyo Night"),
        (ThemeMode::Dark, "Catppuccin Mocha"),
    ] {
        cx.update(|window, cx| {
            let state = if mode.is_dark() {
                app.read(cx).dark_theme_select.clone()
            } else {
                app.read(cx).light_theme_select.clone()
            };
            state.update(cx, |select, cx| {
                let name = SharedString::from(name);
                select.set_selected_value(&name, window, cx);
                cx.emit(SelectEvent::<SearchableVec<ThemeChoice>>::Confirm(Some(
                    name,
                )));
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let app = app.read(cx);
            let theme = Theme::global(cx);
            assert_eq!(app.store.ui.appearance, AppearanceMode::Dark);
            assert_eq!(theme.mode, ThemeMode::Dark);
            if mode.is_dark() {
                assert_eq!(app.store.ui.dark_theme, name);
                assert_eq!(theme.theme_name().as_ref(), name);
                assert_eq!(theme.highlight_theme.name, name);
            } else {
                assert_eq!(app.store.ui.light_theme, name);
                assert_eq!(theme.theme_name().as_ref(), "Conduit Dark");
            }
            assert_eq!(theme.font_size, px(18.));
            assert_eq!(theme.mono_font_size, px(14.));
            assert_eq!(
                palette().canvas,
                theme_preview(theme.theme_name(), theme.mode, cx).canvas
            );
            assert_eq!(app.input.read(cx).value().as_ref(), "retain this draft");
        });
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.appearance = AppearanceMode::System;
            app.apply_appearance(window, cx);
            let expected = if ThemeMode::from(window.appearance()).is_dark() {
                "Catppuccin Mocha"
            } else {
                "Catppuccin Latte"
            };
            assert_eq!(Theme::global(cx).theme_name().as_ref(), expected);
            app.store.ui.appearance = AppearanceMode::Light;
            app.apply_appearance(window, cx);
            assert_eq!(Theme::global(cx).theme_name().as_ref(), "Catppuccin Latte");
            // Read-only settings cannot alter the saved choice.
            app.writable = false;
            app.select_theme(ThemeMode::Light, "Ayu Light", window, cx);
            assert_eq!(app.store.ui.light_theme, "Catppuccin Latte");
            app.persistence.flush();
        });
    });
    let saved = cx.update(|_, cx| serde_json::to_vec(&app.read(cx).store).unwrap());
    cx.update(|window, cx| {
        let restored = cx.new(|cx| {
            ConduitApp::with_store(
                window,
                cx,
                Ok(serde_json::from_slice(&saved).unwrap()),
                false,
                false,
            )
        });
        assert_eq!(restored.read(cx).store.ui.dark_theme, "Catppuccin Mocha");
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "Catppuccin Latte");
        assert_eq!(Theme::global(cx).font_size, px(18.));
        // A removed name or a name from the wrong mode must not prevent startup.
        let mut store: Store = serde_json::from_slice(&saved).unwrap();
        store.ui.light_theme = "removed theme".into();
        store.ui.dark_theme = "Ayu Light".into();
        let fallback = cx.new(|cx| ConduitApp::with_store(window, cx, Ok(store), false, false));
        assert_eq!(fallback.read(cx).store.ui.light_theme, "Conduit Light");
        assert_eq!(fallback.read(cx).store.ui.dark_theme, "Conduit Dark");
        assert_eq!(Theme::global(cx).theme_name().as_ref(), "Conduit Light");
    });
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.run_until_parked();
}
