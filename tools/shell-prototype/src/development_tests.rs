use super::{click, draw};
use crate::development::{DevelopmentProbe, context};
use anyhow::Result;
use conduit_shell_prototype::{
    adapter::{ApprovedPackage, CompletedCommand, DevelopmentHost, HostData, PendingCommand},
    package::PackageSnapshot,
    plugins::{
        context::{ContextInput, SessionKey},
        ids::{CommandId, PluginId, ViewId},
    },
};
use gpui_kit::{AppContext, Entity, TestAppContext, VisualTestContext, px, size};
use gpui_shell::{HostValue, ShellRoot};
use std::{cell::RefCell, collections::BTreeSet, fs, path::Path, rc::Rc, time::Duration};

const OWNER: &str = "example.workspace-overview";

#[path = "overlay_tests.rs"]
mod overlays;

fn source(owner: &str) -> tempfile::TempDir {
    let source = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/development-overview");
    for file in [
        "gpui-shell.json",
        "conduit-plugin.json",
        "navigation.js",
        "summary.js",
    ] {
        fs::write(
            source.path().join(file),
            fs::read_to_string(fixture.join(file))
                .unwrap()
                .replace(OWNER, owner),
        )
        .unwrap();
    }
    source
}

fn snapshot(source: &Path) -> PackageSnapshot {
    PackageSnapshot::prepare(
        source,
        &std::env::temp_dir().join("conduit-shell-test-snapshots"),
    )
    .unwrap()
}

fn approve(snapshot: PackageSnapshot) -> ApprovedPackage {
    let grants = snapshot.manifest().permissions().clone();
    ApprovedPackage::new(snapshot, grants).unwrap()
}

fn edit_manifest(source: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let path = source.join("conduit-plugin.json");
    let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn fixture(
    packages: Vec<ApprovedPackage>,
    cx: &mut TestAppContext,
) -> (Entity<DevelopmentProbe>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_shell::init(cx);
    });
    let output = Rc::new(RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let host = cx.new(|cx| DevelopmentProbe::new(packages, cx).unwrap());
        *slot.borrow_mut() = Some(host.clone());
        let shell = cx.new(|cx| ShellRoot::new(host.into(), window, cx));
        gpui_kit::component::Root::new(shell, window, cx)
    });
    cx.simulate_resize(size(px(1200.), px(780.)));
    let host = output.borrow_mut().take().unwrap();
    draw(cx);
    (host, cx)
}

fn close(host: Entity<DevelopmentProbe>, cx: &mut VisualTestContext) {
    cx.update(|window, _| window.remove_window());
    drop(host);
    // GPUI retires entities at the next App effect cycle, after the test's handle drops.
    cx.cx.update(|_| {});
    cx.run_until_parked();
}

fn text(host: &Entity<DevelopmentProbe>, right: bool, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let host = host.read(cx);
        let (_, view) = if right {
            &host.summary
        } else {
            &host.navigation
        }
        .as_ref()
        .unwrap();
        let view = view.read(cx);
        assert!(view.build_error().is_none(), "{:?}", view.build_error());
        view.snapshot().unwrap().debug_tree()
    })
}

fn dispatch(
    host: &Entity<DevelopmentProbe>,
    owner: &str,
    name: &str,
    cx: &mut VisualTestContext,
) -> PendingCommand {
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            let command = this
                .host
                .prepare_command(
                    &PluginId::new(owner).unwrap(),
                    &CommandId::new(format!("{owner}.{name}")).unwrap(),
                    serde_json::Value::Null,
                )
                .unwrap();
            this.host.dispatch(command, window, cx).unwrap()
        })
    })
}

