use super::{ApprovedPackage, ConduitApp, PluginId, builtins};
use crate::app::plugin_ui::tests::{ElementBounds as _, click, draw, store};
use conduit_shell_prototype::package::PackageSnapshot;
use gpui_kit::component::WindowExt;
use gpui_kit::{AppContext, Entity, Focusable, TestAppContext, VisualTestContext, px, size};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[path = "tests/breakdown.rs"]
mod breakdown;
#[path = "tests/cache.rs"]
mod cache;
#[path = "tests/chat_profile.rs"]
mod chat_profile;
#[path = "tests/diagnostics.rs"]
mod diagnostics;
#[path = "tests/host_profile.rs"]
mod host_profile;
#[path = "tests/interaction.rs"]
mod interaction;
#[cfg(feature = "allocation-probe")]
#[path = "tests/native_memory.rs"]
mod native_memory;
#[path = "tests/performance.rs"]
mod performance;
#[path = "tests/stress.rs"]
mod stress;

fn package() -> ApprovedPackage {
    let source =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/development-overview");
    let staging = std::env::var_os("CONDUIT_DATA_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("plugin-test-snapshots")
        .join(std::process::id().to_string());
    let snapshot = PackageSnapshot::prepare(&source, &staging).unwrap();
    let grants = snapshot.manifest().permissions().clone();
    ApprovedPackage::new(snapshot, grants).unwrap()
}

pub(in crate::app) fn fixture(
    cx: &mut TestAppContext,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    fixture_with_packages(cx, vec![package()])
}

fn fixture_with_packages(
    cx: &mut TestAppContext,
    packages: Vec<ApprovedPackage>,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    fixture_with_store(cx, packages, store())
}

fn fixture_with_store(
    cx: &mut TestAppContext,
    packages: Vec<ApprovedPackage>,
    store: crate::store::Store,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    fixture_with_frame_counter(cx, packages, store, None)
}

fn fixture_with_frame_counter(
    cx: &mut TestAppContext,
    packages: Vec<ApprovedPackage>,
    store: crate::store::Store,
    frames: Option<Rc<std::cell::Cell<u64>>>,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_shell::init(cx);
    });
    let output = Rc::new(RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            if !packages.is_empty() {
                app.install_development_packages(packages, cx).unwrap();
            }
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        let shell =
            cx.new(|cx| gpui_shell::ShellRoot::new(app.into(), window, cx).with_uncached_content());
        let content: gpui_kit::AnyView = match frames {
            Some(frames) => cx
                .new(|_| diagnostics::FrameProbe {
                    content: shell.into(),
                    frames,
                })
                .into(),
            None => shell.into(),
        };
        gpui_kit::component::Root::new(content, window, cx)
    });
    cx.simulate_resize(size(px(1600.), px(800.)));
    draw(cx);
    draw(cx);
    let app = output.borrow_mut().take().unwrap();
    (app, cx)
}

fn text(app: &Entity<ConduitApp>, right: bool, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        let plugins = &app.read(cx).plugins;
        let key = if right {
            plugins.right_key()
        } else {
            plugins.state.navigation_key()
        }
        .unwrap();
        let view = plugins.shell.as_ref().unwrap().host.view(key).unwrap();
        assert!(
            view.read(cx).build_error().is_none(),
            "{:?}",
            view.read(cx).build_error()
        );
        view.read(cx).snapshot().unwrap().debug_tree()
    })
}

pub(in crate::app) fn command(app: &Entity<ConduitApp>, id: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let command = app.plugins.host_command(id).unwrap();
            assert!(app.dispatch_plugin_command(&command, window, cx));
        })
    });
    draw(cx);
}

pub(in crate::app) fn runtime(
    app: &Entity<ConduitApp>,
    cx: &mut VisualTestContext,
) -> std::rc::Weak<gpui_shell::ShellRuntime> {
    cx.update(|_, cx| {
        app.read(cx)
            .plugins
            .shell
            .as_ref()
            .unwrap()
            .host
            .runtime_weak(&PluginId::new(builtins::EXAMPLE).unwrap())
            .unwrap()
    })
}

pub(in crate::app) fn assert_no_overlays(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        gpui_shell::ShellRoot::update(window, cx, |root, _, _| {
            assert_eq!(root.dialog_count(), 0);
            assert!(root.sheet().is_none());
            assert_eq!(root.toast_count(), 0);
        })
        .unwrap();
    });
}

