use super::*;
use gpui_shell::{
    Capabilities, HostModule, HostValue, RuntimeLimits, ShellRuntime, plugin::PluginManager,
    policy::Policy,
};

type Reports = Rc<RefCell<Vec<(String, String, bool, String)>>>;

fn authority(owner: &str, reports: Reports, data: &std::path::Path) -> Policy {
    let identity = owner.to_owned();
    let bound = identity.clone();
    let module = HostModule::new("authority")
        .function("identity", move |_| Ok(HostValue::Str(identity.clone())))
        .function("report", move |args| {
            let allowed = args.boolean(1)?;
            reports.borrow_mut().push((
                bound.clone(),
                args.string(0)?.into(),
                allowed,
                args.string(2)?.into(),
            ));
            Ok(HostValue::Null)
        });
    let policy = Policy::new()
        .with_application("host-will-replace-this")
        .with_host_module(module)
        .unwrap();
    if owner == "probe.a" {
        policy
            .with_capabilities(Capabilities::new().storage(true))
            .with_storage_path(data.join("store.json"))
            .with_host_module(
                HostModule::new("a-only")
                    .function("secret", |_| Ok(HostValue::Str("a-only-allowed".into()))),
            )
            .unwrap()
    } else {
        policy
    }
}

#[gpui_kit::test]
fn manager_injects_distinct_policies_through_await_callbacks_and_unload(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let reports: Reports = Rc::default();
    let mut manager = PluginManager::new(vec![
        fixtures.join("authority-a"),
        fixtures.join("authority-b"),
    ])
    .with_data_home(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/authority-data"));
    assert_eq!(
        manager
            .discover()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        2
    );
    assert!(reports.borrow().is_empty());
    let runtime = ShellRuntime::new_isolated_with_components_and_limits(
        gpui_shell::components::components().unwrap(),
        RuntimeLimits::default(),
    )
    .unwrap();
    runtime.set_editor_files(false);
    let denied = cx.update(|window, cx| {
        manager.load_with_policy(
            &runtime,
            "probe.a",
            |_, _, _| anyhow::bail!("denied before evaluation"),
            window,
            cx,
        )
    });
    assert!(denied.unwrap_err().to_string().contains("denied"));
    assert!(reports.borrow().is_empty());
    for owner in ["probe.a", "probe.b"] {
        cx.update(|window, cx| {
            manager.load_with_policy(
                &runtime,
                owner,
                |manifest, _, data| Ok(authority(manifest.id(), reports.clone(), data)),
                window,
                cx,
            )
        })
        .unwrap();
    }
    assert!(
        manager
            .plugin("probe.a")
            .unwrap()
            .capabilities()
            .has_storage()
    );
    assert!(
        !manager
            .plugin("probe.b")
            .unwrap()
            .capabilities()
            .has_storage()
    );
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.navigation = Some(manager.plugin("probe.a").unwrap().view().clone());
            host.summary = Some(manager.plugin("probe.b").unwrap().view().clone());
            host.model.borrow_mut().right_open = true;
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "probe.a");
    click(cx, "probe.b");
    cx.executor().advance_clock(Duration::from_millis(110));
    draw(cx);
    for owner in ["probe.a", "probe.b"] {
        for stage in ["module", "init", "click", "await"] {
            let reports = reports.borrow();
            let row = reports
                .iter()
                .find(|row| row.0 == owner && row.1 == stage)
                .unwrap_or_else(|| panic!("missing {owner}/{stage}: {reports:?}"));
            assert_eq!(row.2, owner == "probe.a", "{row:?}");
            assert!(std::path::Path::new(&row.3).ends_with(owner), "{row:?}");
        }
    }
    assert!(
        reports
            .borrow()
            .iter()
            .any(|row| row.0 == "probe.a" && row.1 == "a-only-allowed")
    );
    assert!(
        reports
            .borrow()
            .iter()
            .any(|row| row.0 == "probe.b" && row.1 == "a-only-denied")
    );
    assert!(manager.unload("probe.a"));
    cx.executor().advance_clock(Duration::from_millis(110));
    draw(cx);
    assert!(
        !reports
            .borrow()
            .iter()
            .any(|row| row.0 == "probe.a" && row.1 == "late")
    );
    assert!(
        reports
            .borrow()
            .iter()
            .any(|row| row.0 == "probe.b" && row.1 == "late")
    );
    // Reload the same ID with a fresh host binding. Retained callbacks from the
    // previous generation must never dispatch through this new registry.
    let fresh: Reports = Rc::default();
    let previous_count = reports.borrow().len();
    cx.update(|window, cx| {
        manager.load_with_policy(
            &runtime,
            "probe.a",
            |manifest, _, data| Ok(authority(manifest.id(), fresh.clone(), data)),
            window,
            cx,
        )
    })
    .unwrap();
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.navigation = Some(manager.plugin("probe.a").unwrap().view().clone());
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "probe.a");
    cx.executor().advance_clock(Duration::from_millis(110));
    draw(cx);
    assert!(fresh.borrow().iter().any(|row| row.1 == "await"));
    assert!(fresh.borrow().iter().any(|row| row.1 == "click"));
    assert_eq!(reports.borrow().len(), previous_count);
    assert!(manager.unload("probe.a"));
    assert!(manager.unload("probe.b"));
    let count = reports.borrow().len();
    let fresh_count = fresh.borrow().len();
    click(cx, "probe.a");
    click(cx, "probe.b");
    assert_eq!(
        reports.borrow().len(),
        count,
        "retained views must not revive unloaded callbacks"
    );
    assert_eq!(fresh.borrow().len(), fresh_count);
    runtime.stop();
    let weak = Rc::downgrade(&runtime);
    drop(manager);
    drop(runtime);
    close(host, cx);
    assert!(weak.upgrade().is_none());
}