fn receive(
    call: PendingCommand,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Option<CompletedCommand>>> {
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

fn accept(
    host: &Entity<DevelopmentProbe>,
    result: &Rc<RefCell<Option<CompletedCommand>>>,
    cx: &mut VisualTestContext,
) -> Result<Option<HostValue>> {
    draw(cx);
    let reply = result.borrow_mut().take().expect("command must settle");
    cx.update(|_, cx| host.update(cx, |this, cx| this.host.accept(reply, cx)))
}

fn increment(
    host: &Entity<DevelopmentProbe>,
    owner: &str,
    cx: &mut VisualTestContext,
) -> HostValue {
    let call = dispatch(host, owner, "increment", cx);
    let reply = receive(call, cx);
    accept(host, &reply, cx).unwrap().unwrap()
}

#[test]
fn registration_requires_explicit_grants_and_unique_owners() {
    let source = source(OWNER);
    assert!(ApprovedPackage::new(snapshot(source.path()), BTreeSet::new()).is_err());
    let (input, data) = context(0, true);
    assert!(
        DevelopmentHost::new(
            vec![
                approve(snapshot(source.path())),
                approve(snapshot(source.path()))
            ],
            input,
            data
        )
        .is_err()
    );
}

#[gpui_kit::test]
fn manifest_ui_activates_lazily_and_targets_summary_without_moving_focus(cx: &mut TestAppContext) {
    let source = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(source.path()))], cx);
    cx.update(|_, cx| {
        let state = &host.read(cx).host;
        assert_eq!(
            (
                state.views().len(),
                state.activities().len(),
                state.statuses().len()
            ),
            (2, 1, 1)
        );
        assert_eq!((state.runtime_count(), state.instance_count()), (0, 0));
    });
    click(cx, &format!("dev-activity-{OWNER}.activity"));
    assert!(text(&host, false, cx).contains("Sessions: 3"));
    cx.update(|_, cx| assert!(host.read(cx).summary.is_none()));
    click(cx, "dev-host-focus");
    click(cx, &format!("dev-status-{OWNER}.status"));
    assert!(text(&host, true, cx).contains("Command count: 1"));
    cx.update(|window, cx| {
        let state = host.read(cx);
        assert!(state.focus.is_focused(window));
        assert!(state.result.contains("Number(1.0)"), "{}", state.result);
        assert!(state.error.is_none(), "{:?}", state.error);
        assert_eq!(
            (state.host.runtime_count(), state.host.instance_count()),
            (1, 2)
        );
    });
    let version = cx.update(|_, cx| host.read(cx).host.context().version());
    cx.update(|_, cx| {
        host.update(cx, |this, cx| {
            let (input, mut data) = context(0, true);
            data.workspace.as_mut().unwrap().session_count = 9;
            this.host.set_context(input, data, cx).unwrap();
        })
    });
    draw(cx);
    assert!(text(&host, false, cx).contains("Sessions: 9"));
    assert!(text(&host, true, cx).contains("Command count: 1"));
    cx.update(|_, cx| assert_eq!(host.read(cx).host.context().version(), version));
    cx.update(|_, cx| {
        host.update(cx, |this, cx| {
            let right = this.summary.as_ref().unwrap().0.clone();
            this.host.set_visible(&[right]);
            let (input, mut data) = context(0, true);
            data.workspace.as_mut().unwrap().session_count = 11;
            this.host.set_context(input, data, cx).unwrap();
        })
    });
    click(cx, &format!("dev-activity-{OWNER}.activity"));
    assert!(
        text(&host, false, cx).contains("Sessions: 11"),
        "reopened cached views refresh metadata"
    );
    click(cx, "dev-workspace-b");
    cx.update(|_, cx| assert_eq!(host.read(cx).host.runtime_count(), 0));
    click(cx, &format!("dev-status-{OWNER}.status"));
    assert!(text(&host, true, cx).contains("Workspace B"));
    assert!(text(&host, true, cx).contains("Command count: 1"));
    let pending = receive(dispatch(&host, OWNER, "read-delayed", cx), cx);
    draw(cx);
    assert!(pending.borrow().is_none());
    let weak = cx.update(|_, cx| {
        host.read(cx)
            .host
            .runtime_weak(&PluginId::new(OWNER).unwrap())
            .unwrap()
    });
    close(host, cx);
    assert!(
        pending.borrow().is_some(),
        "window close settles pending command waiters"
    );
    assert!(
        weak.upgrade().is_none(),
        "window close must drop its runtime"
    );
}

#[gpui_kit::test]
fn stop_invalidates_prepared_commands_even_before_activation(cx: &mut TestAppContext) {
    let source = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(source.path()))], cx);
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            let id = PluginId::new(OWNER).unwrap();
            let command = CommandId::new(format!("{OWNER}.increment")).unwrap();
            let saved = this
                .host
                .prepare_command(&id, &command, serde_json::Value::Null)
                .unwrap();
            this.host.stop_all();
            assert!(this.host.dispatch(saved, window, cx).is_err());
            assert_eq!(this.host.runtime_count(), 0);
        })
    });
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(1.)
    );
    let saved = cx.update(|_, cx| {
        host.read(cx)
            .host
            .prepare_command(
                &PluginId::new(OWNER).unwrap(),
                &CommandId::new(format!("{OWNER}.increment")).unwrap(),
                serde_json::Value::Null,
            )
            .unwrap()
    });
    click(cx, "dev-stop");
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(1.)
    );
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            assert!(this.host.dispatch(saved, window, cx).is_err())
        })
    });
    close(host, cx);
}

