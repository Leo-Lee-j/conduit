use gpui_kit::{AppContext as _, TestAppContext, point, px};

fn fixture(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::Entity<super::ConduitApp>,
    gpui_kit::VisualTestContext,
    gpui_kit::Entity<super::PetView>,
) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, host) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app =
                super::ConduitApp::with_store(window, cx, Ok(pet_store(false)), false, false);
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.providers.clear();
            app
        });
        *output.borrow_mut() = Some(app.clone());
        app.update(cx, |app, cx| app.set_encourager_pet_enabled(true, cx));
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    host.run_until_parked();
    let handle = host.update(|_, cx| app.read(cx).encourager_pet_window.unwrap());
    let pet = cx.update(|cx| {
        handle
            .downcast::<super::PetView>()
            .unwrap()
            .root(cx)
            .unwrap()
    });
    (
        app,
        gpui_kit::VisualTestContext::from_window(handle, cx),
        pet,
    )
}

fn pet_store(pet_enabled: bool) -> crate::store::Store {
    let mut store = crate::store::Store::default();
    store.ui.encourager.enabled = true;
    store.ui.encourager.pet_enabled = pet_enabled;
    store
}

#[gpui_kit::test]
fn desktop_pet_is_transparent_and_clicks_think_before_feedback(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, MouseButton, VisualTestContext};
    use std::time::Duration;
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, host) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app =
                super::ConduitApp::with_store(window, cx, Ok(pet_store(false)), false, false);
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.store.providers.clear();
            app
        });
        *output.borrow_mut() = Some(app.clone());
        app.update(cx, |app, cx| app.set_encourager_pet_enabled(true, cx));
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    host.run_until_parked();
    let handle = host.update(|_, cx| app.read(cx).encourager_pet_window.unwrap());
    let mut pet = VisualTestContext::from_window(handle, cx);
    pet.update(|window, cx| {
        let _ = window.draw(cx);
        assert!(
            window.painted_quads().len() >= 20,
            "pixel pet must paint its sprite quads"
        );
        assert!(
            window.painted_quads().iter().all(|q| q.bounds.size.height
                <= gpui_kit::ScaledPixels(
                    f32::from(super::cell_size(window.scale_factor())) * window.scale_factor()
                        + 0.01
                )),
            "a compact pet must contain only pixel rows, no window-sized background"
        );
    });
    assert!(pet.debug_bounds("desktop-pet-character").is_some());
    assert!(pet.debug_bounds("desktop-pet-bubble").is_none());
    let center = pet.debug_bounds("desktop-pet-character").unwrap().center();

    // A drag must not be interpreted as a chat request on mouse-up.
    let generation = pet.update(|_, cx| app.read(cx).encourager.generation);
    pet.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    pet.simulate_mouse_move(
        center + point(px(12.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    pet.simulate_mouse_up(
        center + point(px(12.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    pet.update(|_, cx| assert_eq!(app.read(cx).encourager.generation, generation));

    pet.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    pet.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    pet.run_until_parked();
    pet.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(pet.debug_bounds("desktop-pet-thinking").is_some());
    assert!(
        pet.debug_bounds("desktop-pet-bubble").is_none(),
        "even immediate validation feedback follows the thinking pose"
    );
    let generation = pet.update(|_, cx| app.read(cx).encourager.generation);
    pet.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    pet.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    pet.update(|_, cx| assert_eq!(app.read(cx).encourager.generation, generation));

    for _ in 0..4 {
        pet.executor().advance_clock(Duration::from_millis(180));
        pet.run_until_parked();
    }
    pet.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(pet.debug_bounds("desktop-pet-thinking").is_none());
    assert!(pet.debug_bounds("desktop-pet-bubble").is_some());
    pet.update(|_, cx| {
        assert!(
            app.read(cx).encourager.error.is_some(),
            "missing configuration must produce actionable feedback"
        )
    });

    // A slow request keeps thinking until the host reports its completion.
    pet.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.encourager.error = None;
            app.encourager.loading = true;
            cx.notify();
        })
    });
    pet.run_until_parked();
    for _ in 0..8 {
        pet.executor().advance_clock(Duration::from_millis(180));
        pet.run_until_parked();
    }
    pet.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(pet.debug_bounds("desktop-pet-thinking").is_some());
    assert!(pet.debug_bounds("desktop-pet-bubble").is_none());

    // A completed API response follows the same visible feedback path.
    pet.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.encourager.loading = false;
            app.encourager.error = None;
            app.encourager.reply = Some("喵，我在。".into());
            cx.notify();
        })
    });
    pet.executor().advance_clock(Duration::from_millis(180));
    pet.run_until_parked();
    pet.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(pet.debug_bounds("desktop-pet-copy").is_some());
    pet.simulate_keystrokes("escape");
    pet.run_until_parked();
    pet.update(|window, cx| {
        let _ = window.draw(cx);
        assert!(window.painted_quads().len() >= 20);
    });
    assert!(pet.debug_bounds("desktop-pet-bubble").is_none());
    cx.update(|cx| {
        app.update(cx, |app, cx| {
            app.set_encourager_pet_enabled(false, cx);
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn desktop_pet_reply_and_menu_keep_surface_and_character_bounds(cx: &mut TestAppContext) {
    use super::*;
    use gpui_kit::{Modifiers, MouseButton};
    let _locale = crate::test_support::chinese();
    let (app, mut visual, pet) = fixture(cx);
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let surface = visual.update(|window, _| window.bounds());
    let character = visual.debug_bounds("desktop-pet-character").unwrap();
    let center = character.center();
    for _ in 0..3 {
        visual.simulate_mouse_down(center, MouseButton::Right, Modifiers::default());
        visual.simulate_mouse_up(center, MouseButton::Right, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(window.bounds(), surface);
        });
        assert_eq!(
            visual.debug_bounds("desktop-pet-character"),
            Some(character)
        );
        let menu = visual.debug_bounds("desktop-pet-bubble").unwrap();
        assert!(menu.origin.y >= px(0.));
        assert!(menu.bottom() <= character.origin.y);
        visual.simulate_mouse_down(center, MouseButton::Right, Modifiers::default());
        visual.simulate_mouse_up(center, MouseButton::Right, Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(window.bounds(), surface);
        });
        assert!(visual.debug_bounds("desktop-pet-bubble").is_none());
    }
    visual.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.encourager.reply = Some(super::super::finish_reply(
                &"一步一步慢慢来，不用急着解决所有问题。".repeat(12),
                120,
            ));
            cx.notify();
        })
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
        assert_eq!(window.bounds(), surface);
    });
    let bubble = visual.debug_bounds("desktop-pet-bubble").unwrap();
    let body = visual.debug_bounds("desktop-pet-feedback").unwrap();
    let copy = visual.debug_bounds("desktop-pet-copy").unwrap();
    assert!(bubble.origin.y >= px(0.));
    assert!(body.origin.y >= bubble.origin.y);
    assert!(
        body.size.height >= px(100.),
        "reply keeps a readable area above actions"
    );
    assert!(body.bottom() <= copy.origin.y);
    assert!(copy.bottom() <= bubble.bottom());
    assert_eq!(
        visual.debug_bounds("desktop-pet-character"),
        Some(character)
    );
    let region = visual.update(|_, cx| pet.read(cx).visible_region.borrow().clone());
    assert_eq!(
        region.len(),
        2,
        "only character and visible bubble belong to the window region"
    );
    cx.update(|cx| app.update(cx, |app, cx| app.set_encourager_pet_enabled(false, cx)));
}

