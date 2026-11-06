//! Persist stage boundaries even when a probe panics or its process times out.
use super::performance::number;
use super::stress::{ACTIVE, PACKAGES, dataset, owner, packages};
use super::*;
use conduit::plugins::ids::ViewId;
use gpui_kit::{AnyView, Context, IntoElement, Render, Window};
use serde_json::{Value, json};
use std::{cell::Cell, fs::File, io::Write, time::Instant};

/// This uncached root is rendered once by each test Window::draw. Count actual
/// render entries, not requested draws, notifications, or native GPU presents.
pub(super) struct FrameProbe {
    pub(super) content: AnyView,
    pub(super) frames: Rc<Cell<u64>>,
}
impl Render for FrameProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.frames.set(self.frames.get() + 1);
        self.content.clone()
    }
}

struct Trace {
    file: File,
    frames: Rc<Cell<u64>>,
    // Retain diagnostic handles so a reaped VM's first fault and final counters
    // remain readable. No recovery is attempted in this process after a fault.
    runtimes: Vec<Rc<gpui_shell::ShellRuntime>>,
    sequence: usize,
}
impl Trace {
    fn write(&mut self, value: Value) {
        serde_json::to_writer(&mut self.file, &value).unwrap();
        self.file.write_all(b"\n").unwrap();
        self.file.flush().unwrap();
    }

    fn state(&self, app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> Value {
        let frames = self.frames.get();
        let state = cx.update(|_, cx| {
            let app = app.read(cx);
            let shell = app.plugins.shell.as_ref().unwrap();
            let view = app.plugins.right_key().and_then(|key| shell.host.view(key));
            json!({
                "root_renders": frames,
                "runtime_count": shell.host.runtime_count(),
                "instance_count": shell.host.instance_count(),
                "tasks": shell.tasks.len(),
                "result": shell.result,
                "error": shell.error,
                "view_error": view.and_then(|view| view.read(cx).build_error().map(str::to_owned)),
                "runtimes": self.runtimes.iter().enumerate().map(|(index, runtime)| {
                    let metrics = runtime.read_metrics();
                    json!({
                        "owner": owner(index).as_str(), "error": runtime.resource_error(),
                        "diagnostic": runtime.resource_diagnostic(),
                        "script_renders": metrics.script_renders(),
                        "materializations": metrics.materializations(),
                        "frame_script_calls": metrics.frame_script_calls(),
                        "script_render_ms": metrics.script_render_time().as_secs_f64() * 1000.,
                        "host_module_ms": metrics.native_time().as_secs_f64() * 1000.,
                        "materialize_ms": metrics.materialize_time().as_secs_f64() * 1000.,
                    })
                }).collect::<Vec<_>>(),
            })
        });
        assert_eq!(
            frames,
            self.frames.get(),
            "reading diagnostic state drew a frame"
        );
        state
    }

    fn stage(
        &mut self,
        app: &Entity<ConduitApp>,
        cx: &mut VisualTestContext,
        action: &str,
        index: usize,
        phase: &str,
        body: impl FnOnce(&mut VisualTestContext) -> Value,
    ) {
        let before = self.state(app, cx);
        self.write(json!({"event": "begin", "sequence": self.sequence,
            "action": action, "index": index, "phase": phase, "state": before}));
        let started = Instant::now();
        let detail = body(cx);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
        let after = self.state(app, cx);
        self.write(json!({"event": "end", "sequence": self.sequence,
            "action": action, "index": index, "phase": phase,
            "elapsed_ms": elapsed_ms, "detail": detail, "state": after}));
        self.sequence += 1;
        assert!(
            after["error"].is_null(),
            "{action}[{index}] {phase}: {after}"
        );
        assert!(
            after["view_error"].is_null(),
            "{action}[{index}] {phase}: {after}"
        );
        assert!(
            after["runtimes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["error"].is_null()),
            "{action}[{index}] {phase}: {after}"
        );
    }
}

#[gpui_kit::test]
fn frame_counter_observes_automatic_and_explicit_draws(cx: &mut TestAppContext) {
    let frames = Rc::new(Cell::new(0));
    let (app, cx) = fixture_with_frame_counter(cx, vec![], store(), Some(frames.clone()));
    let initial = frames.get();
    cx.update(|_, _| {});
    assert_eq!(frames.get(), initial);
    cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
    assert_eq!(frames.get(), initial + 1);
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(frames.get(), initial + 2);
}

#[gpui_kit::test]
fn command_result_and_snapshot_settle_without_an_explicit_draw(cx: &mut TestAppContext) {
    let frames = Rc::new(Cell::new(0));
    let (app, cx) = fixture_with_frame_counter(cx, vec![package()], store(), Some(frames.clone()));
    for expected in 1..=2 {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.plugins.shell.as_mut().unwrap().result = None;
                let command = app
                    .plugins
                    .host_command("example.workspace-overview.increment")
                    .unwrap();
                assert!(app.dispatch_plugin_command(&command, window, cx));
            })
        });
        let dispatched = frames.get();
        cx.run_until_parked();
        assert_eq!(
            frames.get(),
            dispatched,
            "successful bookkeeping must not redraw the host"
        );
        cx.update(|_, cx| {
            let shell = app.read(cx).plugins.shell.as_ref().unwrap();
            assert!(shell.error.is_none(), "{:?}", shell.error);
            assert!(shell.tasks.is_empty());
            assert!(shell.result.is_some());
        });
        assert!(text(&app, true, cx).contains(&format!("Command count: {expected}")));
    }
}

