//! Independent stress workloads; all timing is CPU/layout on the GPUI test platform.
use super::performance::{counts, memory, number, sample};
use super::*;
use crate::domain::{ChatItemData, ItemKind};
use conduit::plugins::ids::ViewId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, time::Instant};

pub(super) const PACKAGES: usize = 20;
pub(super) const ACTIVE: usize = 3;
pub(super) const SESSIONS: usize = 24;
pub(super) const MESSAGES: usize = 120;
pub(super) const PARAGRAPH: &str = "固定工作负载：中文与 emoji 😀。 Review the plugin lifecycle and preserve host input.\n\n- deterministic item\n- bounded data\n\n```rust\nlet count = 42;\n```\n\n";

pub(super) fn owner(index: usize) -> PluginId {
    PluginId::new(format!("stress.plugin-{index:02}")).unwrap()
}

pub(super) fn dataset() -> (crate::store::Store, Value) {
    let mut store = store();
    let mut digest = Sha256::new();
    let mut bytes = 0;
    let mut longest = 0;
    for (workspace, item) in store.workspaces.iter_mut().enumerate() {
        item.sessions = (0..SESSIONS)
            .map(|index| {
                let mut session = crate::app::tests::session();
                session.id = (workspace * SESSIONS + index + 1) as u64;
                session.title = format!("G5 stress session {:02}", session.id);
                session.messages = (0..MESSAGES)
                    .map(|message| {
                        let text = PARAGRAPH.repeat(if message % 10 == 9 { 64 } else { 4 });
                        bytes += text.len();
                        longest = longest.max(text.len());
                        digest.update(text.as_bytes());
                        ChatItemData {
                            reasoning_summary: None,
                            key: format!("stress-{}-{message}", session.id),
                            created_at: message as u64 + 1,
                            kind: if message % 2 == 0 {
                                ItemKind::User
                            } else {
                                ItemKind::Assistant
                            },
                            text,
                            elapsed_ms: None,
                            input: String::new(),
                            detail: String::new(),
                            status: Default::default(),
                            approval: None,
                            model: None,
                        }
                        .into()
                    })
                    .collect();
                session
            })
            .collect();
    }
    let metadata = json!({
        "id": "long-chat-v1", "workspaces": 2, "sessions_per_workspace": SESSIONS,
        "messages_per_session": MESSAGES, "total_messages": 2 * SESSIONS * MESSAGES,
        "text_bytes": bytes, "longest_message_bytes": longest,
        "text_sha256": format!("{:x}", digest.finalize()),
    });
    (store, metadata)
}

pub(super) fn packages(count: usize) -> Vec<ApprovedPackage> {
    let root = PathBuf::from(std::env::var_os("CONDUIT_STRESS_PACKAGES").unwrap());
    let staging = PathBuf::from(std::env::var_os("CONDUIT_DATA_DIR").unwrap()).join("snapshots");
    (0..count)
        .map(|index| {
            let snapshot =
                PackageSnapshot::prepare(&root.join(owner(index).as_str()), &staging).unwrap();
            assert_eq!(snapshot.manifest().owner(), &owner(index));
            let grants = snapshot.manifest().permissions().clone();
            ApprovedPackage::new(snapshot, grants).unwrap()
        })
        .collect()
}

fn increment(app: &Entity<ConduitApp>, index: usize, expected: usize, cx: &mut VisualTestContext) {
    command(app, &format!("{}.increment", owner(index)), cx);
    assert!(text(app, true, cx).contains(&format!("Command count: {expected}")));
}

pub(super) fn activate(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    for index in 0..ACTIVE {
        increment(app, index, 1, cx);
    }
    assert_eq!(counts(app, cx), (ACTIVE, ACTIVE));
}

pub(super) fn weak_runtimes(
    app: &Entity<ConduitApp>,
    cx: &mut VisualTestContext,
) -> Vec<std::rc::Weak<gpui_shell::ShellRuntime>> {
    cx.update(|_, cx| {
        let host = &app.read(cx).plugins.shell.as_ref().unwrap().host;
        (0..ACTIVE)
            .map(|index| host.runtime_weak(&owner(index)).unwrap())
            .collect()
    })
}

pub(super) fn stop(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.close_right_view(window, cx);
            app.plugins.stop_shell();
            cx.notify();
        })
    });
    draw(cx);
    draw(cx);
    assert_eq!(counts(app, cx), (0, 0));
}

fn fault(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> Value {
    let healthy = weak_runtimes(app, cx);
    let bad = owner(PACKAGES - 1);
    let weak = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let id = ViewId::new(format!("{bad}.summary")).unwrap();
            assert!(app.open_right_view(&id, window, cx));
            app.plugins
                .shell
                .as_ref()
                .unwrap()
                .host
                .runtime_weak(&bad)
                .unwrap()
        })
    });
    draw(cx);
    let started = Instant::now();
    let error = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let action = app
                .plugins
                .host_command(&format!("{bad}.increment"))
                .unwrap();
            assert!(!app.dispatch_plugin_command(&action, window, cx));
            app.plugins
                .shell
                .as_ref()
                .unwrap()
                .error
                .clone()
                .expect("fault reported")
        })
    });
    draw(cx);
    let elapsed = started.elapsed().as_secs_f64() * 1000.;
    assert!(error.contains("execution deadline"), "{error}");
    assert!(weak.upgrade().is_none());
    for index in 0..ACTIVE {
        increment(app, index, 2, cx);
    }
    let current = weak_runtimes(app, cx);
    assert!(
        healthy
            .iter()
            .zip(&current)
            .all(|(old, now)| old.ptr_eq(now) && old.upgrade().is_some())
    );
    assert_eq!(counts(app, cx), (ACTIVE, ACTIVE));
    json!({"owner": bad.as_str(), "kind": "sync-command-deadline", "error": error,
        "elapsed_ms": elapsed, "runtime_released": true, "healthy_runtimes_preserved": ACTIVE,
        "healthy_commands_after_fault": ACTIVE, "remaining_runtime_count": ACTIVE,
        "remaining_instance_count": ACTIVE})
}

