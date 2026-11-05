use super::*;
use crate::commands::{ViewCommandCall, ViewCommandReply};
use gpui_shell::{
    CommandCall, CommandTarget, HostModule, HostValue, RuntimeLimits, ShellRuntime, policy::Policy,
    view::ScriptView,
};

const SOURCE: &str = r#"
import {View,div} from 'gpui-kit';
import {identity,ping} from 'command-owner';
export default class extends View {
  init() { this.count = 0; }
  render() { return div().child(identity() + ':' + this.count); }
  on_command(name, payload, cx) {
    switch(name) {
      case 'increment': this.count++; cx.notify(); return {owner:identity(), count:this.count};
      case 'count': return {owner:identity(), count:this.count};
      case 'echo': return payload;
      case 'delayed': return cx.sleep(20).then(() => ({owner:identity(), count:this.count}));
      case 'mutate-late': return new Promise(resolve => cx.spawn(async cx => {
        await cx.sleep(20); this.count++; ping(); cx.notify(); resolve({owner:identity(), count:this.count});
      }));
      case 'ownerless': cx.spawn(async cx => {await cx.sleep(20); ping();}, {owner:null}); return null;
      case 'pending': return new Promise(() => {});
      case 'reject': return Promise.reject(new Error('command rejected'));
      case 'bad-async': return Promise.resolve({callback:()=>{}});
      case 'cycle': { const a={}; a.self=a; return a; }
      case 'huge': return new Array(1000000);
      case 'bytes': return 'x'.repeat(262145);
      case 'bad-number': return Infinity;
      case 'throw': throw new Error('synchronous failure');
      case 'huge-error': return Promise.reject(new Error('中文'.repeat(4000)));
      case 'stale-cx': return cx.sleep(20).then(() => cx.notify());
      case 'spin': while(true) {}
      case 'spin-later': return cx.sleep(20).then(() => {while(true) {}});
      default: throw new Error('unknown command');
    }
  }
}
"#;

fn runtime() -> Rc<ShellRuntime> {
    let runtime = ShellRuntime::new_isolated_with_components_and_limits(
        gpui_shell::components::components().unwrap(),
        RuntimeLimits::default(),
    )
    .unwrap();
    runtime.set_editor_files(false);
    runtime
}

fn policy(owner: &str, hits: Rc<RefCell<Vec<String>>>) -> Rc<Policy> {
    let identity = owner.to_owned();
    let bound = identity.clone();
    Rc::new(
        Policy::new()
            .with_application(owner)
            .with_host_module(
                HostModule::new("command-owner")
                    .function("identity", move |_| Ok(HostValue::Str(identity.clone())))
                    .function("ping", move |_| {
                        hits.borrow_mut().push(bound.clone());
                        Ok(HostValue::Null)
                    }),
            )
            .unwrap(),
    )
}

fn root() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("artifacts")
        .join(format!("commands-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.js"), SOURCE).unwrap();
    std::fs::write(root.join("right.js"), SOURCE).unwrap();
    std::fs::write(root.join("gpui-shell.json"), r#"{"id":"commands.reload","name":"Command reload probe","entry":"main.js","capabilities":{"storage":false}}"#).unwrap();
    root
}

fn load(
    runtime: &Rc<ShellRuntime>,
    policy: Rc<Policy>,
    entry: &str,
    cx: &mut VisualTestContext,
) -> (Entity<ScriptView>, CommandTarget) {
    cx.update(|window, cx| {
        let view = runtime
            .load_view_with_policy(&root(), entry, policy, window, cx)
            .unwrap();
        let target = runtime.command_target(&view, cx).unwrap();
        (view, target)
    })
}

fn dispatch(target: &CommandTarget, name: &str, cx: &mut VisualTestContext) -> CommandCall {
    cx.update(|window, cx| target.dispatch(name, HostValue::Null, window, cx))
        .unwrap()
}

fn take(call: &mut CommandCall, cx: &mut VisualTestContext) -> Result<HostValue> {
    cx.update(|_, cx| call.try_take(cx))
        .expect("command must have settled")
}

fn count(target: &CommandTarget, cx: &mut VisualTestContext) -> (String, f64) {
    let value = take(&mut dispatch(target, "count", cx), cx).unwrap();
    (
        value.get("owner").unwrap().as_str().unwrap().into(),
        value.get("count").unwrap().as_number().unwrap(),
    )
}