#[gpui_kit::test]
fn command_errors_repaint_and_success_clears_the_banner(cx: &mut TestAppContext) {
    let directory = std::env::var_os("CONDUIT_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "command-error-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    let source = directory.join("source");
    std::fs::create_dir_all(&source).unwrap();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/development-overview");
    for name in [
        "conduit-plugin.json",
        "gpui-shell.json",
        "navigation.js",
        "summary.js",
    ] {
        let mut text = std::fs::read_to_string(fixture.join(name)).unwrap();
        if name == "summary.js" {
            text = text.replace("    if (name === \"increment\") {", "    if (name === \"increment\") {\n      this.failNext = !this.failNext;\n      if (this.failNext) return Promise.reject(new Error('diagnostic command failed'));");
        }
        std::fs::write(source.join(name), text).unwrap();
    }
    let snapshot = PackageSnapshot::prepare(&source, &directory.join("staging")).unwrap();
    let grants = snapshot.manifest().permissions().clone();
    let frames = Rc::new(Cell::new(0));
    let (app, cx) = fixture_with_frame_counter(
        cx,
        vec![ApprovedPackage::new(snapshot, grants).unwrap()],
        store(),
        Some(frames.clone()),
    );
    for fails in [true, false] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let invocation = app
                    .plugins
                    .host_command("example.workspace-overview.increment")
                    .unwrap();
                assert!(app.dispatch_plugin_command(&invocation, window, cx));
            })
        });
        let dispatched = frames.get();
        cx.run_until_parked();
        if fails {
            assert!(
                frames.get() > dispatched,
                "new command error must repaint its banner"
            );
        }
        cx.update(|_, cx| {
            let shell = app.read(cx).plugins.shell.as_ref().unwrap();
            assert_eq!(shell.error.is_some(), fails);
            assert!(shell.tasks.is_empty());
            if fails {
                assert!(
                    shell
                        .error
                        .as_ref()
                        .unwrap()
                        .contains("diagnostic command failed")
                );
            } else {
                assert!(shell.result.is_some());
            }
        });
        if !fails {
            assert!(text(&app, true, cx).contains("Command count: 1"));
        }
    }
}

