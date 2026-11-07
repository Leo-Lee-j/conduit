//! Independent G5 host attribution with delivered frame callbacks and cleared arenas.
use super::performance::{counts, number};
use super::stress::{ACTIVE, PACKAGES, dataset, owner, packages};
use super::*;
use crate::app::chat::profiling;
use conduit::plugins::ids::ViewId;
use serde_json::{Value, json};
use std::{cell::Cell, io::Write, time::Instant};

fn stage(
    file: &mut std::fs::File,
    frames: &Cell<u64>,
    action: &str,
    index: usize,
    cx: &mut VisualTestContext,
    body: impl FnOnce(&mut VisualTestContext),
) {
    // Flush the beginning outside the timed interval, retaining a failed stage.
    writeln!(
        file,
        "{}",
        json!({"event":"begin", "action":action, "index":index})
    )
    .unwrap();
    file.flush().unwrap();
    let before = frames.get();
    if host_counters() {
        profiling::begin_host();
    } else {
        profiling::begin();
    }
    let started = Instant::now();
    body(cx);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    let counters = if host_counters() {
        profiling::finish_host()
    } else {
        profiling::finish()
    };
    writeln!(
        file,
        "{}",
        json!({"event":"end", "action":action, "index":index,
        "elapsed_ms":elapsed_ms, "root_renders":frames.get()-before, "counters":counters})
    )
    .unwrap();
}

#[gpui_kit::test]
#[ignore = "host attribution; run the archived host-profile runner"]
fn desktop_host_profile_probe(cx: &mut TestAppContext) {
    let scenario = std::env::var("CONDUIT_PERF_SCENARIO").unwrap();
    assert!(matches!(
        scenario.as_str(),
        "baseline" | "registered" | "active"
    ));
    let samples = number("CONDUIT_PERF_SAMPLES", 20..=2000);
    let warmup = number("CONDUIT_PERF_WARMUP", 1..=200);
    let (store, metadata) = dataset();
    let registered = if scenario == "baseline" { 0 } else { PACKAGES };
    let active = if scenario == "active" { ACTIVE } else { 0 };
    let frames = Rc::new(Cell::new(0));
    let mut file = std::fs::File::create(std::env::var("CONDUIT_PERF_OUTPUT").unwrap()).unwrap();
    writeln!(
        file,
        "{}",
        json!({"event":"header", "schema_version":1, "suite":"host-profile", "frame_driver":"simulate_next_frame+draw.clear", "host_counters":host_counters(),
        "scenario":scenario, "samples":samples, "warmup":warmup, "pid":std::process::id(),
        "debug_assertions":cfg!(debug_assertions), "platform":"gpui-test-no-gpu",
        "dataset":metadata, "registered_package_count":registered, "active":active,
        "window_px":[1600,800], "pace_ms":16, "input":"G5 中文😀 input A"})
    )
    .unwrap();
    let (app, cx) =
        fixture_with_frame_counter(cx, packages(registered), store, Some(frames.clone()));
    for index in 0..active {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.open_right_view(
                    &ViewId::new(format!("{}.summary", owner(index))).unwrap(),
                    window,
                    cx
                ));
            })
        });
        draw_frame(cx);
    }
    assert_eq!(counts(&app, cx), (active, active));
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    draw_frame(cx);
    // Prove this process really materialized Markdown, then permit cached frames.
    // This calibration and all visibility assertions remain outside timed stages.
    if host_counters() {
        profiling::begin_host();
    } else {
        profiling::begin();
    }
    cx.update(|_, cx| {
        app.read(cx)
            .timeline
            .clone()
            .update(cx, |_, cx| cx.notify())
    });
    draw_frame(cx);
    let counters = if host_counters() {
        profiling::finish_host()
    } else {
        profiling::finish()
    };
    for name in [
        "row_build",
        "markdown_request_layout",
        "markdown_prepaint",
        "markdown_paint",
    ] {
        assert!(counters[name]["count"].as_u64().unwrap() > 0, "{counters}");
    }
    writeln!(
        file,
        "{}",
        json!({"event":"render-proof", "counters":counters})
    )
    .unwrap();
    for index in 0..warmup + samples {
        std::thread::sleep(Duration::from_millis(16));
        stage(&mut file, &frames, "rows-sync", index, cx, |cx| {
            cx.update(|_, cx| app.update(cx, |app, _| app.sync_chat_rows()));
        });
        stage(&mut file, &frames, "select-all", index, cx, |cx| {
            cx.simulate_keystrokes("ctrl-a")
        });
        let input = if index % 2 == 0 {
            "G5 中文😀 input A"
        } else {
            "G5 中文😀 input B"
        };
        // This is the same character input path used by stress, one scalar at a time.
        for (position, ch) in input.chars().enumerate() {
            stage(
                &mut file,
                &frames,
                &format!("key-{position:02}"),
                index,
                cx,
                |cx| cx.simulate_input(&ch.to_string()),
            );
        }
        stage(&mut file, &frames, "input-draw", index, cx, draw_frame);
        stage(&mut file, &frames, "notify-draw", index, cx, |cx| {
            cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
            draw_frame(cx);
        });
        cx.update(|window, cx| {
            let input_state = app.read(cx).input.read(cx);
            assert_eq!(input_state.value().as_str(), input);
            assert!(input_state.focus_handle(cx).is_focused(window));
        });
        let bounds = last_reply_bounds(&app, cx);
        let viewport = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds());
        assert!(
            bounds.size.width > px(0.)
                && bounds.size.height > px(0.)
                && bounds.intersects(&viewport)
        );
        assert_eq!(counts(&app, cx), (active, active));
        if active > 0 {
            assert!(text(&app, true, cx).contains("Development summary"));
        }
        writeln!(
            file,
            "{}",
            json!({"event":"verified", "index":index,
            "runtime_count":active, "instance_count":active, "input":input, "transcript_visible":true})
        )
        .unwrap();
    }
    let result: Value = json!({"event":"complete", "iterations":warmup + samples,
        "runtime_count":active, "instance_count":active});
    writeln!(file, "{result}").unwrap();
}