fn receive(
    call: ViewCommandCall,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Option<ViewCommandReply>>> {
    let result = Rc::new(RefCell::new(None));
    let output = result.clone();
    cx.update(|_, cx| {
        cx.spawn(async move |_| {
            *output.borrow_mut() = Some(call.wait().await);
        })
        .detach()
    });
    result
}

#[gpui_kit::test]
fn footer_targets_summary_without_moving_host_focus(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    click(cx, "probe-host-focus");
    cx.update(|window, cx| assert!(host.read(cx).content_focus.is_focused(window)));
    click(cx, "probe-command-summary");
    assert_eq!(label(cx, "shell-summary-counter"), "Independent counter: 1");
    assert_eq!(label(cx, "shell-open-summary"), "Open summary (0)");
    cx.update(|window, cx| {
        assert!(host.read(cx).content_focus.is_focused(window));
        assert_eq!(host.read(cx).command_result, "Summary command result: 1");
    });
    // The same host path can explicitly target the left instance.
    let call = cx
        .update(|window, cx| {
            host.read(cx).navigation_command.clone().unwrap().dispatch(
                "increment",
                HostValue::Number(2.),
                window,
                cx,
            )
        })
        .unwrap();
    let output = receive(call, cx);
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            output
                .borrow_mut()
                .take()
                .unwrap()
                .accept(cx)
                .unwrap()
                .as_number(),
            Some(2.)
        )
    });
    assert_eq!(label(cx, "shell-open-summary"), "Open summary (2)");
    assert_eq!(label(cx, "shell-summary-counter"), "Independent counter: 1");
    close(host, cx);
}

#[gpui_kit::test]
fn two_owners_keep_command_state_authority_and_stop_boundaries(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let hits = Rc::new(RefCell::new(Vec::new()));
    let a = runtime();
    let b = runtime();
    let a_policy = policy("commands.a", hits.clone());
    let (left, left_target) = load(&a, a_policy.clone(), "main.js", cx);
    let (right, right_target) = load(&a, a_policy, "right.js", cx);
    let (other, other_target) = load(&b, policy("commands.b", hits.clone()), "main.js", cx);
    cx.update(|_, cx| assert!(a.command_target(&other, cx).is_err()));
    take(&mut dispatch(&right_target, "increment", cx), cx).unwrap();
    assert_eq!(count(&left_target, cx), ("commands.a".into(), 0.));
    assert_eq!(count(&right_target, cx), ("commands.a".into(), 1.));
    assert_eq!(count(&other_target, cx), ("commands.b".into(), 0.));
    let mut delayed_a = dispatch(&right_target, "delayed", cx);
    let mut delayed_b = dispatch(&other_target, "delayed", cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(30));
    draw(cx);
    assert_eq!(
        take(&mut delayed_a, cx)
            .unwrap()
            .get("owner")
            .unwrap()
            .as_str(),
        Some("commands.a")
    );
    assert_eq!(
        take(&mut delayed_b, cx)
            .unwrap()
            .get("owner")
            .unwrap()
            .as_str(),
        Some("commands.b")
    );
    let mut pending = dispatch(&right_target, "mutate-late", cx);
    let mut buffered = dispatch(&left_target, "count", cx);
    take(&mut dispatch(&left_target, "ownerless", cx), cx).unwrap();
    take(&mut dispatch(&other_target, "ownerless", cx), cx).unwrap();
    cx.run_until_parked();
    a.stop();
    assert!(take(&mut pending, cx).is_err());
    assert!(take(&mut buffered, cx).is_err());
    for target in [&left_target, &right_target] {
        cx.update(|window, cx| {
            assert!(
                target
                    .dispatch("increment", HostValue::Null, window, cx)
                    .is_err()
            )
        });
    }
    cx.executor().advance_clock(Duration::from_millis(30));
    draw(cx);
    assert_eq!(&*hits.borrow(), &["commands.b"]);
    take(&mut dispatch(&other_target, "increment", cx), cx).unwrap();
    assert_eq!(count(&other_target, cx).1, 1.);
    let weak = Rc::downgrade(&a);
    drop((left, right, a));
    draw(cx);
    assert!(
        weak.upgrade().is_none(),
        "command handles must not retain the stopped VM"
    );
    // Same owner, new VM: retained old handles never redirect to it.
    let fresh = runtime();
    let (fresh_view, fresh_target) = load(&fresh, policy("commands.a", hits), "main.js", cx);
    assert_eq!(count(&fresh_target, cx).1, 0.);
    cx.update(|window, cx| {
        assert!(
            right_target
                .dispatch("increment", HostValue::Null, window, cx)
                .is_err()
        )
    });
    fresh.stop();
    b.stop();
    drop((fresh_view, other, fresh, b));
    close(host, cx);
}