#[gpui_kit::test]
fn desktop_pet_tracks_background_runs_and_does_not_restart_idle_on_host_notifications(
    cx: &mut TestAppContext,
) {
    use super::*;
    let _locale = crate::test_support::chinese();
    let (app, mut visual, pet) = fixture(cx);
    visual.update(|_, cx| {
        app.update(cx, |app, cx| {
            let (runtime, _) = ChatRuntime::test_channel();
            app.session_control.runs.insert(
                42,
                ActiveRun::new(
                    runtime,
                    std::path::PathBuf::from("D:/pet-test"),
                    "turn-pet".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            cx.notify();
        })
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        pet.update(cx, |pet, cx| {
            assert_eq!(
                pet.machine.tick(cx.background_executor().now()).0,
                PetState::Working
            )
        })
    });
    visual.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.get_mut(&42).unwrap().cleanup_error =
                Some("cleanup pending".into());
            cx.emit(PetFeedback { failed: false });
            cx.notify();
        })
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        pet.update(cx, |pet, cx| {
            assert_eq!(
                pet.machine.tick(cx.background_executor().now()).0,
                PetState::Attention
            )
        })
    });
    visual.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.clear();
            cx.notify();
        })
    });
    visual.run_until_parked();
    // Unrelated host notifications must not count as pet activity.
    for _ in 0..6 {
        visual.executor().advance_clock(Duration::from_secs(60));
        visual.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        visual.run_until_parked();
        visual.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
    visual.update(|_, cx| {
        pet.update(cx, |pet, cx| {
            assert_eq!(
                pet.machine.tick(cx.background_executor().now()).0,
                PetState::Sleep
            )
        })
    });
    cx.update(|cx| app.update(cx, |app, cx| app.set_encourager_pet_enabled(false, cx)));
}