#[gpui_kit::test]
fn tickets_reject_superseded_hidden_and_context_expired_results(cx: &mut TestAppContext) {
    let source = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(source.path()))], cx);
    let old = receive(dispatch(&host, OWNER, "read-delayed", cx), cx);
    draw(cx);
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(1.)
    );
    cx.executor().advance_clock(Duration::from_millis(1100));
    assert!(accept(&host, &old, cx).unwrap().is_none());
    let call = dispatch(&host, OWNER, "read-delayed", cx);
    let key = call.target().clone();
    let hidden = receive(call, cx);
    let entity = cx.update(|_, cx| host.read(cx).host.view(&key).unwrap().entity_id());
    cx.update(|_, cx| host.update(cx, |this, _| this.host.set_visible(&[])));
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            let (_, view) = this.host.open_view(&key.view, window, cx).unwrap();
            assert_eq!(entity, view.entity_id(), "hidden view should be reused");
        })
    });
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(1100));
    assert!(accept(&host, &hidden, cx).unwrap().is_none());
    for boundary in 0..4 {
        let queued = receive(dispatch(&host, OWNER, "increment", cx), cx);
        draw(cx);
        cx.update(|_, cx| {
            host.update(cx, |this, cx| {
                let (mut input, data) = context(boundary % 2, true);
                match boundary {
                    0 => {
                        input.document = Some("new-document".into());
                    }
                    1 => {}
                    2 => input.writable = false,
                    _ => input.closing = true,
                }
                this.host.set_context(input, data, cx).unwrap();
            })
        });
        assert!(accept(&host, &queued, cx).unwrap().is_none());
        cx.update(|_, cx| {
            host.update(cx, |this, cx| {
                let (input, data) = context(0, true);
                this.host.set_context(input, data, cx).unwrap();
            })
        });
    }
    let delayed = receive(dispatch(&host, OWNER, "read-delayed", cx), cx);
    draw(cx);
    click(cx, "dev-stop");
    assert!(
        accept(&host, &delayed, cx).unwrap().is_none(),
        "stop settles pending waiter"
    );
    close(host, cx);
}

#[gpui_kit::test]
fn invalid_context_is_atomic_and_session_scopes_follow_the_selected_workspace(
    cx: &mut TestAppContext,
) {
    let source = source(OWNER);
    edit_manifest(source.path(), |value| {
        for view in value["contributes"]["views"].as_array_mut().unwrap() {
            view["scope"] = "session".into();
        }
        for command in value["contributes"]["commands"].as_array_mut().unwrap() {
            command["scope"] = "session".into();
        }
        value["contributes"]["statusItems"][0]["scope"] = "session".into();
    });
    let (host, cx) = fixture(vec![approve(snapshot(source.path()))], cx);
    let view = ViewId::new(format!("{OWNER}.summary")).unwrap();
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            assert!(this.host.open_view(&view, window, cx).is_err());
            let version = this.host.context().version();
            let (mut input, data) = context(0, true);
            input.session = Some(SessionKey {
                workspace: input.workspace.clone().unwrap(),
                id: 7,
            });
            assert!(
                this.host
                    .set_context(input.clone(), HostData::default(), cx)
                    .is_err()
            );
            assert_eq!(this.host.context().version(), version);
            this.host.set_context(input, data, cx).unwrap();
            assert_eq!(
                this.host.context().version(),
                version + 1,
                "rejected context must not advance tracker"
            );
            this.host.open_view(&view, window, cx).unwrap();
            let (mut input, data) = context(1, true);
            input.session = this.host.context().input().session.clone();
            this.host.set_context(input, data, cx).unwrap();
            assert!(this.host.context().input().session.is_none());
            assert!(this.host.open_view(&view, window, cx).is_err());
            this.host
                .set_context(
                    ContextInput {
                        writable: true,
                        ..Default::default()
                    },
                    HostData::default(),
                    cx,
                )
                .unwrap();
            assert!(this.host.open_view(&view, window, cx).is_err());
            assert_eq!(this.host.runtime_count(), 0);
        })
    });
    close(host, cx);
}