fn fault_root(name: &str, source: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join(format!("fault-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("gpui-shell.json"), r#"{"id":"probe.fault","name":"Fault probe","entry":"main.js","capabilities":{"storage":false}}"#).unwrap();
    std::fs::write(root.join("main.js"), source).unwrap();
    root
}

fn fault(
    cx: &mut VisualTestContext,
    host: &Entity<Probe>,
    name: &str,
    source: &str,
    expected: &str,
    limits: RuntimeLimits,
) {
    let previous = cx.update(|_, cx| host.read(cx).summary.clone());
    let runtime = ShellRuntime::new_isolated_with_components_and_limits(
        gpui_shell::components::components().unwrap(),
        limits.clone(),
    )
    .unwrap();
    runtime.set_editor_files(false);
    let before = runtime.heap_usage_bytes();
    let root = fault_root(name, source);
    let started = std::time::Instant::now();
    let loaded = cx.update(|window, cx| {
        runtime.load_view_with_policy(
            &root,
            "main.js",
            Rc::new(Policy::new().with_application(name)),
            window,
            cx,
        )
    });
    let mut load_error = None;
    match loaded {
        Ok(view) => {
            cx.update(|_, cx| {
                host.update(cx, |host, cx| {
                    host.summary = Some(view);
                    host.model.borrow_mut().right_open = true;
                    cx.notify();
                })
            });
            draw(cx);
            if name == "event" {
                click(cx, "fault-loop");
            }
            if name == "timer" {
                cx.executor().advance_clock(Duration::from_millis(10));
                draw(cx);
            }
        }
        Err(error) => load_error = Some(format!("{error:#}")),
    }
    let elapsed = started.elapsed();
    let error = runtime
        .resource_error()
        .or(load_error)
        .unwrap_or_else(|| panic!("{name} did not reach its resource limit"));
    println!(
        "G5 {name}: elapsed_ms={}, heap_before={}, heap_after={}, fault={error}",
        elapsed.as_millis(),
        before,
        runtime.heap_usage_bytes()
    );
    assert!(error.contains(expected), "{name}: {error}");
    assert!(
        elapsed < Duration::from_secs(2),
        "{name} exceeded outer bound: {elapsed:?}"
    );
    assert!(runtime.heap_usage_bytes() <= limits.heap_bytes);
    let retry = cx.update(|window, cx| {
        runtime.load_view_with_policy(&root, "main.js", Rc::new(Policy::new()), window, cx)
    });
    assert!(retry.is_err(), "faulted VM must reject reentry");
    runtime.stop();
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.summary = previous;
            cx.notify();
        })
    });
    draw(cx);
    let weak = Rc::downgrade(&runtime);
    drop(runtime);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none(), "faulted VM leaked: {name}");
}

