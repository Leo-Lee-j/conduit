use super::{OWNER, accept, approve, close, fixture, receive, snapshot};
use crate::development::DevelopmentProbe;
use crate::ui_tests::{click, draw};
use conduit_shell_prototype::{
    adapter::PendingCommand,
    plugins::ids::{CommandId, PluginId},
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, TestAppContext,
    VisualTestContext, Window, div,
};
use gpui_shell::{HostValue, ShellRoot, ShellRuntime, ToastRequest};
use std::{rc::Weak, time::Duration};

const OTHER: &str = "example.other";

fn source(owner: &str) -> tempfile::TempDir {
    let source = super::source(owner);
    super::edit_manifest(source.path(), |manifest| {
        let commands = manifest["contributes"]["commands"].as_array_mut().unwrap();
        let command = commands
            .iter_mut()
            .find(|command| command["binding"]["command"] == "overlays")
            .unwrap();
        command["arguments"] = serde_json::json!({"kind":"object","maxBytes":128,"maxDepth":2});
    });
    source
}

fn bounds(cx: &mut VisualTestContext, id: &str) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
    cx.update(|window, _| {
        gpui_kit::base::test_support::find(window, &[], &gpui_kit::ElementId::from(id.to_owned()))
            .map(|element| element.bounds())
    })
}

fn dispatch(
    host: &Entity<DevelopmentProbe>,
    owner: &str,
    kind: &str,
    cx: &mut VisualTestContext,
) -> PendingCommand {
    cx.update(|window, cx| {
        host.update(cx, |this, cx| {
            let command = this
                .host
                .prepare_command(
                    &PluginId::new(owner).unwrap(),
                    &CommandId::new(format!("{owner}.overlays")).unwrap(),
                    serde_json::json!({"kind":kind}),
                )
                .unwrap();
            this.host.dispatch(command, window, cx).unwrap()
        })
    })
}

fn open(
    host: &Entity<DevelopmentProbe>,
    owner: &str,
    kind: &str,
    cx: &mut VisualTestContext,
) -> HostValue {
    let pending = dispatch(host, owner, kind, cx);
    let reply = receive(pending, cx);
    accept(host, &reply, cx).unwrap().unwrap()
}

fn counts(cx: &mut VisualTestContext) -> (usize, bool, usize, bool) {
    cx.update(|window, cx| {
        ShellRoot::update(window, cx, |root, _, cx| {
            for view in [root.topmost_dialog(), root.sheet()].into_iter().flatten() {
                if let Ok(view) = view.clone().downcast::<gpui_shell::ScriptView>() {
                    assert!(
                        view.read(cx).build_error().is_none(),
                        "{:?}",
                        view.read(cx).build_error()
                    );
                }
            }
            (
                root.dialog_count(),
                root.sheet().is_some(),
                root.toast_count(),
                root.fps_monitor_visible(),
            )
        })
        .unwrap()
    })
}

fn weak(
    host: &Entity<DevelopmentProbe>,
    owner: &str,
    cx: &mut VisualTestContext,
) -> Weak<ShellRuntime> {
    cx.update(|_, cx| {
        host.read(cx)
            .host
            .runtime_weak(&PluginId::new(owner).unwrap())
            .unwrap()
    })
}

struct Native;
impl Render for Native {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("Host overlay")
    }
}

#[gpui_kit::test]
fn reactivation_before_cleanup_keeps_the_new_dialog_and_sheet(cx: &mut TestAppContext) {
    let a = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(a.path()))], cx);
    open(&host, OWNER, "all", cx);
    let old = weak(&host, OWNER, cx);
    let pending = cx.update(|window, cx| {
        host.update(cx, |host, cx| {
            let owner = PluginId::new(OWNER).unwrap();
            host.host.stop_owner(&owner);
            let command = host
                .host
                .prepare_command(
                    &owner,
                    &CommandId::new(format!("{OWNER}.overlays")).unwrap(),
                    serde_json::json!({"kind":"all"}),
                )
                .unwrap();
            // No event-loop turn between revocation and the new incarnation.
            host.host.dispatch(command, window, cx).unwrap()
        })
    });
    let reply = receive(pending, cx);
    assert!(accept(&host, &reply, cx).unwrap().is_some());
    draw(cx);
    assert_eq!(counts(cx), (1, true, 1, false));
    assert!(old.upgrade().is_none());
    assert!(weak(&host, OWNER, cx).upgrade().is_some());
    close(host, cx);
}