#[gpui_kit::test]
fn desktop_pet_feedback_pause_resumes_and_closing_drops_timers(cx: &mut TestAppContext) {
    use super::*;
    let _locale = crate::test_support::chinese();
    let (app, mut visual, pet) = fixture(cx);
    visual.update(|window, cx| {
        pet.update(cx, |pet, cx| {
            pet.interact(window, cx);
        })
    });
    visual.executor().advance_clock(Duration::from_millis(600));
    visual.run_until_parked();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    visual.update(|_, cx| {
        pet.update(cx, |pet, cx| {
            assert!(pet.feedback_visible);
            pet.bubble_hovered = true;
            pet.pause_feedback(true, cx.background_executor().now());
            cx.notify();
        })
    });
    visual.executor().advance_clock(Duration::from_secs(30));
    visual.run_until_parked();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    visual.update(|_, cx| {
        pet.update(cx, |pet, cx| {
            assert!(pet.feedback_visible);
            pet.bubble_hovered = false;
            pet.pause_feedback(false, cx.background_executor().now());
            cx.notify();
        })
    });
    visual.run_until_parked();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    visual.executor().advance_clock(Duration::from_secs(25));
    visual.run_until_parked();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    visual.update(|_, cx| pet.update(cx, |pet, _| assert!(!pet.feedback_visible)));
    let weak = pet.downgrade();
    cx.update(|cx| app.update(cx, |app, cx| app.set_encourager_pet_enabled(false, cx)));
    drop(pet);
    visual.run_until_parked();
    assert!(
        weak.upgrade().is_none(),
        "timer tasks must not retain the closed pet"
    );
}

#[test]
fn desktop_pet_pixel_geometry_tracks_fractional_dpi() {
    for scale in [1., 1.25, 1.5, 2.] {
        let cell = f32::from(super::cell_size(scale)) * scale;
        assert!((cell - cell.round()).abs() < 0.0001);
        let compact = super::compact_size(scale);
        assert!((f32::from(compact.width) * scale - 25. * cell).abs() < 0.0001);
        assert!((f32::from(compact.height) * scale - 19. * cell).abs() < 0.0001);
    }
}

