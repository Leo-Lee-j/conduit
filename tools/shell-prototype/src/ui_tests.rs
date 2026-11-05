use super::{OWNER, Probe, fixture_root};
use anyhow::Result;
use gpui_kit::component::WindowExt;
use gpui_kit::{
    AppContext, ElementId, Entity, Modifiers, TestAppContext, VisualTestContext, px, size,
};
use gpui_shell::ShellRoot;
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

fn fixture(cx: &mut TestAppContext) -> (Entity<Probe>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_shell::init(cx);
    });
    let output = Rc::new(RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let host = cx.new(|cx| Probe::new(cx).unwrap());
        *slot.borrow_mut() = Some(host.clone());
        let shell = cx.new(|cx| ShellRoot::new(host.into(), window, cx));
        gpui_kit::component::Root::new(shell, window, cx)
    });
    cx.simulate_resize(size(px(1200.), px(780.)));
    let host = output.borrow_mut().take().unwrap();
    draw(cx);
    (host, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

fn click(cx: &mut VisualTestContext, id: &str) {
    let bounds = cx.update(|window, _| {
        gpui_kit::base::test_support::find(window, &[], &ElementId::from(id.to_owned()))
            .unwrap_or_else(|| panic!("missing element {id}"))
            .bounds()
    });
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
}

fn label(cx: &mut VisualTestContext, id: &str) -> String {
    cx.update(|window, _| {
        gpui_kit::base::test_support::find(window, &[], &ElementId::from(id.to_owned()))
            .unwrap_or_else(|| panic!("missing element {id}"))
            .label()
            .expect("button label")
            .to_string()
    })
}

fn text(host: &Entity<Probe>, summary: bool, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let host = host.read(cx);
        let view = if summary {
            host.summary.as_ref()
        } else {
            host.navigation.as_ref()
        }
        .unwrap();
        let view = view.read(cx);
        assert!(view.build_error().is_none(), "{:?}", view.build_error());
        view.snapshot()
            .expect("rendered script snapshot")
            .debug_tree()
    })
}

fn close(host: Entity<Probe>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        host.update(cx, |host, cx| host.stop(cx));
        window.remove_window();
    });
    drop(host);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn three_regions_lazily_share_context_and_keep_independent_views(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    cx.update(|_, cx| assert!(host.read(cx).runtime.is_none()));
    click(cx, "probe-status");
    assert!(text(&host, false, cx).contains("Workspace A"));
    assert!(text(&host, true, cx).contains("Workspace A"));
    click(cx, "shell-summary-counter");
    assert_eq!(label(cx, "shell-summary-counter"), "Independent counter: 1");
    assert_eq!(label(cx, "shell-open-summary"), "Open summary (0)");
    click(cx, "shell-open-summary");
    assert_eq!(label(cx, "shell-open-summary"), "Open summary (1)");
    assert_eq!(label(cx, "shell-summary-counter"), "Independent counter: 1");
    click(cx, "probe-workspace-b");
    assert!(text(&host, false, cx).contains("Workspace B"));
    assert!(text(&host, true, cx).contains("Workspace B"));
    assert_eq!(label(cx, "shell-open-summary"), "Open summary (0)");
    assert_eq!(label(cx, "shell-summary-counter"), "Independent counter: 0");
    // The prototype recreates the runtime on scope changes; it does not yet
    // implement the production per-workspace view cache.
    for width in [1200., 850.] {
        cx.simulate_resize(size(px(width), px(780.)));
        draw(cx);
        let right = cx.debug_bounds("probe-right").unwrap();
        assert!(right.right() <= px(width));
        assert_eq!(
            cx.debug_bounds("probe-footer").unwrap().size.height,
            px(34.)
        );
    }
    close(host, cx);
}