#[gpui_kit::test]
#[ignore = "intentional heap/CPU/log pressure; run with the isolated resources phase"]
fn resource_faults_are_bounded_terminal_and_recoverable(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    let cases = [
        ("module", "while (true) {}", "execution deadline"),
        (
            "event",
            "import {View,div} from 'gpui-kit'; import {Button} from 'gpui-component'; export default class extends View { render() { return div().child(Button.new('fault-loop').label('Loop').on_click(() => { while(true) {} })); } }",
            "execution deadline",
        ),
        (
            "timer",
            "import {View,div} from 'gpui-kit'; export default class extends View { init(_props,cx) { cx.spawn(async cx => { await cx.sleep(5); while(true) {} }, {owner:null}); } render() { return div(); } }",
            "execution deadline",
        ),
        (
            "promise",
            "import {View,div} from 'gpui-kit'; export default class extends View { init() { Promise.resolve().then(() => { while(true) {} }); } render() { return div(); } }",
            "execution deadline",
        ),
        (
            "heap",
            "import {View,div} from 'gpui-kit'; export default class extends View { render() { this.kept=[]; for(let i=0;i<128;i++) this.kept.push(new ArrayBuffer(1024*1024)); return div(); } }",
            "heap limit",
        ),
        (
            "nodes",
            "import {View,div} from 'gpui-kit'; export default class extends View { render() { const root=div(); for(let i=0;i<3000;i++) root.child(div()); return root; } }",
            "native description",
        ),
        (
            "logs",
            "import {View,div} from 'gpui-kit'; export default class extends View { render() { for(let i=0;i<200;i++) console.info('bounded log'); return div(); } }",
            "script log",
        ),
        (
            "data",
            "import {View,div} from 'gpui-kit'; export default class extends View { render() { console.log(Array.from({length:100}, () => Array(1000).fill(1))); return div(); } }",
            "data conversion",
        ),
        (
            "depth",
            "import {View,div} from 'gpui-kit'; export default class extends View { render() { let root=div(); for(let i=0;i<80;i++) root=div().child(root); return root; } }",
            "snapshot expansion/depth",
        ),
    ];
    for (name, source, expected) in cases {
        let limits = RuntimeLimits {
            heap_bytes: 32 << 20,
            render: Duration::from_millis(150),
            event: Duration::from_millis(150),
            execution_per_window: Duration::from_millis(500),
            ..RuntimeLimits::default()
        };
        fault(cx, &host, name, source, expected, limits);
        assert!(
            text(&host, false, cx).contains("Workspace A"),
            "fault in one VM affected the other"
        );
    }
    // Explicit activation creates a fresh VM and proves the native host survived.
    click(cx, "probe-stop");
    click(cx, "probe-status");
    assert!(text(&host, true, cx).contains("Script summary"));
    close(host, cx);
}

