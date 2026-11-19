use crate::app::{ConduitApp, ModelProvider, Provider, ProviderSource, Store, palette};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement, Render, Role, Styled, Subscription, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, assets::IconName as FullIconName, component::ThemeMode, div,
    point, px, rgb, size,
};
use std::time::Duration;

struct PetSettings {
    app: Entity<ConduitApp>,
    _subscription: Subscription,
}

impl PetSettings {
    fn new(app: Entity<ConduitApp>, cx: &mut Context<Self>) -> Self {
        Self {
            _subscription: cx.observe(&app, |_, _, cx| cx.notify()),
            app,
        }
    }
}

impl Render for PetSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(16.))
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .child(
                self.app
                    .read(cx)
                    .settings_encourager_content(self.app.clone()),
            )
    }
}

fn sample_store(configured: bool) -> Store {
    let mut store = Store::default();
    store.ui.encourager.enabled = true;
    store.providers.clear();
    if configured {
        store.providers.push(ModelProvider {
            id: "pet-test".into(),
            name: "Development API".into(),
            engine: Provider::Codex,
            base_url: "https://example.invalid/v1".into(),
            api_key_env: String::new(),
            protected_api_key: None,
            models: vec!["sample-chat-model".into(), "sample-chat-model-2".into()],
            source: ProviderSource::Local,
            model_metadata: vec![],
            claude_models: None,
        });
        store.ui.encourager.provider_id = Some("pet-test".into());
        store.ui.encourager.model = Some("sample-chat-model".into());
    }
    store
}

fn create_app(window: &mut Window, cx: &mut App, configured: bool) -> Entity<ConduitApp> {
    cx.new(|cx| {
        let mut app =
            ConduitApp::with_store(window, cx, Ok(sample_store(configured)), false, false);
        app.startup = None;
        app.writable = true;
        app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
        app
    })
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        let _ = window.draw(cx);
    });
}

fn click(cx: &mut VisualTestContext, role: Role, label: &str) {
    let center = cx.update(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|element| {
                element.visible() && element.role() == Some(role) && element.label() == Some(label)
            })
            .expect(label)
            .bounds()
            .center()
    });
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn desktop_pet_settings_fit_and_switches_preserve_configuration(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = create_app(window, cx, true);
        *output.borrow_mut() = Some(app.clone());
        let content = cx.new(|cx| PetSettings::new(app, cx));
        gpui_kit::component::Root::new(content, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    for width in [1100., 420.] {
        cx.simulate_resize(size(px(width), px(900.)));
        draw(cx);
        let card = cx.debug_bounds("programmer-encourager-settings").unwrap();
        assert!(card.size.width <= px(760.));
        assert!(card.right() <= px(width));
        for id in ["encourager-pet-row", "encourager-context-row"] {
            assert_eq!(
                cx.debug_bounds(id).expect(id).size.height,
                px(52.),
                "{id} must keep its switch beside its label"
            );
        }
        let mut left = None;
        for id in [
            "encourager-provider",
            "encourager-model",
            "encourager-persona-setting",
            "encourager-verbosity",
        ] {
            let control = cx.debug_bounds(id).expect(id);
            assert!(
                control.size.width <= px(288.),
                "{id} must have a readable width"
            );
            assert!(control.right() <= card.right(), "{id} must fit in the card");
            assert_eq!(control.size.height, px(32.));
            assert_eq!(*left.get_or_insert(control.left()), control.left());
        }
    }
    assert!(cx.debug_bounds("encourager-service-hint").is_none());
    click(cx, Role::Switch, "参考当前会话最近一轮");
    cx.update(|_, cx| assert!(app.read(cx).store.ui.encourager.include_context));
    click(cx, Role::Switch, "启用宠物功能");
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.store.ui.encourager.enabled);
        assert_eq!(
            app.store.ui.encourager.provider_id.as_deref(),
            Some("pet-test")
        );
        assert!(app.store.ui.encourager.include_context);
    });
    // The disabled pet switch must not create a window.
    click(cx, Role::Switch, "显示悬浮宠物");
    cx.update(|_, cx| assert!(app.read(cx).encourager_pet_window.is_none()));
    click(cx, Role::Switch, "启用宠物功能");
    cx.update(|_, cx| {
        let app = app.read(cx);
        let choice = app
            .plugins
            .right_choices(false)
            .into_iter()
            .find(|choice| choice.id.as_str() == conduit::plugins::builtins::ENCOURAGER_VIEW)
            .unwrap();
        assert_eq!(choice.icon, FullIconName::Cat);
        assert!(choice.request.is_some());
    });
    cx.update(|_, cx| app.update(cx, |app, _| app.persistence.flush()));
}