const AUTHORITY_SOURCE: &str = r#"
import {View,div} from 'gpui-kit';
import {getContext,getWorkspaceMetadata,getSessionSummary} from 'conduit';
function permissions(){
  let workspace='denied',session='denied';
  try{workspace=getWorkspaceMetadata().name;}catch(_){}
  try{session=String(getSessionSummary().count);}catch(_){}
  return workspace+'/'+session;
}
const imported=permissions();
export default class extends View {
  init(){this.initialized=permissions();}
  on_command(name,_payload,cx){
    const reply=()=>({owner:getContext().pluginId,imported,initialized:this.initialized,current:permissions()});
    return name==='read-after-delay'?cx.sleep(10).then(reply):reply();
  }
  render(){return div().child(permissions());}
}
"#;

#[gpui_kit::test]
fn owners_keep_permissions_during_import_init_commands_and_await(cx: &mut TestAppContext) {
    let a = source(OWNER);
    let b = source("example.no-access");
    for source in [&a, &b] {
        fs::write(source.path().join("summary.js"), AUTHORITY_SOURCE).unwrap();
    }
    edit_manifest(b.path(), |value| {
        value["permissions"] = serde_json::json!([]);
        value["contributes"]["commands"][1]["requiredPermissions"] = serde_json::json!([]);
    });
    let (host, cx) = fixture(
        vec![approve(snapshot(a.path())), approve(snapshot(b.path()))],
        cx,
    );
    for (owner, expected) in [
        (OWNER, "Workspace A/3"),
        ("example.no-access", "denied/denied"),
    ] {
        let result = increment(&host, owner, cx);
        assert_eq!(result.get("owner").unwrap().as_str(), Some(owner));
        for phase in ["imported", "initialized", "current"] {
            assert_eq!(result.get(phase).unwrap().as_str(), Some(expected));
        }
        let delayed = receive(dispatch(&host, owner, "read-delayed", cx), cx);
        draw(cx);
        cx.executor().advance_clock(Duration::from_millis(20));
        let result = accept(&host, &delayed, cx).unwrap().unwrap();
        assert_eq!(result.get("current").unwrap().as_str(), Some(expected));
    }
    cx.update(|_, cx| {
        host.update(cx, |this, _| {
            assert_eq!(this.host.runtime_count(), 2);
            this.host.stop_owner(&PluginId::new(OWNER).unwrap());
            assert_eq!(this.host.runtime_count(), 1);
        })
    });
    assert_eq!(
        increment(&host, "example.no-access", cx)
            .get("owner")
            .unwrap()
            .as_str(),
        Some("example.no-access")
    );
    close(host, cx);
}

#[gpui_kit::test]
fn load_errors_stop_only_the_owner(cx: &mut TestAppContext) {
    let bad = source("example.broken");
    fs::write(
        bad.path().join("navigation.js"),
        "import missing from 'unapproved-module'; export default missing;",
    )
    .unwrap();
    let good = source(OWNER);
    let (host, cx) = fixture(
        vec![
            approve(snapshot(bad.path())),
            approve(snapshot(good.path())),
        ],
        cx,
    );
    increment(&host, "example.broken", cx);
    let weak = cx.update(|_, cx| {
        host.read(cx)
            .host
            .runtime_weak(&PluginId::new("example.broken").unwrap())
            .unwrap()
    });
    increment(&host, OWNER, cx);
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            assert!(
                this.host
                    .open_view(&ViewId::new("example.broken.index").unwrap(), window, cx)
                    .is_err()
            );
            assert_eq!(this.host.runtime_count(), 1);
        })
    });
    draw(cx);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(2.)
    );
    close(host, cx);
}

#[gpui_kit::test]
fn rejected_promises_reach_the_development_ui_and_do_not_poison_the_vm(cx: &mut TestAppContext) {
    let source = source(OWNER);
    fs::write(
        source.path().join("summary.js"),
        r#"
import {View,div} from 'gpui-kit';
export default class extends View {
  on_command(){return Promise.reject(new Error('development rejection'));}
  render(){return div().child('Still usable');}
}
"#,
    )
    .unwrap();
    let (host, cx) = fixture(vec![approve(snapshot(source.path()))], cx);
    click(cx, &format!("dev-status-{OWNER}.status"));
    cx.update(|_, cx| {
        let state = host.read(cx);
        assert!(
            state
                .error
                .as_ref()
                .unwrap()
                .contains("development rejection")
        );
        assert_eq!(state.host.runtime_count(), 1);
    });
    assert!(text(&host, true, cx).contains("Still usable"));
    close(host, cx);
}

