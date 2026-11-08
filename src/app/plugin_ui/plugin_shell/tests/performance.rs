//! Opt-in CPU/layout probe. The test platform does not present frames to a GPU.
use super::*;
use crate::performance::PerformanceMonitor;
use serde_json::{Value, json};
use std::time::Instant;

const INCREMENT: &str = "example.workspace-overview.increment";

pub(super) fn memory() -> Value {
    let monitor = PerformanceMonitor::new();
    json!({
        "working_set_bytes": monitor.memory_bytes.expect("process working set"),
        "private_bytes": monitor.private_bytes.expect("process private bytes"),
        "peak_working_set_bytes": monitor.peak_memory_bytes.expect("process peak working set"),
    })
}

pub(super) fn counts(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> (usize, usize) {
    cx.update(|_, cx| {
        app.read(cx).plugins.shell.as_ref().map_or((0, 0), |shell| {
            assert!(shell.error.is_none(), "{:?}", shell.error);
            (shell.host.runtime_count(), shell.host.instance_count())
        })
    })
}

pub(super) fn number(name: &str, range: std::ops::RangeInclusive<usize>) -> usize {
    let value = std::env::var(name).expect("use verify-desktop.py --phase performance");
    let value = value.parse::<usize>().expect("numeric probe option");
    assert!(range.contains(&value), "{name} out of range");
    value
}

pub(super) fn sample(warmup: usize, count: usize, mut action: impl FnMut()) -> Vec<f64> {
    let mut samples = Vec::with_capacity(count);
    for index in 0..warmup + count {
        // Real time pacing stays outside the measurement and avoids turning a
        // normal interaction sample into an artificial cumulative-budget flood.
        std::thread::sleep(Duration::from_millis(16));
        let start = Instant::now();
        action();
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if index >= warmup {
            samples.push(elapsed);
        }
    }
    samples
}

#[gpui_kit::test]
#[ignore = "wall-clock measurements; run verify-desktop.py --phase performance"]
fn desktop_performance_probe(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let scenario = std::env::var("CONDUIT_PERF_SCENARIO").expect("explicit probe scenario");
    assert!(matches!(
        scenario.as_str(),
        "baseline" | "registered" | "navigation" | "dual" | "hidden" | "stopped"
    ));
    let count = number("CONDUIT_PERF_SAMPLES", 20..=2000);
    let warmup = number("CONDUIT_PERF_WARMUP", 1..=200);
    let before_fixture = memory();
    let started = Instant::now();
    let packages = if scenario == "baseline" {
        vec![]
    } else {
        vec![package()]
    };
    let (app, cx) = fixture_with_packages(cx, packages);
    let fixture_ms = started.elapsed().as_secs_f64() * 1000.;
    assert_eq!(
        counts(&app, cx),
        (0, 0),
        "registration must not activate JS"
    );
    let registered_memory = memory();
    let started = Instant::now();
    if matches!(
        scenario.as_str(),
        "navigation" | "dual" | "hidden" | "stopped"
    ) {
        click(cx, builtins::EXAMPLE_ACTIVITY);
        assert!(text(&app, false, cx).contains("Development navigation"));
    }
    if matches!(scenario.as_str(), "dual" | "hidden" | "stopped") {
        command(&app, INCREMENT, cx);
        assert!(text(&app, true, cx).contains("Command count: 1"));
    }
    let activation_ms = started.elapsed().as_secs_f64() * 1000.;
    let active_memory = memory();
    if matches!(scenario.as_str(), "hidden" | "stopped") {
        click(cx, "nav-workspaces");
        cx.update(|window, cx| app.update(cx, |app, cx| app.close_right_view(window, cx)));
        draw(cx);
        draw(cx);
        cx.update(|_, cx| {
            let plugins = &app.read(cx).plugins;
            assert!(plugins.is_selected(builtins::WORKSPACES));
            assert!(!plugins.right_open());
        });
    }
    let weak = (scenario == "stopped").then(|| runtime(&app, cx));
    if scenario == "stopped" {
        cx.update(|window, cx| app.update(cx, |app, cx| app.disable_shell(window, cx)));
        draw(cx);
        draw(cx);
        assert!(weak.as_ref().unwrap().upgrade().is_none());
    }
    let expected = match scenario.as_str() {
        "navigation" => (1, 1),
        "dual" | "hidden" => (1, 2),
        _ => (0, 0),
    };
    assert_eq!(counts(&app, cx), expected);
    // GPUI does not record debug selectors in release. Check actual host state
    // so a missing debug marker cannot make a visibility assertion pass.
    cx.update(|_, cx| {
        let plugins = &app.read(cx).plugins;
        assert_eq!(
            plugins.is_selected(builtins::EXAMPLE_INDEX),
            matches!(scenario.as_str(), "navigation" | "dual")
        );
        assert_eq!(plugins.right_open(), scenario == "dual");
    });
    let ready_memory = memory();

    // Fixed-size host editing: no growing draft or progressively larger layout.
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    draw(cx);
    let mut alternate = false;
    let input = sample(warmup, count, || {
        alternate = !alternate;
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input(if alternate {
            "G5 中文😀 input A"
        } else {
            "G5 中文😀 input B"
        });
        draw(cx);
    });
    cx.update(|window, cx| {
        let input = app.read(cx).input.read(cx);
        assert_eq!(
            input.value().as_str(),
            if alternate {
                "G5 中文😀 input A"
            } else {
                "G5 中文😀 input B"
            }
        );
        assert!(input.focus_handle(cx).is_focused(window));
    });
    // Includes native element materialization, layout and test paint. Cached
    // ScriptView snapshots are deliberately reused in this workload.
    let redraw = sample(warmup, count, || {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        draw(cx);
    });
    let mut timings = json!({"host_edit_draw_ms": input, "host_notify_draw_ms": redraw});
    if scenario == "dual" {
        timings["plugin_command_draw_ms"] = json!(sample(warmup, count, || {
            command(&app, INCREMENT, cx);
        }));
        assert!(text(&app, true, cx).contains(&format!("Command count: {}", 1 + warmup + count)));
    }
    assert_eq!(
        counts(&app, cx),
        expected,
        "probe must not silently lose a runtime"
    );
    if scenario == "navigation" || scenario == "dual" {
        let _ = text(&app, false, cx);
    }
    let after_samples = memory();
    let report = json!({
        "schema_version": 1,
        "scenario": scenario,
        "pid": std::process::id(),
        "platform": "gpui-test-no-gpu",
        "debug_assertions": cfg!(debug_assertions),
        "window_px": [1600, 800],
        "dataset": "plugin-ui-store: 2 workspaces, 1 synthetic session each",
        "samples": count,
        "warmup": warmup,
        "pace_ms": 16,
        "runtime_count": expected.0,
        "instance_count": expected.1,
        "fixture_ms": fixture_ms,
        "activation_ms": activation_ms,
        "memory": {
            "before_fixture": before_fixture,
            "registered": registered_memory,
            "active": active_memory,
            "ready": ready_memory,
            "after_samples": after_samples,
        },
        "timings": timings,
    });
    let output = std::env::var("CONDUIT_PERF_OUTPUT").expect("probe output path");
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