#[gpui_kit::test]
fn commands_settle_errors_and_bound_both_directions(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let vm = runtime();
    let (view, target) = load(&vm, policy("commands.data", Rc::default()), "main.js", cx);
    let payload = HostValue::Object(vec![(
        "items".into(),
        HostValue::Array(vec![HostValue::Bool(true), HostValue::Number(3.)]),
    )]);
    let mut call = cx
        .update(|window, cx| target.dispatch("echo", payload, window, cx))
        .unwrap();
    assert!(take(&mut call, cx).unwrap().get("items").is_some());
    for name in [
        "reject",
        "bad-async",
        "cycle",
        "huge",
        "bytes",
        "bad-number",
        "huge-error",
    ] {
        let error = take(&mut dispatch(&target, name, cx), cx)
            .unwrap_err()
            .to_string();
        assert!(
            error.len() <= 4096,
            "{name} produced an unbounded diagnostic"
        );
    }
    for name in ["throw", "unknown", "", "bad command", &"x".repeat(129)] {
        cx.update(|window, cx| {
            assert!(target.dispatch(name, HostValue::Null, window, cx).is_err())
        });
    }
    let mut nested = HostValue::Null;
    for _ in 0..18 {
        nested = HostValue::Array(vec![nested]);
    }
    for payload in [
        HostValue::Number(f64::NAN),
        HostValue::Str("x".repeat(262145)),
        HostValue::Array(vec![HostValue::Null; 4096]),
        nested,
    ] {
        cx.update(|window, cx| assert!(target.dispatch("echo", payload, window, cx).is_err()));
    }
    // Conversion failures do not leave a pending reply or poison an otherwise healthy VM.
    assert_eq!(count(&target, cx).1, 0.);
    let mut stale_cx = dispatch(&target, "stale-cx", cx);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(30));
    draw(cx);
    assert!(
        take(&mut stale_cx, cx).is_err(),
        "call-scoped cx must not survive await"
    );
    vm.stop();
    drop((view, vm));
    close(host, cx);
}

#[gpui_kit::test]
fn workspace_tickets_reject_queued_results_and_old_targets(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    let target = cx.update(|_, cx| host.read(cx).summary_command.clone().unwrap());
    let call = cx
        .update(|window, cx| target.dispatch("increment", HostValue::Number(1.), window, cx))
        .unwrap();
    let queued = receive(call, cx);
    draw(cx);
    assert!(queued.borrow().is_some());
    // Invalidate only the host context, keeping the VM live: acceptance must
    // fail on the host ticket independently of Shell's VM generation check.
    cx.update(|_, cx| {
        host.read(cx)
            .model
            .borrow_mut()
            .select_workspace(1)
            .unwrap()
    });
    cx.update(|_, cx| assert!(queued.borrow_mut().take().unwrap().accept(cx).is_err()));
    cx.update(|window, cx| {
        assert!(
            target
                .dispatch("increment", HostValue::Number(1.), window, cx)
                .is_err()
        )
    });
    click(cx, "probe-workspace-a");
    let target = cx.update(|_, cx| host.read(cx).summary_command.clone().unwrap());
    let call = cx
        .update(|window, cx| target.dispatch("read-after-delay", HostValue::Null, window, cx))
        .unwrap();
    let delayed = receive(call, cx);
    cx.run_until_parked();
    click(cx, "probe-workspace-b");
    cx.executor().advance_clock(Duration::from_millis(1100));
    draw(cx);
    cx.update(|_, cx| {
        assert!(
            delayed
                .borrow_mut()
                .take()
                .expect("stop settles pending waiter")
                .accept(cx)
                .is_err()
        )
    });
    assert_eq!(label(cx, "shell-summary-counter"), "Independent counter: 0");
    assert!(text(&host, true, cx).contains("Workspace B"));
    click(cx, "probe-stop");
    click(cx, "probe-status");
    cx.update(|window, cx| {
        assert!(
            target
                .dispatch("increment", HostValue::Number(1.), window, cx)
                .is_err()
        )
    });
    close(host, cx);
}