// Selecting an existing service must populate models and persist subsequent choices.
#[gpui_kit::test]
fn desktop_pet_selects_existing_provider_and_model(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = create_app(window, cx, true);
        app.update(cx, |app, _| {
            let mut imported = app.store.providers[0].clone();
            imported.id = "ccs:codex:pet-imported".into();
            imported.name = "Imported API".into();
            imported.models = vec!["imported-model".into()];
            imported.source = ProviderSource::Ccs {
                id: "pet-imported".into(),
            };
            app.store.providers.push(imported);
            app.store.ui.encourager.provider_id = None;
            app.store.ui.encourager.model = None;
        });
        *output.borrow_mut() = Some(app.clone());
        let content = cx.new(|cx| PetSettings::new(app, cx));
        gpui_kit::component::Root::new(content, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(1100.), px(900.)));
    draw(cx);
    click(
        cx,
        Role::Button,
        conduit::i18n::text_static("encourager.setting.service"),
    );
    click(cx, Role::MenuItem, "Development API");
    cx.update(|_, cx| {
        let settings = &app.read(cx).store.ui.encourager;
        assert_eq!(settings.provider_id.as_deref(), Some("pet-test"));
        assert_eq!(settings.model.as_deref(), Some("sample-chat-model"));
    });
    click(
        cx,
        Role::Button,
        conduit::i18n::text_static("encourager.setting.model"),
    );
    click(cx, Role::MenuItem, "sample-chat-model-2");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).store.ui.encourager.model.as_deref(),
            Some("sample-chat-model-2")
        );
    });
    // Reopening the same service must preserve the explicitly selected model.
    click(
        cx,
        Role::Button,
        conduit::i18n::text_static("encourager.setting.service"),
    );
    click(cx, Role::MenuItem, "Development API");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).store.ui.encourager.model.as_deref(),
            Some("sample-chat-model-2")
        )
    });
    click(
        cx,
        Role::Button,
        conduit::i18n::text_static("encourager.setting.service"),
    );
    click(cx, Role::MenuItem, "Imported API");
    cx.update(|_, cx| {
        let settings = &app.read(cx).store.ui.encourager;
        assert_eq!(
            settings.provider_id.as_deref(),
            Some("ccs:codex:pet-imported")
        );
        assert_eq!(settings.model.as_deref(), Some("imported-model"));
    });
    click(
        cx,
        Role::Button,
        conduit::i18n::text_static("encourager.setting.model"),
    );
    click(cx, Role::MenuItem, "imported-model");
    click(
        cx,
        Role::Button,
        conduit::i18n::text_static("encourager.setting.service"),
    );
    click(cx, Role::MenuItem, "Development API");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).store.ui.encourager.model.as_deref(),
            Some("sample-chat-model")
        );
        app.update(cx, |app, _| app.persistence.flush());
    });
}

/// Manually run on Windows to inspect the actual shared form with the GPU:
/// cargo test --features ui-tests --bin conduit desktop_pet_settings_native_snapshots -- --ignored --nocapture --test-threads=1
#[test]
#[ignore = "requires a real Windows desktop and GPU"]
fn desktop_pet_settings_native_snapshots() {
    let _locale = crate::test_support::chinese();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            crate::app::install_theme_packs(cx);
            cx.spawn(async move |cx| {
                for (name, width, height, dark, configured) in [
                    ("dark-empty", 1000., 620., true, false),
                    ("dark-configured", 1000., 620., true, true),
                    ("light-configured", 1000., 620., false, true),
                    ("dark-narrow", 420., 800., true, true),
                ] {
                    let handle = cx.update(|cx| {
                        cx.open_window(
                            WindowOptions {
                                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                    point(px(100.), px(100.)),
                                    size(px(width), px(height)),
                                ))),
                                focus: false,
                                ..Default::default()
                            },
                            |window, cx| {
                                let app = create_app(window, cx, configured);
                                // App initialization applies the stored theme;
                                // select this preview's mode afterward.
                                let ui = Store::default().ui;
                                crate::app::apply_theme(
                                    if dark {
                                        ThemeMode::Dark
                                    } else {
                                        ThemeMode::Light
                                    },
                                    &ui.light_theme,
                                    &ui.dark_theme,
                                    cx,
                                );
                                let view = cx.new(|cx| PetSettings::new(app, cx));
                                cx.new(|cx| gpui_kit::component::Root::new(view, window, cx))
                            },
                        )
                        .unwrap()
                    });
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    let saved = std::rc::Rc::new(std::cell::Cell::new(false));
                    let completed = saved.clone();
                    handle
                        .update(cx, |_, window, cx| {
                            // Rendering traverses Root again; release its typed
                            // update lease before drawing the shared settings view.
                            window.defer(cx, move |window, cx| {
                                assert_eq!(
                                    gpui_kit::component::Theme::global(cx).mode.is_dark(),
                                    dark
                                );
                                window.refresh();
                                window.draw(cx).clear(cx);
                                let image = window.render_to_image().unwrap();
                                std::fs::create_dir_all("target/pet-settings").unwrap();
                                image
                                    .save(format!("target/pet-settings/{name}.png"))
                                    .unwrap();
                                println!("saved {name}: {}x{}", image.width(), image.height());
                                completed.set(true);
                            });
                        })
                        .unwrap();
                    // Closing the only window quits GPUI before the next case.
                    // Keep these preview windows alive until every save finishes.
                    cx.background_executor()
                        .timer(Duration::from_millis(50))
                        .await;
                    assert!(saved.get(), "snapshot {name} was not saved");
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
}