#[gpui_kit::test]
fn desktop_disable_cleans_script_overlays_and_preserves_settings(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.overlays", cx);
    let weak = runtime(&app, cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    cx.update(|window, cx| {
        let focus = window.focused(cx).unwrap();
        app.update(cx, |app, cx| app.disable_shell(window, cx));
        assert!(app.read(cx).settings_ui.open);
        assert!(focus.is_focused(window));
    });
    assert_no_overlays(cx);
    draw(cx);
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn desktop_registry_revocation_stops_without_waiting_for_a_render(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.overlays", cx);
    command(&app, "example.workspace-overview.read-delayed", cx);
    let weak = runtime(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let registration = app
                .plugins
                .registry
                .registration(&PluginId::new(builtins::EXAMPLE).unwrap())
                .unwrap()
                .clone();
            app.plugins.registry.unregister(&registration).unwrap();
            app.sync_plugin_context();
            let shell = app.plugins.shell.as_ref().unwrap();
            assert_eq!(shell.host.runtime_count(), 0);
            assert!(shell.tasks.is_empty());
            assert!(shell.manifests.is_empty());
        })
    });
    cx.run_until_parked();
    assert_no_overlays(cx);
    draw(cx);
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn desktop_entries_are_static_and_three_surfaces_use_real_metadata(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        let shell = app.plugins.shell.as_ref().unwrap();
        assert_eq!(shell.host.runtime_count(), 0);
        assert!(
            !shell.manifests[&PluginId::new(builtins::EXAMPLE).unwrap()]
                .contributions()
                .owner
                .is_host()
        );
        assert!(!app.plugins.commands.contains_key(
            &conduit::plugins::ids::CommandId::new(builtins::EXAMPLE_COMMAND).unwrap()
        ));
    });
    click(cx, builtins::EXAMPLE_ACTIVITY);
    assert!(text(&app, false, cx).contains("conduit-plugin-navigation-0"));
    assert!(text(&app, false, cx).contains("1"));
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    click(cx, builtins::EXAMPLE_STATUS);
    assert!(text(&app, true, cx).contains("Command count: 1"));
    cx.update(|window, cx| {
        let app = app.read(cx);
        let shell = app.plugins.shell.as_ref().unwrap();
        assert_eq!(shell.host.runtime_count(), 1);
        assert_eq!(shell.host.instance_count(), 2);
        assert!(
            shell
                .result
                .as_ref()
                .is_some_and(|text| text.contains("count"))
        );
        assert!(shell.tasks.is_empty());
        assert!(app.input.read(cx).focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn desktop_context_change_stops_vm_and_requires_explicit_reopen(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    click(cx, builtins::EXAMPLE_ACTIVITY);
    command(&app, "example.workspace-overview.increment", cx);
    command(&app, "example.workspace-overview.overlays", cx);
    let (weak, saved) = cx.update(|_, cx| {
        let app = app.read(cx);
        (
            app.plugins
                .shell
                .as_ref()
                .unwrap()
                .host
                .runtime_weak(&PluginId::new(builtins::EXAMPLE).unwrap())
                .unwrap(),
            app.plugins
                .host_command("example.workspace-overview.increment")
                .unwrap(),
        )
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(1);
            app.selected_session = Some(2);
            app.sync_plugin_context();
            assert_eq!(app.plugins.shell.as_ref().unwrap().host.runtime_count(), 0);
            cx.notify();
        })
    });
    draw(cx);
    assert_no_overlays(cx);
    assert!(weak.upgrade().is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.dispatch_plugin_command(&saved, window, cx));
            assert_eq!(app.plugins.shell.as_ref().unwrap().host.runtime_count(), 0);
        })
    });
    click(cx, builtins::EXAMPLE_ACTIVITY);
    assert!(text(&app, false, cx).contains("conduit-plugin-navigation-1"));
}

