//! Separate from timing probes: atomics perturb allocation costs. All rows are
//! preallocated; optional JSON retention reproduces the older probe's observer cost.
use super::{
    performance::{counts, number},
    stress, *,
};
use crate::{
    allocation_probe::{self, AllocationSample},
    performance::PerformanceMonitor,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Instant;

#[derive(Clone, Copy, Serialize)]
struct ProcessMemory {
    working_set_bytes: usize,
    private_bytes: usize,
    peak_working_set_bytes: usize,
}
#[derive(Clone, Copy, Serialize)]
struct Sample {
    process: ProcessMemory,
    rust: AllocationSample,
}
fn sample() -> Sample {
    let monitor = PerformanceMonitor::new();
    let process = ProcessMemory {
        working_set_bytes: monitor.memory_bytes.unwrap(),
        private_bytes: monitor.private_bytes.unwrap(),
        peak_working_set_bytes: monitor.peak_memory_bytes.unwrap(),
    };
    drop(monitor);
    Sample {
        process,
        rust: allocation_probe::sample(),
    }
}
#[derive(Clone, Copy, Serialize)]
struct Point {
    cycle: usize,
    released_runtime_count: usize,
    activate_draw_ms: f64,
    stop_draw_ms: f64,
    ready: Sample,
    after_stop: Sample,
    settled: Sample,
}

fn draw_memory(cx: &mut VisualTestContext, frames: &str) {
    if frames == "manual" {
        // Preserve the original probe path for a directly comparable control.
        draw(cx);
    } else {
        cx.run_until_parked();
        cx.update(|window, cx| {
            // The test platform has no automatic frame source. Deliver the
            // callbacks a production frame would run, then release this draw's arena.
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
}

#[gpui_kit::test]
fn memory_frame_driver_delivers_deferred_work(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (_, cx) = fixture(cx);
    let calls = Rc::new(std::cell::Cell::new(0));
    cx.update(|window, _| {
        let calls = calls.clone();
        window.on_next_frame(move |window, _| {
            calls.set(calls.get() + 1);
            window.on_next_frame(move |_, _| calls.set(calls.get() + 1));
        });
    });
    draw_memory(cx, "manual");
    assert_eq!(
        calls.get(),
        0,
        "manual draws do not deliver platform frames"
    );
    draw_memory(cx, "complete");
    assert_eq!(
        calls.get(),
        1,
        "callbacks scheduled during delivery wait a frame"
    );
    draw_memory(cx, "complete");
    assert_eq!(calls.get(), 2);
}

#[gpui_kit::test]
#[ignore = "allocation instrumentation; run retained binary with allocation-probe feature"]
fn desktop_native_memory_probe(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let scenario = std::env::var("CONDUIT_PERF_SCENARIO").unwrap();
    assert!(matches!(scenario.as_str(), "lifecycle" | "idle"));
    let recording = std::env::var("CONDUIT_MEMORY_RECORDING").unwrap();
    assert!(matches!(recording.as_str(), "fixed" | "json"));
    let clock = std::env::var("CONDUIT_MEMORY_CLOCK").unwrap_or_else(|_| "frozen".into());
    assert!(matches!(clock.as_str(), "frozen" | "advance"));
    let frames = std::env::var("CONDUIT_MEMORY_FRAMES").unwrap_or_else(|_| "manual".into());
    assert!(matches!(frames.as_str(), "manual" | "complete"));
    let cycles = number("CONDUIT_STRESS_CYCLES", 3..=2000);
    let (store, dataset) = stress::dataset();
    let (app, cx) = fixture_with_store(cx, stress::packages(stress::PACKAGES), store);
    assert_eq!(counts(&app, cx), (0, 0));
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    draw_memory(cx, &frames);
    // Match the original probe's 10 warmup + 60 formal edit/notify operations.
    for index in 0..70 {
        std::thread::sleep(Duration::from_millis(16));
        cx.simulate_keystrokes("ctrl-a");
        cx.simulate_input(if index % 2 == 0 {
            "G5 中文😀 input A"
        } else {
            "G5 中文😀 input B"
        });
        draw_memory(cx, &frames);
    }
    for _ in 0..70 {
        std::thread::sleep(Duration::from_millis(16));
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        draw_memory(cx, &frames);
    }
    let mut points = Vec::with_capacity(cycles);
    let record_capacity_bytes = points.capacity() * std::mem::size_of::<Point>();
    let mut json_points = Vec::<Value>::new();
    let before_cycles = sample();
    for cycle in 1..=cycles {
        std::thread::sleep(Duration::from_millis(16));
        let started = Instant::now();
        if scenario == "lifecycle" {
            stress::activate(&app, cx);
        } else {
            draw_memory(cx, &frames);
        }
        let activate_draw_ms = started.elapsed().as_secs_f64() * 1000.;
        let active = if scenario == "lifecycle" {
            stress::ACTIVE
        } else {
            0
        };
        assert_eq!(counts(&app, cx), (active, active));
        let ready = sample();
        let weak = if active > 0 {
            stress::weak_runtimes(&app, cx)
        } else {
            vec![]
        };
        let started = Instant::now();
        stress::stop(&app, cx);
        let stop_draw_ms = started.elapsed().as_secs_f64() * 1000.;
        assert!(weak.iter().all(|vm| vm.upgrade().is_none()));
        let released_runtime_count = weak.len();
        drop(weak);
        let after_stop = sample();
        std::thread::sleep(Duration::from_millis(16));
        draw_memory(cx, &frames);
        draw_memory(cx, &frames);
        if clock == "advance" {
            cx.executor().advance_clock(Duration::from_millis(32));
            draw_memory(cx, &frames);
            draw_memory(cx, &frames);
        }
        let settled = sample();
        points.push(Point {
            cycle,
            released_runtime_count,
            activate_draw_ms,
            stop_draw_ms,
            ready,
            after_stop,
            settled,
        });
        if recording == "json" {
            // Same per-cycle fields as stress.rs, including three process samples.
            json_points.push(json!({"cycle":cycle,"active_runtime_count":active,
                "active_instance_count":active,"stopped_runtime_count":0,
                "stopped_instance_count":0,"released_runtime_count":released_runtime_count,
                "activate_draw_ms":activate_draw_ms,"stop_draw_ms":stop_draw_ms,
                "ready":ready.process,"after_stop":after_stop.process,"settled":settled.process}));
        }
    }
    let before_clock_settle = sample();
    // Distinguish virtual test-time retention from objects surviving elapsed time.
    // This is an extra diagnostic point, never a replacement for the raw curve.
    cx.executor().advance_clock(Duration::from_secs(5));
    draw_memory(cx, &frames);
    draw_memory(cx, &frames);
    let after_clock_settle = sample();
    let pending_frame_callbacks = cx.update(|window, cx| {
        let callbacks = window.simulate_next_frame(cx);
        window.draw(cx).clear(cx);
        callbacks
    });
    draw_memory(cx, "complete");
    let after_frame_settle = sample();
    let before_drop_records = allocation_probe::sample();
    drop(json_points);
    let after_drop_records = allocation_probe::sample();
    let json_record_live_bytes =
        before_drop_records.live_bytes as i64 - after_drop_records.live_bytes as i64;
    let json_record_live_blocks =
        before_drop_records.live_blocks as i64 - after_drop_records.live_blocks as i64;
    let report = json!({"schema_version":1,"suite":"native-memory","platform":"gpui-test-no-gpu",
        "production_gate":"not_evaluated","leak_verdict":"not_evaluated",
        "debug_assertions":cfg!(debug_assertions),"pid":std::process::id(),
        "scenario":scenario,"recording":recording,"clock":clock,"frames":frames,
        "pending_frame_callbacks":pending_frame_callbacks,"after_frame_settle":after_frame_settle,
        "before_clock_settle":before_clock_settle,"after_clock_settle":after_clock_settle,"cycles":cycles,"pace_ms":16,
        "samples":60,"warmup":10,"dataset":dataset,"record_capacity_bytes":record_capacity_bytes,
        "json_record_live_bytes":json_record_live_bytes,"json_record_live_blocks":json_record_live_blocks,
        "before_cycles":before_cycles,"lifecycle":points});
    std::fs::write(
        std::env::var("CONDUIT_PERF_OUTPUT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
