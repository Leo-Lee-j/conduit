//! Separate G5 input/Markdown attribution; preserves the historical stress driver.
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
    profiling::begin();
    let started = Instant::now();
    body(cx);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    let counters = profiling::finish();
    writeln!(
        file,
        "{}",
        json!({"event":"end", "action":action, "index":index,
        "elapsed_ms":elapsed_ms, "root_renders":frames.get()-before, "counters":counters})
    )
    .unwrap();
}

#[gpui_kit::test]
#[ignore = "chat attribution; use verify-desktop.py --phase chat-profile"]
fn desktop_chat_profile_probe(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
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
        json!({"event":"header", "schema_version":2, "suite":"chat-profile",
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
        draw(cx);
    }
    assert_eq!(counts(&app, cx), (active, active));
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    draw(cx);
    // Prove this process really materialized Markdown, then permit cached frames.
    // This calibration and all visibility assertions remain outside timed stages.
    profiling::begin();
    cx.update(|_, cx| {
        app.read(cx)
            .timeline
            .clone()
            .update(cx, |_, cx| cx.notify())
    });
    draw(cx);
    let counters = profiling::finish();
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
        stage(&mut file, &frames, "input-draw", index, cx, draw);
        stage(&mut file, &frames, "notify-draw", index, cx, |cx| {
            cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
            draw(cx);
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

#[gpui_kit::test]
fn desktop_chat_cache_survives_shell_root_input_notifications(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (store, _) = dataset();
    let (app, cx) = fixture_with_frame_counter(cx, Vec::new(), store, None);
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("warmup");
    draw(cx);
    draw(cx);
    cx.simulate_keystrokes("ctrl-a");
    let before = last_reply_bounds(&app, cx);
    profiling::begin();
    cx.simulate_input("G5 中文😀 input A");
    draw(cx);
    let counters = profiling::finish();
    assert_eq!(counters["row_build"]["count"], 0, "{counters}");
    assert_eq!(last_reply_bounds(&app, cx), before);
    cx.update(|window, cx| {
        let input = app.read(cx).input.read(cx);
        assert_eq!(input.value().as_str(), "G5 中文😀 input A");
        assert!(input.focus_handle(cx).is_focused(window));
    });
}