#[gpui_kit::test]
fn desktop_hidden_views_refresh_without_reactivation_and_readonly_releases(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx);
    click(cx, builtins::EXAMPLE_ACTIVITY);
    command(&app, "example.workspace-overview.increment", cx);
    let original = cx.update(|_, cx| {
        let plugins = &app.read(cx).plugins;
        plugins
            .shell
            .as_ref()
            .unwrap()
            .host
            .view(plugins.state.navigation_key().unwrap())
            .unwrap()
            .entity_id()
    });
    click(cx, "nav-workspaces");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let mut session = crate::app::tests::session();
            session.id = 77;
            app.store.workspaces[0].sessions.push(session);
            app.refresh_shell_context(cx);
            cx.notify();
        })
    });
    click(cx, builtins::EXAMPLE_ACTIVITY);
    assert!(text(&app, false, cx).contains("2"));
    assert!(text(&app, true, cx).contains("Command count: 1"));
    command(&app, "example.workspace-overview.overlays", cx);
    let weak = runtime(&app, cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let shell = app.plugins.shell.as_ref().unwrap();
            assert_eq!(
                shell
                    .host
                    .view(app.plugins.state.navigation_key().unwrap())
                    .unwrap()
                    .entity_id(),
                original
            );
            app.writable = false;
            app.sync_plugin_context();
            assert_eq!(app.plugins.shell.as_ref().unwrap().host.runtime_count(), 0);
            cx.notify();
        })
    });
    draw(cx);
    assert_no_overlays(cx);
    assert!(weak.upgrade().is_none());
}

#[gpui_kit::test]
fn desktop_stop_revokes_saved_actions_and_pending_results(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    click(cx, builtins::EXAMPLE_ACTIVITY);
    command(&app, "example.workspace-overview.read-delayed", cx);
    let saved = cx.update(|_, cx| {
        app.read(cx)
            .plugins
            .host_command("example.workspace-overview.increment")
            .unwrap()
    });
    click(cx, "shell-stop-example.workspace-overview.summary");
    cx.executor().advance_clock(Duration::from_secs(2));
    draw(cx);
    assert!(cx.element_bounds(builtins::EXAMPLE_ACTIVITY).is_none());
    assert!(cx.element_bounds(builtins::EXAMPLE_STATUS).is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.plugins.shell.is_none());
            assert!(app.plugins.is_selected(builtins::WORKSPACES));
            assert!(!app.plugins.right_open());
            assert!(!app.dispatch_plugin_command(&saved, window, cx));
        })
    });
}

#[gpui_kit::test]
fn desktop_narrow_dock_keeps_toolbar_and_host_dialog_focus(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.increment", cx);
    for width in [1600., 1100., 850.] {
        cx.simulate_resize(size(px(width), px(700.)));
        draw(cx);
        draw(cx);
        let toolbar = cx.element_bounds("conversation-toolbar").unwrap();
        let dock = cx.element_bounds("detail-panel").unwrap();
        assert!(dock.right() <= px(width));
        assert_eq!(toolbar.size.height, px(34.));
        if width < 1100. {
            assert!(dock.bottom() <= toolbar.top());
        } else {
            assert!(toolbar.right() <= dock.left());
        }
    }
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    draw(cx);
    cx.update(|window, cx| {
        assert!(app.read(cx).settings_ui.open);
        assert_eq!(
            gpui_shell::ShellRoot::update(window, cx, |root, _, _| root.dialog_count()),
            Some(0)
        );
        app.update(cx, |app, cx| app.close_settings(window, cx));
    });
    draw(cx);
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("中文输入验证");
    draw(cx);
    cx.update(|_, cx| assert_eq!(app.read(cx).input.read(cx).value().as_str(), "中文输入验证"));
}

#[gpui_kit::test]
fn desktop_window_close_stops_vm_with_pending_command(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    command(&app, "example.workspace-overview.overlays", cx);
    command(&app, "example.workspace-overview.read-delayed", cx);
    let weak = cx.update(|_, cx| {
        app.read(cx)
            .plugins
            .shell
            .as_ref()
            .unwrap()
            .host
            .runtime_weak(&PluginId::new(builtins::EXAMPLE).unwrap())
            .unwrap()
    });
    // Conduit postpones native close while its normal shutdown poll drains work.
    assert!(!cx.simulate_close());
    assert_no_overlays(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.closing);
        let shell = app.plugins.shell.as_ref().unwrap();
        assert_eq!(shell.host.runtime_count(), 0);
        assert!(shell.tasks.is_empty());
    });
    draw(cx);
    assert!(weak.upgrade().is_none());
    cx.update(|window, _| window.remove_window());
    drop(app);
    cx.cx.update(|_| {});
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}