#[gpui_kit::test]
fn stop_preserves_other_owners_and_repairs_interleaved_focus(cx: &mut TestAppContext) {
    let a = source(OWNER);
    let b = source(OTHER);
    let (host, cx) = fixture(
        vec![approve(snapshot(a.path())), approve(snapshot(b.path()))],
        cx,
    );
    let focus = cx.update(|window, cx| {
        let focus = host.read(cx).focus.clone();
        focus.focus(window, cx);
        let native = cx.new(|_| Native);
        ShellRoot::update(window, cx, |root, window, cx| {
            root.open_dialog(native.into(), window, cx);
            root.push_toast(
                ToastRequest::new("Host toast")
                    .with_id("overlay-status")
                    .with_timeout(None),
                window,
                cx,
            );
        });
        focus
    });
    draw(cx);
    let host_focus = cx.update(|window, cx| window.focused(cx).unwrap());
    open(&host, OWNER, "all", cx);
    open(&host, OTHER, "dialog", cx);
    open(&host, OTHER, "extras", cx);
    let b_focus = cx.update(|window, cx| window.focused(cx).unwrap());
    open(&host, OWNER, "dialog", cx);
    // A removed layer may own focus through one of its child controls.
    cx.simulate_keystrokes("tab");
    draw(cx);
    let old = weak(&host, OWNER, cx);
    assert_eq!(counts(cx), (4, true, 3, true));
    cx.update(|_, cx| {
        host.update(cx, |host, _| {
            host.host.stop_owner(&PluginId::new(OWNER).unwrap())
        })
    });
    // No timer advance: stop must wake cleanup on the foreground executor.
    draw(cx);
    assert_eq!(counts(cx), (2, false, 2, true));
    assert!(old.upgrade().is_none());
    cx.update(|window, _| assert!(b_focus.is_focused(window)));
    // A's fresh incarnation must not be removed by an old stop notification.
    open(&host, OWNER, "dialog", cx);
    assert_eq!(counts(cx).0, 3);
    open(&host, OWNER, "clear", cx);
    assert_eq!(counts(cx), (2, false, 2, true));
    open(&host, OTHER, "clear", cx);
    cx.update(|window, _| assert!(host_focus.is_focused(window)));
    cx.update(|window, cx| {
        ShellRoot::update(window, cx, |root, window, cx| {
            root.close_all_dialogs(window, cx);
        });
    });
    cx.update(|window, _| assert!(focus.is_focused(window)));
    close(host, cx);
}

#[gpui_kit::test]
fn scripts_cannot_replace_or_close_host_or_other_owners_overlays(cx: &mut TestAppContext) {
    let a = source(OWNER);
    let b = source(OTHER);
    let (host, cx) = fixture(
        vec![approve(snapshot(a.path())), approve(snapshot(b.path()))],
        cx,
    );
    open(&host, OWNER, "all", cx);
    open(&host, OWNER, "extras", cx);
    let original = cx.update(|window, cx| {
        ShellRoot::update(window, cx, |root, _, _| root.sheet().unwrap().entity_id()).unwrap()
    });
    open(&host, OTHER, "sheet", cx);
    assert_eq!(
        open(&host, OTHER, "clear", cx)
            .get("closed")
            .unwrap()
            .as_number(),
        Some(0.)
    );
    assert_eq!(counts(cx), (1, true, 1, true));
    cx.update(|window, cx| {
        ShellRoot::update(window, cx, |root, _, _| {
            assert_eq!(root.sheet().unwrap().entity_id(), original)
        })
    });
    let old = weak(&host, OWNER, cx);
    cx.update(|_, cx| {
        host.update(cx, |host, _| {
            host.host
                .unregister(&PluginId::new(OWNER).unwrap())
                .unwrap()
        })
    });
    draw(cx);
    assert_eq!(counts(cx), (0, false, 0, false));
    assert!(old.upgrade().is_none());
    open(&host, OTHER, "all", cx);
    assert_eq!(counts(cx), (1, true, 1, false));
    close(host, cx);
}

