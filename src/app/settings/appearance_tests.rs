use super::{AppearanceMode, ConduitApp, FontChoice, Store, TypographyField};
use crate::store::{DEFAULT_UI_FONT_SIZE, MAX_UI_FONT_SIZE, MIN_UI_FONT_SIZE};
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, SharedString, TestAppContext,
    VisualTestContext,
    component::{Theme, searchable_list::SearchableVec, select::SelectEvent},
    px, size,
};

fn fixture(cx: &mut TestAppContext, store: Store) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.connection_settings.syncing = true;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            // GPUI's NoopTextSystem has no installed fonts; inject selectable
            // families into the picker without changing production discovery.
            app.ui_font_select.update(cx, |select, cx| {
                select.set_items(
                    SearchableVec::new(vec![
                        FontChoice::default_entry("默认（系统界面）"),
                        FontChoice {
                            label: "Conduit Test UI".into(),
                            value: Some("Conduit Test UI".into()),
                        },
                    ]),
                    window,
                    cx,
                );
            });
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
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

fn confirm_ui_font(
    app: &Entity<ConduitApp>,
    family: Option<SharedString>,
    cx: &mut VisualTestContext,
) {
    cx.update(|window, cx| {
        let select = app.read(cx).ui_font_select.clone();
        select.update(cx, |select, cx| {
            select.set_selected_value(&family, window, cx);
            cx.emit(SelectEvent::Confirm(Some(family)));
        });
    });
    draw(cx);
}

#[gpui_kit::test]
fn settings_ui_font_is_independent_and_survives_theme_changes(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, Store::default());
    let (before, default_font, chosen) = cx.update(|_, cx| {
        let app = app.read(cx);
        let font = "Conduit Test UI";
        (
            app.chat_typography(),
            app.default_ui_font.clone(),
            SharedString::from(font),
        )
    });
    cx.update(|_, cx| app.update(cx, |app, _| app.writable = false));
    confirm_ui_font(&app, Some(chosen.clone()), cx);
    cx.update(|_, cx| {
        assert!(app.read(cx).store.ui.ui_font_family.is_none());
        assert_eq!(Theme::global(cx).font_family, default_font);
    });
    cx.update(|_, cx| app.update(cx, |app, _| app.writable = true));
    confirm_ui_font(&app, Some(chosen.clone()), cx);
    for mode in [
        AppearanceMode::Light,
        AppearanceMode::Dark,
        AppearanceMode::System,
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.appearance = mode;
                app.apply_appearance(window, cx);
                assert_eq!(
                    app.store.ui.ui_font_family.as_deref(),
                    Some(chosen.as_ref())
                );
                assert_eq!(Theme::global(cx).font_family, chosen);
                assert_eq!(
                    gpui_kit::base::Theme::global(cx).tokens.typography.sans,
                    chosen
                );
                assert_eq!(Theme::global(cx).mono_font_family, before.code_font);
                assert!(
                    app.chat_typography() == before,
                    "UI font must not change chat or code typography"
                );
            })
        });
        draw(cx);
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.chat_font_family = Some(default_font.to_string());
            app.store.ui.chat_font_size = 18.;
            app.store.ui.code_font_size = 16.;
            app.reset_typography(window, cx);
            assert_eq!(Theme::global(cx).font_family, chosen);
            assert!(app.chat_typography() == before);
            app.persistence.flush();
            assert!(app.persistence.error.is_none());
        })
    });
    confirm_ui_font(&app, None, cx);
    cx.update(|_, cx| {
        assert!(app.read(cx).store.ui.ui_font_family.is_none());
        assert_eq!(Theme::global(cx).font_family, default_font);
        assert!(app.read(cx).chat_typography() == before);
    });
}

#[gpui_kit::test]
fn settings_ui_font_missing_font_falls_back_to_the_original_default(cx: &mut TestAppContext) {
    let mut store = Store::default();
    store.ui.ui_font_family = Some("Conduit Missing UI Font 7D820ABF".into());
    let (app, cx) = fixture(cx, store);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.store.ui.ui_font_family.is_none());
        assert_eq!(app.ui_font_select.read(cx).selected_value(), Some(&None));
        assert_eq!(Theme::global(cx).font_family, app.default_ui_font);
        assert_eq!(
            app.chat_typography().body_font.as_ref(),
            Some(&app.default_ui_font)
        );
        assert!(app.store.ui.chat_font_family.is_none());
    });
}