#[gpui_kit::test]
fn dropping_a_view_cancels_pending_commands_without_retaining_its_runtime(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let vm = runtime();
    let (view, target) = load(&vm, policy("commands.drop", Rc::default()), "main.js", cx);
    let mut call = dispatch(&target, "pending", cx);
    drop(view);
    draw(cx);
    assert!(take(&mut call, cx).is_err());
    let weak = Rc::downgrade(&vm);
    drop(vm);
    draw(cx);
    assert!(weak.upgrade().is_none());
    close(host, cx);
}

#[gpui_kit::test]
fn unloading_invalidates_retained_views_and_same_id_reactivation(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    let vm = runtime();
    let mut manager =
        gpui_shell::plugin::PluginManager::new(vec![root()]).with_data_home(root().join("data"));
    assert_eq!(
        manager
            .discover()
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        1
    );
    let hits = Rc::new(RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        manager.load_with_policy(
            &vm,
            "commands.reload",
            |_, _, _| {
                Ok(Rc::try_unwrap(policy("commands.reload", hits.clone()))
                    .ok()
                    .unwrap())
            },
            window,
            cx,
        )
    })
    .unwrap();
    let old_view = manager.plugin("commands.reload").unwrap().view().clone();
    let target = cx.update(|_, cx| vm.command_target(&old_view, cx)).unwrap();
    let mut buffered = dispatch(&target, "increment", cx);
    let mut pending = dispatch(&target, "mutate-late", cx);
    cx.run_until_parked();
    assert!(manager.unload("commands.reload"));
    assert!(take(&mut buffered, cx).is_err());
    assert!(take(&mut pending, cx).is_err());
    cx.update(|window, cx| {
        manager.load_with_policy(
            &vm,
            "commands.reload",
            |_, _, _| {
                Ok(Rc::try_unwrap(policy("commands.reload", hits.clone()))
                    .ok()
                    .unwrap())
            },
            window,
            cx,
        )
    })
    .unwrap();
    let current = cx
        .update(|_, cx| vm.command_target(manager.plugin("commands.reload").unwrap().view(), cx))
        .unwrap();
    assert_eq!(count(&current, cx).1, 0.);
    cx.update(|window, cx| {
        assert!(
            target
                .dispatch("increment", HostValue::Null, window, cx)
                .is_err()
        )
    });
    cx.update(|_, cx| assert!(vm.command_target(&old_view, cx).is_err()));
    cx.executor().advance_clock(Duration::from_millis(30));
    draw(cx);
    assert!(hits.borrow().is_empty());
    assert_eq!(count(&current, cx).1, 0.);
    vm.stop();
    let weak = Rc::downgrade(&vm);
    drop((old_view, manager, vm));
    draw(cx);
    assert!(weak.upgrade().is_none());
    close(host, cx);
}

#[gpui_kit::test]
#[ignore = "intentional runaway commands; run with the isolated commands phase"]
fn command_watchdogs_stop_sync_and_async_loops(cx: &mut TestAppContext) {
    let (host, cx) = fixture(cx);
    click(cx, "probe-status");
    for name in ["spin", "spin-later"] {
        let vm = runtime();
        let (view, target) = load(
            &vm,
            policy("commands.watchdog", Rc::default()),
            "main.js",
            cx,
        );
        let started = std::time::Instant::now();
        let result = cx.update(|window, cx| target.dispatch(name, HostValue::Null, window, cx));
        if name == "spin" {
            assert!(result.is_err());
        } else {
            let mut call = result.unwrap();
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(30));
            draw(cx);
            assert!(take(&mut call, cx).is_err());
        }
        println!(
            "G3 {name}: elapsed_ms={}, fault={}",
            started.elapsed().as_millis(),
            vm.resource_error().unwrap()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(vm.resource_error().unwrap().contains("execution deadline"));
        cx.update(|window, cx| {
            assert!(
                target
                    .dispatch("count", HostValue::Null, window, cx)
                    .is_err()
            )
        });
        assert!(text(&host, true, cx).contains("Script summary"));
        let weak = Rc::downgrade(&vm);
        drop((view, vm));
        draw(cx);
        assert!(weak.upgrade().is_none());
    }
    close(host, cx);
}