#[gpui_kit::test]
#[ignore = "controlled cumulative CPU probe; run with the isolated resources phase"]
fn short_calls_share_a_cumulative_budget(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let limits = RuntimeLimits {
        render: Duration::from_millis(100),
        event: Duration::from_millis(100),
        execution_per_window: Duration::from_millis(80),
        ..RuntimeLimits::default()
    };
    let runtime = ShellRuntime::new_isolated_with_components_and_limits(
        gpui_shell::components::components().unwrap(),
        limits,
    )
    .unwrap();
    runtime.set_editor_files(false);
    let root = fault_root(
        "cumulative",
        "import {View,div} from 'gpui-kit'; export default class extends View { render() { const until=Date.now()+8; while(Date.now()<until) {} return div().child('chunk'); } }",
    );
    let view = cx
        .update(|window, cx| {
            runtime.load_view_with_policy(&root, "main.js", Rc::new(Policy::new()), window, cx)
        })
        .unwrap();
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.summary = Some(view.clone());
            host.model.borrow_mut().right_open = true;
            cx.notify();
        })
    });
    for _ in 0..20 {
        cx.update(|_, cx| view.update(cx, |view, cx| view.refresh(cx)));
        draw(cx);
        if runtime.resource_error().is_some() {
            break;
        }
    }
    println!("G5 cumulative: {}", runtime.resource_error().unwrap());
    assert!(runtime.resource_error().unwrap().contains("cumulative"));
    runtime.stop();
    drop(view);
    drop(runtime);
    close(host, cx);
}

#[gpui_kit::test]
#[ignore = "bounded native stall; run with the isolated resources phase"]
fn native_work_is_charged_after_return_and_cannot_revive_host_calls(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let calls = Rc::new(std::cell::Cell::new(0));
    let after = calls.clone();
    let policy = Policy::new()
        .with_host_module(
            HostModule::new("bounded")
                .function("slow", |_| {
                    std::thread::sleep(Duration::from_millis(30));
                    Ok(HostValue::Null)
                })
                .function("after", move |_| {
                    after.set(after.get() + 1);
                    Ok(HostValue::Null)
                }),
        )
        .unwrap();
    let limits = RuntimeLimits {
        render: Duration::from_millis(10),
        ..RuntimeLimits::default()
    };
    let runtime = ShellRuntime::new_isolated_with_components_and_limits(
        gpui_shell::components::components().unwrap(),
        limits,
    )
    .unwrap();
    runtime.set_editor_files(false);
    let root = fault_root(
        "native",
        "import {View,div} from 'gpui-kit'; import {slow,after} from 'bounded'; export default class extends View { render() { try { slow(); } catch(_) {} try { after(); } catch(_) {} return div(); } }",
    );
    let view = cx
        .update(|window, cx| {
            runtime.load_view_with_policy(&root, "main.js", Rc::new(policy), window, cx)
        })
        .unwrap();
    let started = std::time::Instant::now();
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.summary = Some(view);
            host.model.borrow_mut().right_open = true;
            cx.notify();
        })
    });
    draw(cx);
    let elapsed = started.elapsed();
    println!(
        "G5 native return boundary: elapsed_ms={}, {}",
        elapsed.as_millis(),
        runtime.resource_error().unwrap()
    );
    assert!(elapsed >= Duration::from_millis(30) && elapsed < Duration::from_millis(500));
    assert!(
        runtime
            .resource_error()
            .unwrap()
            .contains("execution deadline")
    );
    assert_eq!(
        calls.get(),
        0,
        "a caught resource exception must not allow a later zero-argument host call"
    );
    let fault = runtime.resource_diagnostic().unwrap();
    for detail in [
        "entry=render",
        "elapsed_ms=",
        "limit_ms=10.000",
        "native_calls=",
        "bridge_values=",
        "site=",
    ] {
        assert!(fault.contains(detail), "missing {detail}: {fault}");
    }
    runtime.stop();
    assert_eq!(
        runtime.resource_diagnostic().unwrap(),
        fault,
        "stop must preserve the first fault's evidence"
    );
    drop(runtime);
    close(host, cx);
}
