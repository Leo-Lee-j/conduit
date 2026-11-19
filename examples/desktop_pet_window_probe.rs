//! Real Windows/DWM probe, independent of GPUI's simulated TestPlatform.
//! cargo run --offline --features ui-tests --example desktop_pet_window_probe
//! Add --baseline to inspect the uncorrected toolkit window.
#[cfg(all(target_os = "windows", feature = "ui-tests"))]
#[path = "../src/app/encourager/native_window.rs"]
mod native_window;

#[cfg(all(target_os = "windows", feature = "ui-tests"))]
mod probe {
    use gpui_kit::{prelude::*, *};
    use windows_sys::Win32::{Foundation::RECT, Graphics::Dwm::*, UI::WindowsAndMessaging::*};

    struct Probe;
    struct Backdrop;
    impl Render for Backdrop {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().bg(rgb(0x244a3a))
        }
    }
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                div()
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .font_family("Consolas")
                    .text_size(px(22.))
                    .text_color(rgb(0xffd18a))
                    .child("( o.o )"),
            )
        }
    }

    fn inspect(window: &Window, label: &str, baseline: bool, expected: (i32, i32)) {
        let hwnd = super::native_window::handle(window).unwrap();
        unsafe {
            let mut outer = RECT::default();
            let mut client = RECT::default();
            assert_ne!(GetWindowRect(hwnd, &mut outer), 0);
            assert_ne!(GetClientRect(hwnd, &mut client), 0);
            let mut nc: i32 = -1;
            assert!(
                DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_NCRENDERING_ENABLED as u32,
                    (&mut nc as *mut i32).cast(),
                    4
                ) >= 0
            );
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            println!(
                "{label}: style={style:#x}, exstyle={:#x}, non-client={nc}, outer={}x{}, client={}x{}, logical={:?}",
                GetWindowLongW(hwnd, GWL_EXSTYLE),
                outer.right - outer.left,
                outer.bottom - outer.top,
                client.right,
                client.bottom,
                window.bounds()
            );
            println!(
                "capture:{label}:{},{},{},{}",
                outer.left - 12,
                outer.top - 12,
                outer.right + 12,
                outer.bottom + 12
            );
            if !baseline {
                assert_eq!(style & (WS_CAPTION | WS_THICKFRAME), 0);
                assert_ne!(style & WS_POPUP, 0);
                assert_eq!(nc, 0);
                assert_eq!(
                    (outer.right - outer.left, outer.bottom - outer.top),
                    (client.right, client.bottom)
                );
                let scale = window.scale_factor();
                assert_eq!(
                    (client.right, client.bottom),
                    (
                        (expected.0 as f32 * scale).round() as i32,
                        (expected.1 as f32 * scale).round() as i32
                    )
                );
            }
        }
        let image = window.render_to_image().unwrap();
        let transparent = image.pixels().filter(|p| p.0[3] == 0).count();
        println!(
            "{label}: render={}x{}, transparent pixels={transparent}/{}",
            image.width(),
            image.height(),
            image.pixels().count()
        );
        assert!(transparent > image.pixels().count() * 9 / 10);
        assert!(
            transparent < image.pixels().count(),
            "probe character must be rendered"
        );
        std::fs::create_dir_all("target/pet-probe").unwrap();
        image.save(format!("target/pet-probe/{label}.png")).unwrap();
    }

    pub fn run() {
        let baseline = std::env::args().any(|arg| arg == "--baseline");
        let capture = std::env::args().any(|arg| arg == "--capture");
        gpui_kit::application().run(move |cx| {
            let bounds = Bounds::new(point(px(400.), px(400.)), size(px(148.), px(112.)));
            let foreground = unsafe { GetForegroundWindow() };
            let backdrop = capture.then(|| {
                let bounds = Bounds::new(point(px(100.), px(100.)), size(px(600.), px(500.)));
                let handle = cx
                    .open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            kind: WindowKind::PopUp,
                            titlebar: None,
                            focus: false,
                            show: false,
                            is_resizable: false,
                            window_background: WindowBackgroundAppearance::Transparent,
                            ..Default::default()
                        },
                        |window, cx| {
                            super::native_window::prepare(window, bounds, cx).unwrap();
                            cx.new(|_| Backdrop)
                        },
                    )
                    .unwrap();
                handle
                    .update(cx, |_, window, _| super::native_window::show(window))
                    .unwrap();
                handle
            });
            let handle = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: None,
                        kind: WindowKind::PopUp,
                        focus: false,
                        show: baseline,
                        is_movable: true,
                        app_owns_titlebar_drag: true,
                        is_resizable: false,
                        is_minimizable: false,
                        window_background: WindowBackgroundAppearance::Transparent,
                        ..Default::default()
                    },
                    |window, cx| {
                        if !baseline {
                            super::native_window::prepare(window, bounds, cx).unwrap();
                        }
                        cx.new(|_| Probe)
                    },
                )
                .unwrap();
            cx.spawn(async move |cx| {
                for (index, (width, height)) in
                    [(148, 112), (320, 312), (148, 112)].into_iter().enumerate()
                {
                    if index > 0 {
                        handle
                            .update(cx, |_, window, cx| {
                                let old = window.bounds();
                                let size = size(px(width as f32), px(height as f32));
                                let bounds =
                                    Bounds::new(old.origin + old.size.into() - size.into(), size);
                                if baseline {
                                    window.resize(size);
                                } else {
                                    super::native_window::prepare(window, bounds, cx).unwrap();
                                }
                                cx.notify();
                            })
                            .unwrap();
                    }
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(120))
                        .await;
                    handle
                        .update(cx, |_, window, cx| {
                            window.defer(cx, move |window, cx| {
                                window.draw(cx).clear(cx);
                                if !baseline {
                                    super::native_window::show(window);
                                }
                                let label = format!(
                                    "{}-{index}",
                                    if baseline { "baseline" } else { "overlay" }
                                );
                                inspect(window, &label, baseline, (width, height));
                            });
                        })
                        .unwrap();
                    if capture {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(800))
                            .await;
                    }
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(120))
                    .await;
                assert_eq!(
                    unsafe { GetForegroundWindow() },
                    foreground,
                    "probe must not steal focus"
                );
                handle
                    .update(cx, |_, window, _| window.remove_window())
                    .unwrap();
                if let Some(backdrop) = backdrop {
                    backdrop
                        .update(cx, |_, window, _| window.remove_window())
                        .unwrap();
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}

fn main() {
    #[cfg(all(target_os = "windows", feature = "ui-tests"))]
    probe::run();
    #[cfg(not(all(target_os = "windows", feature = "ui-tests")))]
    eprintln!("Run on Windows with --features ui-tests to probe native transparency.");
}