fn last_reply_bounds(
    app: &Entity<ConduitApp>,
    cx: &mut VisualTestContext,
) -> gpui_kit::Bounds<gpui_kit::Pixels> {
    let id = cx.update(|_, cx| {
        let app = app.read(cx);
        let session = app.session(app.selected_session.unwrap()).unwrap();
        let item = session.messages.last().unwrap();
        format!("message-{}-{}", session.id, item.key)
    });
    cx.element_bounds(&id)
        .unwrap_or_else(|| panic!("missing visible reply {id}"))
}

fn draw_frame(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
    });
}

fn host_counters() -> bool {
    match std::env::var("CONDUIT_HOST_COUNTERS")
        .as_deref()
        .unwrap_or("on")
    {
        "on" => true,
        "off" => false,
        value => panic!("invalid host counter mode: {value}"),
    }
}

#[gpui_kit::test]
fn desktop_dock_header_cache_skips_draft_and_script_updates(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("warmup");
    draw_frame(cx);
    let before = cx.element_bounds("detail-header").unwrap();
    assert!(before.size.width > px(250.) && before.size.height == px(crate::app::TAB_BAR_HEIGHT));
    profiling::begin_host();
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("G5 中文😀 input A");
    cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
    draw_frame(cx);
    let measured = profiling::finish_host();
    assert!(measured["host_render"]["count"].as_u64().unwrap() >= 14);
    assert_eq!(
        measured["dock_header_build"]["count"].as_u64().unwrap_or(0),
        0,
        "{measured}"
    );
    assert_eq!(cx.element_bounds("detail-header").unwrap(), before);
    profiling::begin_host();
    command(&app, "example.workspace-overview.increment", cx);
    draw_frame(cx);
    let measured = profiling::finish_host();
    assert_eq!(
        measured["dock_header_build"]["count"].as_u64().unwrap_or(0),
        0,
        "{measured}"
    );
    assert!(text(&app, true, cx).contains("Command count: 2"));
    crate::app::plugin_ui::tests::click_labeled(cx, gpui_kit::Role::Tab, "通知");
    assert!(cx.element_bounds("notification-panel").is_some());
    crate::app::plugin_ui::tests::click_labeled(cx, gpui_kit::Role::Tab, "Workspace summary");
    assert!(text(&app, true, cx).contains("Command count: 2"));
    crate::app::plugin_ui::tests::click_labeled(
        cx,
        gpui_kit::Role::Button,
        "关闭Workspace summary",
    );
    assert!(cx.element_bounds("detail-panel").is_none());
}