#[gpui_kit::test]
#[ignore = "stage diagnostics; use verify-desktop.py --phase diagnostics"]
fn desktop_diagnostics_probe(cx: &mut TestAppContext) {
    let driver = std::env::var("CONDUIT_DIAGNOSTIC_DRIVER").unwrap();
    assert!(matches!(driver.as_str(), "explicit-draw" | "effects-only"));
    let samples = number("CONDUIT_PERF_SAMPLES", 20..=2000);
    let warmup = number("CONDUIT_PERF_WARMUP", 1..=200);
    let (store, dataset_info) = dataset();
    let mut trace = Trace {
        file: File::create(std::env::var("CONDUIT_PERF_OUTPUT").unwrap()).unwrap(),
        frames: Rc::new(Cell::new(0)),
        runtimes: vec![],
        sequence: 0,
    };
    trace.write(
        json!({"event": "header", "schema_version": 1, "suite": "diagnostics",
        "platform": "gpui-test-no-gpu", "driver": driver, "samples": samples,
        "warmup": warmup, "pid": std::process::id(), "debug_assertions": cfg!(debug_assertions),
        "dataset": dataset_info, "registered_package_count": PACKAGES,
        "pace_ms": 16, "window_px": [1600, 800]}),
    );
    let (app, cx) =
        fixture_with_frame_counter(cx, packages(PACKAGES), store, Some(trace.frames.clone()));
    assert_eq!(super::performance::counts(&app, cx), (0, 0));
    let initial = trace.state(&app, cx);
    trace.write(json!({"event": "ready", "state": initial}));
    // Match breakdown's pre-activation and cached-view workload before commands.
    for action in ["host-rows", "host-draw"] {
        for index in 0..warmup + samples {
            std::thread::sleep(Duration::from_millis(16));
            trace.stage(&app, cx, action, index, "update", |cx| {
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        if action == "host-rows" {
                            app.sync_chat_rows();
                        } else {
                            cx.notify();
                        }
                    })
                });
                if action == "host-draw" {
                    draw(cx);
                }
                Value::Null
            });
        }
    }
    for index in 0..ACTIVE {
        trace.stage(&app, cx, "open", index, "update", |cx| {
            let id = ViewId::new(format!("{}.summary", owner(index))).unwrap();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| assert!(app.open_right_view(&id, window, cx)))
            });
            draw(cx);
            Value::Null
        });
        let runtime = cx.update(|_, cx| {
            app.read(cx)
                .plugins
                .shell
                .as_ref()
                .unwrap()
                .host
                .runtime_weak(&owner(index))
                .unwrap()
                .upgrade()
                .unwrap()
        });
        trace.runtimes.push(runtime);
    }
    for index in 0..warmup + samples {
        std::thread::sleep(Duration::from_millis(16));
        trace.stage(&app, cx, "cached-draw", index, "update", |cx| {
            cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
            draw(cx);
            Value::Null
        });
    }
    let mut expected = 0;
    for action in ["local", "metadata"] {
        let id = format!("{}.{}", owner(ACTIVE - 1), action);
        for index in 0..warmup + samples {
            std::thread::sleep(Duration::from_millis(16));
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    let shell = app.plugins.shell.as_mut().unwrap();
                    assert!(shell.tasks.is_empty());
                    shell.result = None;
                })
            });
            trace.stage(&app, cx, action, index, "dispatch-effects", |cx| {
                let dispatch_ms = cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        let started = Instant::now();
                        let command = app.plugins.host_command(&id).unwrap();
                        let accepted = app.dispatch_plugin_command(&command, window, cx);
                        (started.elapsed().as_secs_f64() * 1000., accepted)
                    })
                });
                json!({"dispatch_ms": dispatch_ms.0, "accepted": dispatch_ms.1})
            });
            trace.stage(&app, cx, action, index, "settle", |cx| {
                cx.run_until_parked();
                Value::Null
            });
            if driver == "explicit-draw" {
                trace.stage(&app, cx, action, index, "draw", |cx| {
                    cx.update(|window, cx| {
                        let _ = window.draw(cx);
                    });
                    Value::Null
                });
            }
            expected += 1;
            // Check before any further draw can conceal a stale result/snapshot.
            let state = trace.state(&app, cx);
            assert_eq!(state["tasks"], 0);
            assert!(state["result"].is_string());
            assert!(text(&app, true, cx).contains(&format!("Command count: {expected}")));
            trace.write(
                json!({"event": "verified", "action": action, "index": index,
                "expected_count": expected, "state": state}),
            );
        }
    }
    assert_eq!(super::performance::counts(&app, cx), (ACTIVE, ACTIVE));
    let state = trace.state(&app, cx);
    trace.write(json!({"event": "complete", "commands": expected, "stages": trace.sequence, "state": state}));
}
