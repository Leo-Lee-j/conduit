use super::{Arc, AtomicBool, Category, ConduitApp, Ordering, Query, Sort, Store};
use conduit::i18n;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext, px, size};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

// Disclosure must not toggle the background or keep cancelled requests alive.
#[gpui_kit::test]
fn inline_browser_and_background_switch_have_independent_lifetimes(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(Store::default()), false, false);
            app.startup = None;
            app.settings_ui.open = true;
            app.settings_ui.page = 1;
            app.settings_ui.group = Some(5);
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(1100.), px(700.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.set_background_enabled(true, window, cx);
            assert!(app.store.ui.background.enabled);
            assert!(app.background.browser.is_none());
            assert!(app.background.operation.is_none());
            app.open_background_browser(window, cx);
            assert!(
                app.background
                    .browser
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .cancel
                    .is_none()
            );
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("wallpaper-search").is_some(),
        "inline form must render inside settings"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let browser = app.background.browser.clone().unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            browser.update(cx, |browser, _| browser.cancel = Some(cancel.clone()));
            app.close_background_browser(cx);
            assert!(cancel.load(Ordering::Acquire));
            assert!(!browser.read(cx).accepts(&cancel));
            assert!(app.background.browser.is_none());
            assert!(app.store.ui.background.enabled);
            app.open_background_browser(window, cx);
            app.set_background_enabled(false, window, cx);
            assert!(app.background.browser.is_some());
            assert!(!app.store.ui.background.enabled);
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.settings_ui.page = 0;
            app.settings_ui.group = None;
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.background.browser.is_none());
            app.writable = false;
            app.open_background_browser(window, cx);
            assert!(app.background.browser.is_none());
        })
    });
}

// Language changes must refresh retained controls and errors without losing a search.
#[gpui_kit::test]
fn language_switch_preserves_wallpaper_search_and_translates_retained_messages(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::scoped("en");
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut store = Store::default();
            store.ui.language = i18n::Locale::default();
            ConduitApp::with_store(window, cx, Ok(store), false, false)
        });
        app.update(cx, |app, cx| {
            app.open_background_browser(window, cx);
            let browser = app.background.browser.clone().unwrap();
            let input = browser.read(cx).input.clone();
            input.update(cx, |input, cx| {
                input.set_value("mountains 蓝色", window, cx)
            });
            browser.update(cx, |browser, _| {
                browser.category = Category::General;
                browser.sort = Sort::Random;
                browser.color = Some("0066cc".into());
                browser.solid_only = true;
                browser.query = Some(Query {
                    text: "mountains 蓝色".into(),
                    page: 2,
                    seed: Some("7Oo8tO".into()),
                    ..Default::default()
                });
                browser.last_page = 3;
                browser.error = Some(i18n::Message::new("wallpaper.wait_cancel", &[]));
            });
            let ticket = Arc::new(AtomicBool::new(false));
            app.background.operation = Some(ticket.clone());
            app.finish_background(
                "aaaaaa".into(),
                true,
                &ticket,
                Err(anyhow::anyhow!("raw failure")),
                cx,
            );
            let english_hint = input.read(cx).presentation().placeholder().to_string();
            let english_error = app.background.error.as_ref().unwrap().render();
            for language in [i18n::Locale::legacy(), i18n::Locale::default()] {
                app.select_language(language.clone(), window, cx);
                assert_eq!(app.background.browser.as_ref().unwrap(), &browser);
                let browser = browser.read(cx);
                assert_eq!(browser.input, input);
                assert_eq!(input.read(cx).value().as_str(), "mountains 蓝色");
                assert_eq!(
                    input.read(cx).presentation().placeholder().as_ref(),
                    i18n::text("wallpaper.query_hint")
                );
                assert!(browser.category == Category::General && browser.sort == Sort::Random);
                assert_eq!(browser.color.as_deref(), Some("0066cc"));
                assert!(browser.solid_only);
                assert_eq!(browser.query.as_ref().unwrap().page, 2);
                assert_eq!(
                    browser.query.as_ref().unwrap().seed.as_deref(),
                    Some("7Oo8tO")
                );
                assert_eq!(browser.last_page, 3);
                assert!(
                    browser.cancel.is_none(),
                    "language changes must not start requests"
                );
                assert_eq!(
                    browser.error.as_ref().unwrap().render(),
                    i18n::text("wallpaper.wait_cancel")
                );
                assert_eq!(
                    app.background.error.as_ref().unwrap().render(),
                    i18n::text("wallpaper.error.unknown")
                );
                if language == i18n::Locale::legacy() {
                    assert_ne!(
                        input.read(cx).presentation().placeholder().as_ref(),
                        english_hint
                    );
                    assert_ne!(
                        app.background.error.as_ref().unwrap().render(),
                        english_error
                    );
                }
                for query in ["背景图", "纯色", "background", "solid-color", "Wallhaven"] {
                    let result = app
                        .settings_search_matches(query)
                        .into_iter()
                        .find(|result| result.page == 1 && result.group == Some(5))
                        .expect("background settings must match both languages");
                    assert_eq!(result.title, i18n::text("wallpaper.title"));
                }
            }
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}
