//! Component timing breakdown for the fixed long-chat stress dataset.
use super::performance::{counts, number, sample};
use super::stress::{ACTIVE, MESSAGES, PACKAGES, dataset, owner, packages};
use super::*;
use conduit::plugins::ids::ViewId;
use serde_json::json;
use std::time::Instant;

fn dispatch_only(app: &Entity<ConduitApp>, id: &str, cx: &mut VisualTestContext) -> f64 {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            // GPUI test-support draws dirty windows when App::update flushes
            // effects. Stop the dispatch timer before leaving this callback.
            let started = Instant::now();
            let command = app.plugins.host_command(id).unwrap();
            assert!(app.dispatch_plugin_command(&command, window, cx));
            started.elapsed().as_secs_f64() * 1000.
        })
    })
}

#[derive(Default)]
struct CommandTimings {
    dispatch: Vec<f64>,
    effects: Vec<f64>,
    settle: Vec<f64>,
    draw: Vec<f64>,
}

fn split_command(
    app: &Entity<ConduitApp>,
    id: &str,
    warmup: usize,
    samples: usize,
    initial_count: usize,
    cx: &mut VisualTestContext,
) -> CommandTimings {
    let mut timings = CommandTimings::default();
    for index in 0..warmup + samples {
        std::thread::sleep(Duration::from_millis(16));
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                let shell = app.plugins.shell.as_mut().unwrap();
                assert!(shell.tasks.is_empty());
                shell.result = None;
            })
        });
        let started = Instant::now();
        let dispatch_ms = dispatch_only(app, id, cx);
        let effects_ms = (started.elapsed().as_secs_f64() * 1000. - dispatch_ms).max(0.);
        let started = Instant::now();
        cx.run_until_parked();
        let settle_ms = started.elapsed().as_secs_f64() * 1000.;
        let started = Instant::now();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let draw_ms = started.elapsed().as_secs_f64() * 1000.;
        cx.update(|_, cx| {
            let shell = app.read(cx).plugins.shell.as_ref().unwrap();
            assert!(shell.error.is_none(), "{:?}", shell.error);
            assert!(
                shell.tasks.is_empty(),
                "command must settle before next sample"
            );
            assert!(shell.result.is_some(), "command result must be accepted");
        });
        assert!(
            text(app, true, cx).contains(&format!("Command count: {}", initial_count + index + 1))
        );
        if index >= warmup {
            timings.dispatch.push(dispatch_ms);
            timings.effects.push(effects_ms);
            timings.settle.push(settle_ms);
            timings.draw.push(draw_ms);
        }
    }
    timings
}

fn open_first_view(app: &Entity<ConduitApp>, index: usize, cx: &mut VisualTestContext) -> f64 {
    let id = ViewId::new(format!("{}.summary", owner(index))).unwrap();
    let started = Instant::now();
    cx.update(|window, cx| app.update(cx, |app, cx| assert!(app.open_right_view(&id, window, cx))));
    draw(cx);
    started.elapsed().as_secs_f64() * 1000.
}

#[gpui_kit::test]
#[ignore = "component breakdown; use verify-desktop.py --phase breakdown"]
fn desktop_breakdown_probe(cx: &mut TestAppContext) {
    let scenario = std::env::var("CONDUIT_PERF_SCENARIO").unwrap();
    assert!(matches!(
        scenario.as_str(),
        "host" | "registered" | "plugin"
    ));
    let samples = number("CONDUIT_PERF_SAMPLES", 20..=2000);
    let warmup = number("CONDUIT_PERF_WARMUP", 1..=200);
    let registered = if scenario == "host" { 0 } else { PACKAGES };
    let before = super::performance::memory();
    let (store, dataset_info) = dataset();
    let (app, cx) = fixture_with_store(cx, packages(registered), store);
    let registered_counts = counts(&app, cx);
    assert_eq!(registered_counts, (0, 0));
    let registered_count = cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session(app.selected_session.unwrap())
                .unwrap()
                .messages
                .len(),
            MESSAGES
        );
        app.plugins.shell.as_ref().map_or(0, |shell| {
            assert_eq!(shell.host.views().len(), PACKAGES * 2);
            assert_eq!(shell.registrations.len(), PACKAGES);
            assert!(
                shell
                    .registrations
                    .values()
                    .all(|r| app.plugins.registry.is_registered(r))
            );
            shell.manifests.len()
        })
    });
    assert_eq!(registered_count, registered);
    let registered_memory = super::performance::memory();
    let host_chat_rows_sync_ms = sample(warmup, samples, || {
        cx.update(|_, cx| app.update(cx, |app, _| app.sync_chat_rows()));
    });
    let host_draw_ms = sample(warmup, samples, || {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        draw(cx);
    });
    let timings = json!({
        "host_chat_rows_sync_ms": host_chat_rows_sync_ms,
        "host_draw_ms": host_draw_ms,
    });
    let mut plugin_first_view_materialize_ms = Vec::new();
    let mut plugin_cached_view_draw_ms = Vec::new();
    let mut local = CommandTimings::default();
    let mut host = CommandTimings::default();
    if scenario == "plugin" {
        for index in 0..ACTIVE {
            plugin_first_view_materialize_ms.push(open_first_view(&app, index, cx));
            assert_eq!(counts(&app, cx), (index + 1, index + 1));
            assert!(text(&app, true, cx).contains("Command count: 0"));
        }
        assert_eq!(counts(&app, cx), (ACTIVE, ACTIVE));
        let cached = sample(warmup, samples, || {
            cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
            draw(cx);
        });
        plugin_cached_view_draw_ms = cached;
        local = split_command(
            &app,
            &format!("{}.local", owner(ACTIVE - 1)),
            warmup,
            samples,
            0,
            cx,
        );
        host = split_command(
            &app,
            &format!("{}.metadata", owner(ACTIVE - 1)),
            warmup,
            samples,
            warmup + samples,
            cx,
        );
        assert!(text(&app, true, cx).contains("Development summary"));
    }
    let final_counts = counts(&app, cx);
    let active = if scenario == "plugin" { ACTIVE } else { 0 };
    assert_eq!(final_counts, (active, active));
    let after = super::performance::memory();
    let report = json!({
        "schema_version": 2,
        "suite": "breakdown",
        "scenario": scenario,
        "pid": std::process::id(),
        "platform": "gpui-test-no-gpu",
        "debug_assertions": cfg!(debug_assertions),
        "window_px": [1600, 800],
        "samples": samples,
        "warmup": warmup,
        "pace_ms": 16,
        "dataset": dataset_info,
        "registered_package_count": registered_count,
        "registered_runtime_count": registered_counts.0,
        "registered_instance_count": registered_counts.1,
        "runtime_count": final_counts.0,
        "instance_count": final_counts.1,
        "first_view_count": plugin_first_view_materialize_ms.len(),
        "memory": {"before_fixture": before, "registered": registered_memory, "after": after},
        "timings": timings,
        "plugin_first_view_materialize_ms": plugin_first_view_materialize_ms,
        "plugin_cached_view_draw_ms": plugin_cached_view_draw_ms,
        "plugin_local_command_dispatch_ms": local.dispatch,
        "plugin_local_command_effects_ms": local.effects,
        "plugin_local_command_settle_ms": local.settle,
        "plugin_local_command_draw_ms": local.draw,
        "plugin_host_module_dispatch_ms": host.dispatch,
        "plugin_host_module_effects_ms": host.effects,
        "plugin_host_module_settle_ms": host.settle,
        "plugin_host_module_draw_ms": host.draw,
    });
    std::fs::write(
        std::env::var("CONDUIT_PERF_OUTPUT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