#[gpui_kit::test]
fn context_stop_cancels_late_open_and_preserves_host_sheet(cx: &mut TestAppContext) {
    let a = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(a.path()))], cx);
    cx.update(|window, cx| {
        let native = cx.new(|_| Native);
        ShellRoot::update(window, cx, |root, window, cx| {
            root.open_sheet(gpui_kit::base::Placement::Left, native.into(), window, cx)
        });
    });
    open(&host, OWNER, "all", cx);
    let pending = dispatch(&host, OWNER, "delayed", cx);
    let key = pending.target().clone();
    let delayed = receive(pending, cx);
    let old = weak(&host, OWNER, cx);
    let target = cx.update(|_, cx| {
        let view = host.read(cx).host.view(&key).unwrap();
        old.upgrade().unwrap().command_target(&view, cx).unwrap()
    });
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            let (input, data) = crate::development::context(1, true);
            host.host.set_context(input, data, cx).unwrap();
        })
    });
    draw(cx);
    assert_eq!(counts(cx), (0, true, 0, false));
    cx.update(|window, cx| {
        assert!(
            target
                .dispatch("overlays", HostValue::Null, window, cx)
                .is_err()
        )
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    assert!(accept(&host, &delayed, cx).unwrap().is_none());
    draw(cx);
    assert_eq!(counts(cx), (0, true, 0, false));
    assert!(old.upgrade().is_none());
    close(host, cx);
}

#[gpui_kit::test]
fn stopping_a_hovered_plugin_clears_tooltip_without_waiting_for_mouse_move(
    cx: &mut TestAppContext,
) {
    let a = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(a.path()))], cx);
    click(cx, &format!("dev-status-{OWNER}.status"));
    let snapshot = super::text(&host, true, cx);
    assert!(snapshot.contains("plugin-open-overlays"), "{snapshot}");
    let trigger = bounds(cx, "plugin-open-overlays").unwrap();
    cx.simulate_mouse_move(trigger.center(), None, gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            ShellRoot::tooltip_overlay(window, cx)
                .unwrap()
                .read(cx)
                .is_visible()
        )
    });
    let old = weak(&host, OWNER, cx);
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.host.stop_all();
            host.summary = None;
            host.navigation = None;
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            !ShellRoot::tooltip_overlay(window, cx)
                .unwrap()
                .read(cx)
                .is_visible()
        )
    });
    cx.executor().advance_clock(Duration::from_secs(2));
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            !ShellRoot::tooltip_overlay(window, cx)
                .unwrap()
                .read(cx)
                .is_visible()
        )
    });
    assert!(old.upgrade().is_none());
    close(host, cx);
}

#[gpui_kit::test]
fn stopping_plugin_preserves_a_host_replacement_tooltip(cx: &mut TestAppContext) {
    let a = source(OWNER);
    let (host, cx) = fixture(vec![approve(snapshot(a.path()))], cx);
    click(cx, &format!("dev-status-{OWNER}.status"));
    let trigger = bounds(cx, "plugin-open-overlays").unwrap();
    cx.simulate_mouse_move(trigger.center(), None, gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    cx.update(|window, cx| {
        let overlay = ShellRoot::tooltip_overlay(window, cx).unwrap();
        assert!(overlay.read(cx).is_visible());
        let label = cx.new(|_| Native);
        let request =
            gpui_kit::base::TooltipRequest::new(trigger, move |_, _| label.clone().into())
                .with_show_delay(Duration::ZERO);
        overlay.update(cx, |overlay, cx| overlay.request_show(request, window, cx));
        host.update(cx, |host, cx| {
            host.stop();
            cx.notify();
        });
    });
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            ShellRoot::tooltip_overlay(window, cx)
                .unwrap()
                .read(cx)
                .is_visible()
        )
    });
    close(host, cx);
}