#[gpui_kit::test]
fn cached_frames_errors_readonly_and_runtime_release(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    let before = cx.update(|_, cx| host.read(cx).runtime.as_ref().unwrap().read_metrics());
    draw(cx);
    draw(cx);
    let after = cx.update(|_, cx| host.read(cx).runtime.as_ref().unwrap().read_metrics());
    println!(
        "Snapshot sample: renders={}, mean_script_us={}, mean_materialize_us={}, slowest_script_us={}",
        before.script_renders(),
        before.mean_script_render().as_micros(),
        before.mean_materialize().as_micros(),
        before.slowest_script_render().as_micros()
    );
    assert_eq!(
        before.script_renders(),
        after.script_renders(),
        "clean frames must reuse snapshots"
    );
    click(cx, "probe-error");
    cx.update(|_, cx| {
        let view = host.read(cx).summary.as_ref().unwrap().read(cx);
        assert!(
            view.build_error()
                .unwrap()
                .contains("intentional prototype")
        );
        assert!(
            view.snapshot().is_some(),
            "last usable snapshot is retained"
        );
    });
    click(cx, "probe-error");
    assert!(text(&host, true, cx).contains("Script summary"));
    let weak = cx.update(|_, cx| Rc::downgrade(host.read(cx).runtime.as_ref().unwrap()));
    click(cx, "probe-readonly");
    draw(cx);
    cx.update(|_, cx| {
        let state = host.read(cx);
        assert!(!state.model.borrow().active);
        assert!(state.runtime.is_none() && state.navigation.is_none() && state.summary.is_none());
    });
    assert!(
        weak.upgrade().is_none(),
        "runtime must be released after views and frame state retire"
    );
    click(cx, "probe-status");
    cx.update(|_, cx| assert!(host.read(cx).error.as_ref().unwrap().contains("read-only")));
    close(host, cx);
}

#[gpui_kit::test]
fn component_and_shell_overlays_resolve_through_the_same_window(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-host-dialog");
    cx.update(|window, cx| {
        assert!(window.has_active_dialog(cx));
        window.close_dialog(cx);
    });
    draw(cx);
    click(cx, "probe-shell-dialog");
    cx.update(|window, cx| {
        assert_eq!(
            ShellRoot::update(window, cx, |root, _, _| root.dialog_count()),
            Some(1)
        );
        ShellRoot::update(window, cx, |root, window, cx| {
            root.close_all_dialogs(window, cx)
        });
    });
    draw(cx);
    close(host, cx);
}

#[gpui_kit::test]
fn late_script_status_is_rejected_after_context_switch_and_cancelled_on_stop(
    cx: &mut TestAppContext,
) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    click(cx, "shell-delayed-status");
    cx.executor().advance_clock(Duration::from_millis(1100));
    draw(cx);
    assert_eq!(label(cx, "probe-status"), "Updated: Workspace A");
    click(cx, "shell-delayed-status");
    click(cx, "probe-workspace-b");
    cx.executor().advance_clock(Duration::from_millis(1100));
    draw(cx);
    cx.update(|_, cx| assert_eq!(host.read(cx).model.borrow().status, "Workspace overview"));
    assert!(text(&host, false, cx).contains("Workspace B"));
    click(cx, "shell-delayed-status");
    let weak = cx.update(|_, cx| Rc::downgrade(host.read(cx).runtime.as_ref().unwrap()));
    click(cx, "probe-stop");
    cx.executor().advance_clock(Duration::from_millis(1100));
    draw(cx);
    cx.update(|_, cx| assert_eq!(host.read(cx).model.borrow().status, "Not activated"));
    assert!(
        weak.upgrade().is_none(),
        "stopped fixture must not retain its pending script task"
    );
    close(host, cx);
}