#[gpui_kit::test]
fn snapshots_pin_code_detect_tampering_and_reload_without_host_rebuild(cx: &mut TestAppContext) {
    let source = source(OWNER);
    let original = snapshot(source.path());
    let root = original.root().to_path_buf();
    let summary = fs::read_to_string(source.path().join("summary.js")).unwrap();
    fs::write(
        source.path().join("summary.js"),
        summary.replace("this.count = 0;", "this.count = 40;"),
    )
    .unwrap();
    let (host, cx) = fixture(vec![approve(original)], cx);
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(1.)
    );
    cx.update(|_, cx| {
        host.update(cx, |this, _| {
            this.host
                .unregister(&PluginId::new(OWNER).unwrap())
                .unwrap()
        })
    });
    assert!(!root.exists(), "unregister releases owned snapshot");
    let fresh = approve(snapshot(source.path()));
    cx.update(|_, cx| {
        host.update(cx, |this, _| {
            let (input, data) = context(0, true);
            this.host = DevelopmentHost::new(vec![fresh], input, data).unwrap();
        })
    });
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(41.)
    );
    let changed = snapshot(source.path());
    fs::write(
        changed.root().join("summary.js"),
        "throw new Error('tampered');",
    )
    .unwrap();
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            let (input, data) = context(0, true);
            this.host = DevelopmentHost::new(vec![approve(changed)], input, data).unwrap();
            let error = this
                .host
                .open_view(
                    &ViewId::new(format!("{OWNER}.summary")).unwrap(),
                    window,
                    cx,
                )
                .unwrap_err();
            assert!(error.to_string().contains("snapshot changed"));
            assert_eq!(this.host.runtime_count(), 0);
        })
    });
    close(host, cx);
}

#[gpui_kit::test]
#[ignore = "intentional runaway script; use the isolated development-fault phase"]
fn adapter_reaps_async_resource_faults_without_stopping_other_owners(cx: &mut TestAppContext) {
    let bad = source("example.runaway");
    fs::write(
        bad.path().join("summary.js"),
        r#"
import {View,div} from 'gpui-kit';
export default class extends View {
  on_command(_name,_payload,cx){
    window.open_sheet(()=>div().child('Fault sheet'));
    window.open_dialog(()=>div().child('Fault dialog'));
    window.push_toast({title:'Fault toast',timeout:null});
    return cx.sleep(10).then(()=>{while(true){}});
  }
  render(){return div().child('Runaway fixture');}
}
"#,
    )
    .unwrap();
    let good = source(OWNER);
    let (host, cx) = fixture(
        vec![
            approve(snapshot(bad.path())),
            approve(snapshot(good.path())),
        ],
        cx,
    );
    increment(&host, OWNER, cx);
    let pending = receive(dispatch(&host, "example.runaway", "increment", cx), cx);
    let weak = cx.update(|_, cx| {
        host.read(cx)
            .host
            .runtime_weak(&PluginId::new("example.runaway").unwrap())
            .unwrap()
    });
    draw(cx);
    let started = std::time::Instant::now();
    cx.update(|window, cx| {
        ShellRoot::update(window, cx, |root, _, _| {
            assert_eq!(root.dialog_count(), 1);
            assert!(root.sheet().is_some());
        })
    });
    cx.executor().advance_clock(Duration::from_millis(20));
    let error = accept(&host, &pending, cx).unwrap_err();
    assert!(
        error.to_string().contains("execution deadline"),
        "{error:#}"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    draw(cx);
    assert!(weak.upgrade().is_none());
    cx.update(|window, cx| {
        ShellRoot::update(window, cx, |root, _, _| {
            assert_eq!(root.dialog_count(), 0);
            assert!(root.sheet().is_none());
            assert_eq!(root.toast_count(), 0);
        })
    });
    cx.update(|_, cx| assert_eq!(host.read(cx).host.runtime_count(), 1));
    assert_eq!(
        increment(&host, OWNER, cx)
            .get("count")
            .unwrap()
            .as_number(),
        Some(2.)
    );
    close(host, cx);
}