#[gpui_kit::test]
fn settings_ui_font_search_picker_and_default_fit_the_existing_tab(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, Store::default());
    cx.simulate_keystrokes("ctrl-f");
    cx.simulate_input("界面字体");
    draw(cx);
    assert!(cx.debug_bounds("settings-search-result-0").is_some());
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(
        app.read_with(cx, |app, _| (app.settings_ui.page, app.settings_ui.group)),
        (1, Some(1))
    );
    let chosen = "Conduit Test UI".to_string();
    for (width, height, mode) in [
        (850., 500., AppearanceMode::Light),
        (1440., 900., AppearanceMode::Dark),
    ] {
        cx.simulate_resize(size(px(width), px(height)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.appearance = mode;
                app.apply_appearance(window, cx);
            })
        });
        draw(cx);
        assert!(cx.debug_bounds("settings-tab").is_some());
        let body = cx.debug_bounds("settings-workspace-body").unwrap();
        let picker = cx.debug_bounds("appearance-ui-font").unwrap();
        assert!(picker.left() >= body.left() && picker.right() <= body.right());
        assert!(picker.top() >= body.top() && picker.bottom() <= body.bottom());
        cx.simulate_mouse_down(picker.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(picker.center(), MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.simulate_input(&chosen);
        draw(cx);
        cx.simulate_keystrokes("enter");
        draw(cx);
        assert_eq!(
            app.read_with(cx, |app, _| app.store.ui.ui_font_family.clone()),
            Some(chosen.clone())
        );
        // Reopen, filter the default option, and confirm the reset using the keyboard.
        let picker = cx.debug_bounds("appearance-ui-font").unwrap();
        cx.simulate_mouse_down(picker.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(picker.center(), MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.simulate_input("默认");
        draw(cx);
        cx.simulate_keystrokes("enter");
        draw(cx);
        assert!(app.read_with(cx, |app, _| app.store.ui.ui_font_family.is_none()));
    }
    // Existing search destinations still open the correct typography groups.
    for (query, group, selector) in [
        ("聊天字体", 2, "appearance-chat-font"),
        ("代码块显示", 3, "appearance-code-font"),
    ] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let result = app
                    .settings_search_matches(query)
                    .into_iter()
                    .next()
                    .unwrap();
                assert_eq!(result.group, Some(group));
                app.open_settings_result(result, window, cx);
            })
        });
        draw(cx);
        assert!(cx.debug_bounds(selector).is_some());
    }
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let button = cx.debug_bounds(selector).expect(selector);
    cx.simulate_mouse_down(button.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(button.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn settings_ui_font_size_buttons_save_clamp_and_reset_without_touching_chat(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx, Store::default());
    cx.simulate_resize(size(px(1440.), px(900.)));
    let before = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.chat_font_family = Some("Conduit Test Chat".into());
            app.store.ui.chat_font_size = 17.;
            app.store.ui.chat_line_height = 1.7;
            app.store.ui.code_font_size = 14.;
            app.store.ui.code_line_numbers = true;
            app.apply_code_font(cx);
            let result = app
                .settings_search_matches("界面字号")
                .into_iter()
                .next()
                .unwrap();
            app.open_settings_result(result, window, cx);
            app.chat_typography()
        })
    });
    confirm_ui_font(&app, Some("Conduit Test UI".into()), cx);
    click(cx, "ui-font-size-plus");
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert_eq!(app.store.ui.ui_font_size, DEFAULT_UI_FONT_SIZE + 1.);
        assert_eq!(Theme::global(cx).font_size, px(DEFAULT_UI_FONT_SIZE + 1.));
        assert_eq!(window.rem_size(), px(DEFAULT_UI_FONT_SIZE + 1.));
        assert!(app.chat_typography() == before);
    });
    for _ in 0..6 {
        click(cx, "ui-font-size-plus");
    }
    assert_eq!(
        app.read_with(cx, |app, _| app.store.ui.ui_font_size),
        MAX_UI_FONT_SIZE
    );
    for _ in 0..10 {
        click(cx, "ui-font-size-minus");
    }
    assert_eq!(
        app.read_with(cx, |app, _| app.store.ui.ui_font_size),
        MIN_UI_FONT_SIZE
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            for (writable, closing) in [(false, false), (true, true)] {
                app.writable = writable;
                app.closing = closing;
                app.adjust_typography(TypographyField::UiSize, 1., cx);
                app.reset_ui_typography(window, cx);
                assert_eq!(app.store.ui.ui_font_size, MIN_UI_FONT_SIZE);
                assert_eq!(
                    app.store.ui.ui_font_family.as_deref(),
                    Some("Conduit Test UI")
                );
            }
            app.writable = true;
            app.closing = false;
            for mode in [
                AppearanceMode::Light,
                AppearanceMode::Dark,
                AppearanceMode::System,
            ] {
                app.store.ui.appearance = mode;
                app.apply_appearance(window, cx);
                assert_eq!(Theme::global(cx).font_size, px(MIN_UI_FONT_SIZE));
                assert_eq!(Theme::global(cx).mono_font_size, px(before.code_size));
                assert!(app.chat_typography() == before);
            }
            let restored: Store =
                serde_json::from_value(serde_json::to_value(&app.store).unwrap()).unwrap();
            assert_eq!(restored.ui.ui_font_size, MIN_UI_FONT_SIZE);
            app.reset_typography(window, cx);
            assert_eq!(app.store.ui.ui_font_size, MIN_UI_FONT_SIZE);
            assert_eq!(Theme::global(cx).font_size, px(MIN_UI_FONT_SIZE));
            // Restore the chat options so the interface reset can prove it preserves them.
            app.store.ui.chat_font_family = before.body_font.as_ref().map(ToString::to_string);
            app.store.ui.chat_font_size = before.body_size;
            app.store.ui.chat_line_height = before.line_height;
            app.store.ui.code_font_size = before.code_size;
            app.store.ui.code_line_numbers = before.code_line_numbers;
            app.apply_code_font(cx);
        })
    });
    draw(cx);
    click(cx, "appearance-ui-reset");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.store.ui.ui_font_size, DEFAULT_UI_FONT_SIZE);
            assert!(app.store.ui.ui_font_family.is_none());
            assert_eq!(app.ui_font_select.read(cx).selected_value(), Some(&None));
            assert_eq!(Theme::global(cx).font_size, px(DEFAULT_UI_FONT_SIZE));
            assert_eq!(window.rem_size(), px(DEFAULT_UI_FONT_SIZE));
            assert!(app.chat_typography() == before);
            app.persistence.flush();
            assert!(app.persistence.error.is_none());
        })
    });
}

#[gpui_kit::test]
fn settings_ui_font_size_restores_saved_values_and_clamps_old_configuration(
    cx: &mut TestAppContext,
) {
    let mut store = Store::default();
    store.ui.ui_font_size = 18.;
    let (app, cx) = fixture(cx, store);
    cx.update(|window, cx| {
        assert_eq!(Theme::global(cx).font_size, px(18.));
        assert_eq!(window.rem_size(), px(18.));
        assert_eq!(app.read(cx).store.ui.ui_font_size, 18.);
        assert_eq!(app.read(cx).chat_typography().body_size, 15.);
    });
    for (stored, expected) in [(2., MIN_UI_FONT_SIZE), (128., MAX_UI_FONT_SIZE)] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let mut store = Store::default();
                store.ui.ui_font_size = stored;
                *app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.startup = None;
                app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
                assert_eq!(app.store.ui.ui_font_size, expected);
                assert_eq!(Theme::global(cx).font_size, px(expected));
                assert_eq!(app.chat_typography().body_size, 15.);
            })
        });
        draw(cx);
        cx.update(|window, _| assert_eq!(window.rem_size(), px(expected)));
    }
}