#[gpui_kit::test]
fn legacy_manager_requires_explicit_host_module_policy(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let mut manager = gpui_shell::plugin::PluginManager::new(vec![fixture_root()])
        .with_data_home(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/manager-data"));
    assert_eq!(
        manager
            .discover()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(manager.loaded().count(), 0);
    let runtime = gpui_shell::components::new_isolated_runtime().unwrap();
    runtime.set_editor_files(false);
    let denied = cx
        .update(|window, cx| manager.load(&runtime, OWNER, |_| false, window, cx))
        .unwrap_err();
    assert!(denied.to_string().contains("not approved"));
    let error = cx
        .update(|window, cx| manager.load(&runtime, OWNER, |_| true, window, cx))
        .unwrap_err();
    let error = format!("{error:#}");
    assert!(
        error.contains("cannot resolve module") && error.contains("conduit-probe"),
        "unexpected failure: {error}"
    );
    assert_eq!(manager.loaded().count(), 0);
    println!("Legacy load cannot import a module absent from its own Policy: {error}");
    drop(manager);
    drop(runtime);
    close(host, cx);
}

#[gpui_kit::test]
fn dropping_publicly_mounted_root_cancels_ownerless_application_tasks(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    let extra = cx.update(|window, cx| {
        let runtime = host.read(cx).runtime.as_ref().unwrap().clone();
        runtime
            .load_view_with_policy(
                &fixture_root(),
                "lifecycle.js",
                host.read(cx).policy.clone(),
                window,
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    drop(extra);
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(1100));
    draw(cx);
    cx.update(|_, cx| assert_eq!(host.read(cx).model.borrow().lifecycle_callbacks, 0));
    // Positive control: the same task must run while its root stays alive.
    let alive = cx.update(|window, cx| {
        let runtime = host.read(cx).runtime.as_ref().unwrap().clone();
        runtime
            .load_view_with_policy(
                &fixture_root(),
                "lifecycle.js",
                host.read(cx).policy.clone(),
                window,
                cx,
            )
            .unwrap()
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(1100));
    draw(cx);
    cx.update(|_, cx| assert_eq!(host.read(cx).model.borrow().lifecycle_callbacks, 1));
    drop(alive);
    close(host, cx);
}

#[gpui_kit::test]
fn fixture_policy_denies_filesystem_and_shell_storage(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    cx.update(|window, cx| {
        host.update(cx, |host, cx| {
            let runtime = host.runtime.as_ref().unwrap().clone();
            host.summary = Some(
                runtime
                    .load_view_with_policy(
                        &fixture_root(),
                        "capabilities.js",
                        host.policy.clone(),
                        window,
                        cx,
                    )
                    .unwrap(),
            );
            cx.notify();
        });
    });
    draw(cx);
    cx.run_until_parked();
    draw(cx);
    let output = text(&host, true, cx);
    assert!(output.contains("filesystem denied"), "{output}");
    assert!(output.contains("storage denied"), "{output}");
    close(host, cx);
}

#[gpui_kit::test]
#[ignore = "intentional runaway script; run with the isolated watchdog phase"]
fn render_watchdog_interrupts_an_infinite_loop(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    // VisualTestContext may render before update returns.
    let started = std::time::Instant::now();
    cx.update(|window, cx| {
        host.update(cx, |host, cx| {
            let runtime = host.runtime.as_ref().unwrap().clone();
            host.summary = Some(
                runtime
                    .load_view_with_policy(
                        &fixture_root(),
                        "watchdog.js",
                        host.policy.clone(),
                        window,
                        cx,
                    )
                    .unwrap(),
            );
            cx.notify();
        });
    });
    draw(cx);
    let elapsed = started.elapsed();
    let error = cx.update(|_, cx| {
        host.read(cx)
            .summary
            .as_ref()
            .unwrap()
            .read(cx)
            .build_error()
            .unwrap()
            .to_owned()
    });
    println!(
        "G5 render watchdog: elapsed_ms={}, error={error}",
        elapsed.as_millis()
    );
    assert!(error.contains("execution deadline exceeded"), "{error}");
    assert!(
        elapsed < Duration::from_millis(500),
        "watchdog exceeded outer expectation"
    );
    click(cx, "probe-stop");
    click(cx, "probe-status");
    assert!(text(&host, true, cx).contains("Script summary"));
    close(host, cx);
}

#[path = "controls_tests.rs"]
mod controls;

#[path = "commands_tests.rs"]
mod commands;

#[path = "development_tests.rs"]
mod development;