#[gpui_kit::test]
fn desktop_dock_header_cache_tracks_context_theme_bounds_and_revocation(cx: &mut TestAppContext) {
    use crate::app::plugin_ui::tests::click_labeled;
    use gpui_kit::Role;
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    draw_frame(cx);
    profiling::begin_host();
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
    draw_frame(cx);
    let measured = profiling::finish_host();
    assert!(
        measured["dock_header_build"]["count"].as_u64().unwrap_or(0) > 0,
        "{measured}"
    );
    click_labeled(cx, Role::Tab, "通知");
    // A cached request from the previous context would be rejected here.
    click_labeled(cx, Role::Tab, "Workspace summary");
    assert!(text(&app, true, cx).contains("Development summary"));
    click_labeled(cx, Role::Tab, "通知");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        })
    });
    draw_frame(cx);
    click_labeled(cx, Role::Tab, "Workspace summary");
    assert!(cx.element_bounds("notification-panel").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            cx.notify();
        })
    });
    draw_frame(cx);
    click_labeled(cx, Role::Tab, "Workspace summary");
    assert!(text(&app, true, cx).contains("Development summary"));
    for mode in [
        crate::store::AppearanceMode::Light,
        crate::store::AppearanceMode::Dark,
    ] {
        profiling::begin_host();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.appearance = mode;
                app.apply_appearance(window, cx);
            })
        });
        draw_frame(cx);
        let measured = profiling::finish_host();
        assert!(
            measured["dock_header_build"]["count"].as_u64().unwrap_or(0) > 0,
            "{measured}"
        );
    }
    for width in [1000., 1400.] {
        cx.simulate_resize(size(px(width), px(900.)));
        draw_frame(cx);
        let header = cx.element_bounds("detail-header").unwrap();
        let panel = cx.element_bounds("detail-panel").unwrap();
        assert!(header.size.width > px(250.) && header.size.width <= panel.size.width);
        assert_eq!(header.size.height, px(crate::app::TAB_BAR_HEIGHT));
    }
    cx.update(|window, cx| app.update(cx, |app, cx| app.disable_shell(window, cx)));
    draw_frame(cx);
    assert!(cx.labeled_bounds(Role::Tab, "Workspace summary").is_none());
}

#[gpui_kit::test]
fn desktop_dock_header_cache_rebinds_identical_registry(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    draw_frame(cx);
    // Replace the entire registry with identical declarations and numeric
    // versions, and reopen the same selection without painting an intermediate
    // closed dock. The retained header must obtain new registration handles.
    let packages = vec![package()];
    profiling::begin_host();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.install_development_packages(packages, cx).unwrap();
            let command = app
                .plugins
                .host_command("example.workspace-overview.increment")
                .unwrap();
            assert!(app.dispatch_plugin_command(&command, window, cx));
        })
    });
    draw_frame(cx);
    let measured = profiling::finish_host();
    assert!(
        measured["dock_header_build"]["count"].as_u64().unwrap_or(0) > 0,
        "{measured}"
    );
    crate::app::plugin_ui::tests::click_labeled(cx, gpui_kit::Role::Tab, "通知");
    assert!(cx.element_bounds("notification-panel").is_some());
    crate::app::plugin_ui::tests::click_labeled(cx, gpui_kit::Role::Tab, "Workspace summary");
    assert!(text(&app, true, cx).contains("Command count: 1"));
}