#[gpui_kit::test]
#[ignore = "stress and intentional watchdog fault; use verify-desktop.py --phase stress"]
fn desktop_stress_probe(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let scenario = std::env::var("CONDUIT_PERF_SCENARIO").unwrap();
    assert!(matches!(
        scenario.as_str(),
        "baseline" | "registered" | "active" | "mixed" | "lifecycle" | "idle"
    ));
    let samples = number("CONDUIT_PERF_SAMPLES", 20..=2000);
    let warmup = number("CONDUIT_PERF_WARMUP", 1..=200);
    let cycles = number("CONDUIT_STRESS_CYCLES", 3..=200);
    let registered = if scenario == "baseline" { 0 } else { PACKAGES };
    let active = if matches!(scenario.as_str(), "active" | "mixed") {
        ACTIVE
    } else {
        0
    };
    let before = memory();
    let (store, dataset) = dataset();
    let (app, cx) = fixture_with_store(cx, packages(registered), store);
    assert_eq!(counts(&app, cx), (0, 0));
    let registered_count = cx.update(|_, cx| {
        let app = app.read(cx);
        let session = app.session(app.selected_session.unwrap()).unwrap();
        assert_eq!(session.messages.len(), MESSAGES);
        assert!(session.transcript.loaded);
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
    let registered_memory = memory();
    if active > 0 {
        activate(&app, cx);
    }
    let failure = if scenario == "mixed" {
        fault(&app, cx)
    } else {
        Value::Null
    };
    cx.update(|_, cx| {
        let plugins = &app.read(cx).plugins;
        assert!(plugins.is_selected(builtins::WORKSPACES));
        assert_eq!(plugins.right_open(), active > 0);
    });
    let ready = memory();
    let mut alternate = false;
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    draw(cx);
    let edit = sample(warmup, samples, || {
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
    let notify = sample(warmup, samples, || {
        cx.update(|_, cx| app.update(cx, |_, cx| cx.notify()));
        draw(cx);
    });
    let mut timings = json!({"host_edit_draw_ms": edit, "host_notify_draw_ms": notify});
    if active > 0 {
        let mut counter = if scenario == "mixed" { 2 } else { 1 };
        timings["three_plugin_commands_draw_ms"] = json!(sample(warmup, samples, || {
            counter += 1;
            for index in 0..ACTIVE {
                command(&app, &format!("{}.increment", owner(index)), cx);
            }
        }));
        // Validate cached state for every owner outside the timed command loop.
        for index in 0..ACTIVE {
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
            assert!(text(&app, true, cx).contains(&format!("Command count: {counter}")));
        }
    }
    assert_eq!(counts(&app, cx), (active, active));
    let after_samples = memory();
    let mut lifecycle = Vec::new();
    if matches!(scenario.as_str(), "lifecycle" | "idle") {
        for cycle in 0..cycles {
            std::thread::sleep(Duration::from_millis(16));
            let started = Instant::now();
            if scenario == "lifecycle" {
                activate(&app, cx);
            } else {
                draw(cx);
            }
            let activate_ms = started.elapsed().as_secs_f64() * 1000.;
            let cycle_active = if scenario == "lifecycle" { ACTIVE } else { 0 };
            assert_eq!(counts(&app, cx), (cycle_active, cycle_active));
            let ready = memory();
            let weak = if cycle_active > 0 {
                weak_runtimes(&app, cx)
            } else {
                vec![]
            };
            let started = Instant::now();
            stop(&app, cx);
            let stop_ms = started.elapsed().as_secs_f64() * 1000.;
            assert!(weak.iter().all(|vm| vm.upgrade().is_none()));
            let stopped = memory();
            // A fixed settled sample, not an assertion that the OS returned pages or GC ran.
            std::thread::sleep(Duration::from_millis(16));
            draw(cx);
            draw(cx);
            lifecycle.push(
                json!({"cycle": cycle + 1, "active_runtime_count": cycle_active,
                "active_instance_count": cycle_active, "stopped_runtime_count": 0,
                "stopped_instance_count": 0, "released_runtime_count": weak.len(),
                "activate_draw_ms": activate_ms, "stop_draw_ms": stop_ms,
                "ready": ready, "after_stop": stopped, "settled": memory()}),
            );
        }
    }
    let report = json!({"schema_version": 1, "suite": "stress", "scenario": scenario,
        "platform": "gpui-test-no-gpu", "debug_assertions": cfg!(debug_assertions),
        "pid": std::process::id(), "window_px": [1600, 800], "samples": samples,
        "warmup": warmup, "pace_ms": 16, "cycles": cycles, "dataset": dataset,
        "registered_package_count": registered_count, "runtime_count": active,
        "instance_count": active, "registered_runtime_count": 0, "registered_instance_count": 0,
        "visible_plugin_view_count": usize::from(active > 0),
        "fault": failure, "lifecycle": lifecycle, "timings": timings,
        "memory": {"before_fixture": before, "registered": registered_memory,
            "ready": ready, "after_samples": after_samples}});
    std::fs::write(
        std::env::var("CONDUIT_PERF_OUTPUT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}