#[test]
#[ignore = "requires a real Windows desktop and GPU"]
fn desktop_pet_pixel_native_snapshots() {
    use super::*;
    let _locale = crate::test_support::chinese();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            crate::app::install_theme_packs(cx);
            let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
            let output = holder.clone();
            let _host = cx
                .open_window(
                    WindowOptions {
                        show: false,
                        focus: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        let app = cx.new(|cx| {
                            let mut store = pet_store(false);
                            store.ui.encourager.pet_position =
                                Some(crate::store::EncouragerPetPosition { x: 400, y: 400 });
                            let mut app =
                                ConduitApp::with_store(window, cx, Ok(store), false, false);
                            app.persistence =
                                crate::persistence::Persistence::with_writer(|_| Ok(()));
                            app.store.providers.clear();
                            app
                        });
                        *output.borrow_mut() = Some(app.clone());
                        cx.new(|cx| gpui_kit::component::Root::new(app, window, cx))
                    },
                )
                .unwrap();
            let app = holder.borrow().as_ref().unwrap().clone();
            app.update(cx, |app, cx| app.set_encourager_pet_enabled(true, cx));
            let handle = app
                .read(cx)
                .encourager_pet_window
                .unwrap()
                .downcast::<PetView>()
                .unwrap();
            cx.spawn(async move |cx| {
                for state in [
                    PetState::Idle,
                    PetState::Working,
                    PetState::Attention,
                    PetState::Success,
                    PetState::Error,
                    PetState::Sleep,
                    PetState::Poked,
                    PetState::Dragged,
                ] {
                    handle
                        .update(cx, |pet, _, cx| {
                            pet.machine.event(
                                pet_core::PetEvent::DebugState(state),
                                cx.background_executor().now(),
                            );
                            cx.notify();
                        })
                        .unwrap();
                    cx.background_executor()
                        .timer(Duration::from_millis(200))
                        .await;
                    handle
                        .update(cx, |_, window, cx| {
                            window.defer(cx, move |window, cx| {
                                window.draw(cx).clear(cx);
                                let image = window.render_to_image().unwrap();
                                let transparent =
                                    image.pixels().filter(|pixel| pixel.0[3] == 0).count();
                                assert!(transparent > image.pixels().count() / 2);
                                assert!(
                                    image
                                        .pixels()
                                        .any(|pixel| pixel.0[3] > 0 && pixel.0[3] < 255),
                                    "shadow must remain translucent"
                                );
                                std::fs::create_dir_all("target/pet-pixels").unwrap();
                                image
                                    .save(format!("target/pet-pixels/{}.png", state_name(state)))
                                    .unwrap();
                                #[cfg(target_os = "windows")]
                                unsafe {
                                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                                    let hwnd = super::super::native_window::handle(window).unwrap();
                                    assert_eq!(
                                        SendMessageW(hwnd, WM_MOUSEACTIVATE, 0, 0),
                                        MA_NOACTIVATE as isize
                                    );
                                    assert_ne!(
                                        GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE,
                                        0
                                    );
                                }
                            });
                        })
                        .unwrap();
                }
                let stable_bounds = handle.update(cx, |_, window, _| window.bounds()).unwrap();
                let stable_position = handle.update(cx, |_, window, _| position_from_window(window)).unwrap();
                for mode in ["reply", "menu", "dismissed", "reply", "menu", "dismissed"] {
                    handle.update(cx, |pet, _, cx| {
                        let now = cx.background_executor().now();
                        pet.machine.event(pet_core::PetEvent::Reset, now);
                        pet.output.reply = Some(super::super::finish_reply(
                            &"一步一步慢慢来，不用急着解决所有问题。先完成手边的小目标，剩下的我们一起处理。".repeat(5),
                            120,
                        ));
                        pet.menu = mode == "menu";
                        pet.feedback_visible = mode == "reply";
                        pet.feedback_until = None;
                        cx.notify();
                    }).unwrap();
                    cx.background_executor().timer(Duration::from_millis(120)).await;
                    handle.update(cx, move |pet, window, cx| {
                        let rects = pet.visible_region.borrow().clone();
                        assert_eq!(rects.len(), if mode == "dismissed" { 1 } else { 2 });
                        window.defer(cx, move |window, cx| {
                            window.draw(cx).clear(cx);
                            assert_eq!(window.bounds(), stable_bounds, "popup must not resize or move its surface");
                            assert_eq!(position_from_window(window), stable_position, "popup must not move the pet");
                            window.render_to_image().unwrap().save(format!("target/pet-pixels/bubble-{mode}.png")).unwrap();
                            #[cfg(target_os = "windows")]
                            unsafe {
                                use windows_sys::Win32::Graphics::Gdi::*;
                                let hwnd = super::super::native_window::handle(window).unwrap();
                                let region = CreateRectRgn(0, 0, 0, 0);
                                assert!(!region.is_null());
                                assert_ne!(GetWindowRgn(hwnd, region), 0);
                                assert_eq!(PtInRegion(region, 1, 1), 0, "unused canvas must not block the desktop");
                                for [left, top, right, bottom] in rects {
                                    assert_ne!(PtInRegion(region, (left + right) / 2, (top + bottom) / 2), 0);
                                }
                                if mode == "dismissed" {
                                    assert_eq!(PtInRegion(region, 12, 24), 0, "closing must release the bubble region");
                                }
                                DeleteObject(region);
                            }
                        });
                    }).unwrap();
                }
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                // Keep a real isolated fixture visible only when explicitly requested
                // for mouse/focus verification; no configured API service is present.
                if std::env::var_os("CONDUIT_PET_HOLD").is_some() {
                    handle
                        .update(cx, |pet, _, cx| {
                            pet.machine
                                .event(pet_core::PetEvent::Reset, cx.background_executor().now());
                            cx.notify();
                        })
                        .unwrap();
                    cx.background_executor()
                        .timer(Duration::from_secs(180))
                        .await;
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
}
