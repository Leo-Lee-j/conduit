use super::visible_count;
use crate::app::{
    ConduitApp, Store,
    plugin_ui::{
        PluginUi,
        tests::{click, draw, fixture, store},
    },
};
use conduit::plugins::{
    builtins,
    commands::{ArgumentPolicy, Capability, CommandDescriptor, CommandInvocation},
    context::{Scope, When},
    contributions::{
        ActivityAction, ActivityItem, Alignment, ContributionSet, StatusItem, StatusValue,
    },
    ids::{CommandId, ItemId, Owner, PluginId},
};
use gpui_kit::{Context, Modifiers, TestAppContext, Window, base::test_support, point, px, size};
use std::collections::BTreeSet;

const NATIVE: &str = "example.workspace-overview";
const EXTERNAL: &str = "test.status-external";

fn declarations(owner: Owner, overflow: usize) -> ContributionSet {
    let prefix = owner.id().to_string();
    let mut set = ContributionSet::empty(owner);
    let names = ["read".to_owned(), "write".to_owned(), "unbound".to_owned()]
        .into_iter()
        .chain((0..overflow).map(|i| format!("overflow-{i:02}")));
    for (i, name) in names.enumerate() {
        let command = CommandId::new(format!("{prefix}.{name}")).unwrap();
        set.commands.push(CommandDescriptor {
            id: command.clone(),
            title: name.clone(),
            scope: Scope::Workspace,
            arguments: ArgumentPolicy::None,
            required_capabilities: BTreeSet::from([Capability::WorkspaceSummary]),
            requires_writable: name == "write",
        });
        set.status_items.push(StatusItem {
            id: ItemId::new(format!("{prefix}.{name}-status")).unwrap(),
            scope: Scope::Workspace,
            alignment: if i % 2 == 0 {
                Alignment::Left
            } else {
                Alignment::Right
            },
            priority: if i < 3 { 60 - i as i32 } else { -(i as i32) },
            initial: StatusValue {
                text: if i >= 3 {
                    format!("{name} · {}", "状态".repeat(25))
                } else {
                    format!("{name} {prefix}")
                },
                tooltip: Some(format!("Full tooltip {prefix}.{name}")),
            },
            icon: None,
            command: Some(command.clone()),
            when: When::parse(Some("workspace.open")).unwrap(),
        });
        if name == "read" {
            set.activity_items.push(ActivityItem {
                id: ItemId::new(format!("{prefix}.read-activity")).unwrap(),
                title: name,
                icon: "folder".into(),
                action: ActivityAction::ExecuteCommand(command),
                order: 150,
                when: When::default(),
            });
        }
    }
    set
}

fn record(
    app: &mut ConduitApp,
    request: &CommandInvocation,
    _: &mut Window,
    cx: &mut Context<ConduitApp>,
) -> bool {
    app.message = Some(format!("{:?}:{}", request.origin(), request.command()));
    cx.notify();
    true
}

fn plugins(overflow: usize) -> PluginUi {
    let mut builtins = builtins::register(true).unwrap();
    builtins
        .registry
        .unregister(builtins.example.as_ref().unwrap())
        .unwrap();
    // Host identity is issued by the library, never forged by the app adapter.
    let mut native = builtins::workspace_overview().unwrap();
    let extra = declarations(native.owner.clone(), overflow);
    let external = declarations(
        Owner::external(PluginId::new(EXTERNAL).unwrap()).unwrap(),
        overflow,
    );
    let commands: Vec<_> = extra
        .commands
        .iter()
        .chain(&external.commands)
        .filter(|c| !c.id.as_str().ends_with(".unbound"))
        .map(|c| c.id.to_string())
        .collect();
    native.commands.extend(extra.commands);
    native.activity_items.extend(extra.activity_items);
    native.status_items.extend(extra.status_items);
    for set in [native, external] {
        builtins
            .registry
            .register(set, BTreeSet::from([Capability::WorkspaceSummary]))
            .unwrap();
    }
    let mut ui = PluginUi::from_registry(builtins.registry);
    for id in commands {
        ui.bind_command(&id, record);
    }
    ui
}

#[gpui_kit::test]
fn toolbar_reflows_around_core_controls_in_wide_narrow_and_docked_layouts(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, store(), plugins(8));
    // Include a long core error. Its space is reserved before contribution packing.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.git.repository = Some(crate::git::Repository {
                current: "main".into(),
                detached: false,
                branches: vec![],
                truncated: false,
                upstream: Some("origin/main".into()),
                ahead: 12,
                context: Default::default(),
                branch_info: vec![],
                integration: None,
                conflicts: vec![],
                remotes: vec![],
            });
            app.git.last_branch_check = std::time::Instant::now();
            app.session_control
                .naming_errors
                .insert(1, "命名错误".repeat(50));
            cx.notify();
        })
    });
    // Match the native window's 850px minimum; docked content can be much narrower.
    for width in [1600., 1100., 900., 850., 1600.] {
        cx.simulate_resize(size(px(width), px(700.)));
        draw(cx);
        draw(cx); // measured remaining width is applied on the next frame
        let toolbar = cx.debug_bounds("conversation-toolbar").unwrap();
        let region = cx.debug_bounds("toolbar-contributions").unwrap();
        let controls = cx.debug_bounds("toolbar-controls").unwrap();
        let more = cx.debug_bounds("toolbar-more").unwrap();
        assert_eq!(toolbar.size.height, px(34.));
        assert!(
            controls.right() <= toolbar.right() && controls.right() <= px(width),
            "core controls at {width}: {controls:?}; toolbar {toolbar:?}"
        );
        assert!(region.right() <= controls.left());
        assert!(
            more.left() >= region.left() && more.right() <= region.right(),
            "more at {width}: {more:?} vs {region:?}"
        );
        for id in ["notification-bell", "performance-monitor", "toolbar-hints"] {
            let bounds = cx
                .debug_bounds(id)
                .or_else(|| {
                    cx.update(|window, _| {
                        test_support::find(window, &[], &gpui_kit::SharedString::from(id).into())
                            .map(|e| e.bounds())
                    })
                })
                .unwrap_or_else(|| panic!("missing {id}"));
            assert!(
                bounds.left() >= region.right() && bounds.right() <= toolbar.right(),
                "core {id} at {width}"
            );
        }
        let branch = cx.debug_bounds("git-branch-trigger").unwrap();
        let badge = cx.debug_bounds("nav-branches-badge").unwrap();
        assert!(badge.left() >= branch.left() && badge.right() <= branch.right());
        assert!(badge.top() >= branch.top() && badge.bottom() <= branch.bottom());
        cx.update(|window, _| {
            assert!(test_support::find(window, &[], &"git-push-pending".into()).is_none());
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let entries = app.status_entries();
            assert!(entries.iter().all(|entry| entry.width <= 180.));
            assert!(
                entries
                    .iter()
                    .all(|entry| entry.tooltip.contains(&entry.value.text))
            );
        });
    }
    // A dock reduces the conversation's width without resizing the window.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let command = app.plugins.host_command(builtins::EXAMPLE_COMMAND).unwrap();
            assert!(app.dispatch_plugin_command(&command, window, cx));
        })
    });
    draw(cx);
    draw(cx);
    let toolbar = cx.debug_bounds("conversation-toolbar").unwrap();
    let dock = cx.debug_bounds("detail-panel").unwrap();
    assert!(toolbar.right() <= dock.left());
    let controls = cx.debug_bounds("toolbar-controls").unwrap();
    assert!(controls.right() <= toolbar.right());
    cx.simulate_resize(size(px(1100.), px(700.)));
    draw(cx);
    draw(cx);
    let toolbar = cx.debug_bounds("conversation-toolbar").unwrap();
    assert!(cx.debug_bounds("toolbar-controls").unwrap().right() <= toolbar.right());
    assert!(cx.debug_bounds("toolbar-more").is_some());
    assert!(
        cx.debug_bounds("toolbar-workspace").is_none(),
        "when no complete item fits, only More remains"
    );
}

#[gpui_kit::test]
fn overflow_menu_scrolls_and_routes_last_enabled_command(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, store(), plugins(24));
    cx.simulate_resize(size(px(700.), px(350.)));
    draw(cx);
    draw(cx);
    let last = cx.update(|_, cx| {
        let app = app.read(cx);
        let entries = app.status_entries();
        let visible = visible_count(entries.iter().map(|e| e.width), app.plugins.status_width);
        let hidden: Vec<_> = entries[visible..]
            .iter()
            .filter_map(|e| e.request.as_ref())
            .collect();
        hidden.last().unwrap().command().to_string()
    });
    click(cx, "toolbar-more");
    for height in [350., 300., 350.] {
        cx.simulate_resize(size(px(700.), px(height)));
        draw(cx);
        cx.update(|window, _| {
            let menu = test_support::snapshots(window)
                .into_iter()
                .find(|e| e.role() == Some(gpui_kit::Role::Menu))
                .unwrap();
            assert!(menu.bounds().top() >= px(0.) && menu.bounds().bottom() <= px(height));
        });
    }
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::default());
    draw(cx);
    // From an unselected menu, Up selects and reveals the final enabled row.
    cx.simulate_keystrokes("up");
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).message.as_deref(),
            Some(format!("Status:{last}").as_str())
        )
    });
}

#[gpui_kit::test]
fn activity_and_status_commands_share_owner_policy_and_disable_unbound_actions(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx, store(), plugins(0));
    cx.simulate_resize(size(px(2200.), px(800.)));
    draw(cx);
    draw(cx);
    click(cx, "example.workspace-overview.read-activity");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).message.as_deref(),
            Some("Activity:example.workspace-overview.read")
        )
    });
    click(cx, "test.status-external.read-status");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).message.as_deref(),
            Some("Status:test.status-external.read")
        )
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        })
    });
    draw(cx);
    for id in [
        "example.workspace-overview.write-status",
        "example.workspace-overview.unbound-status",
        "test.status-external.read-status",
        "test.status-external.read-activity",
    ] {
        cx.update(|window, _| {
            assert_eq!(
                test_support::find(window, &[], &gpui_kit::SharedString::from(id).into())
                    .unwrap()
                    .disabled(),
                Some(true),
                "{id}"
            );
        });
    }
    click(cx, "example.workspace-overview.read-status");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).message.as_deref(),
            Some("Status:example.workspace-overview.read")
        )
    });
}

#[gpui_kit::test]
fn saved_commands_reject_scope_readonly_revocation_missing_capability_and_closing(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx, store(), plugins(0));
    let request = cx.update(|_, cx| {
        app.read(cx)
            .plugins
            .host_command("example.workspace-overview.read")
            .unwrap()
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.switch(1, Some(2), window, cx);
            assert!(!app.dispatch_plugin_command(&request, window, cx));
            let mutation = app
                .plugins
                .host_command("example.workspace-overview.write")
                .unwrap();
            let external = app
                .plugins
                .host_command("test.status-external.read")
                .unwrap();
            app.writable = false;
            assert!(!app.dispatch_plugin_command(&mutation, window, cx));
            assert!(!app.dispatch_plugin_command(&external, window, cx));
            let native = app
                .plugins
                .host_command("example.workspace-overview.read")
                .unwrap();
            assert!(app.dispatch_plugin_command(&native, window, cx));
            app.writable = true;
            app.sync_plugin_context();
            let old = app
                .plugins
                .host_command("example.workspace-overview.read")
                .unwrap();
            let owner = PluginId::new(NATIVE).unwrap();
            let registration = app.plugins.registry.registration(&owner).unwrap().clone();
            app.plugins.registry.unregister(&registration).unwrap();
            app.plugins
                .registry
                .register(
                    declarations(builtins::workspace_overview().unwrap().owner, 0),
                    BTreeSet::new(),
                )
                .unwrap();
            app.plugins
                .bind_command("example.workspace-overview.read", record);
            assert!(!app.dispatch_plugin_command(&old, window, cx));
            assert!(
                app.plugins
                    .host_command("example.workspace-overview.read")
                    .is_none()
            );
            let close = app
                .plugins
                .host_command("test.status-external.read")
                .unwrap();
            app.closing = true;
            assert!(!app.dispatch_plugin_command(&close, window, cx));
            assert!(
                app.plugins
                    .host_command(builtins::EXAMPLE_COMMAND)
                    .is_none()
            );
        })
    });
}

#[gpui_kit::test]
fn dynamic_status_uses_metadata_without_mutating_registry_and_empty_scope_hides_items(
    cx: &mut TestAppContext,
) {
    let (app, cx) = fixture(cx, store(), plugins(0));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let revision = app.plugins.registry.revision();
            let mut session = crate::app::tests::session();
            session.id = 99;
            app.store.workspaces[0].sessions.push(session);
            let entries = app.status_entries();
            let overview = entries
                .iter()
                .find(|e| e.item.id.as_str() == builtins::EXAMPLE_STATUS)
                .unwrap();
            assert_eq!(overview.value.text, "概览 · 2 个会话");
            let workspace = entries
                .iter()
                .find(|e| e.item.id.as_str() == builtins::WORKSPACE_STATUS)
                .unwrap();
            assert_eq!(workspace.value.text, app.store.workspaces[0].name());
            assert_eq!(app.plugins.registry.revision(), revision);
            assert_eq!(
                app.plugins
                    .registry
                    .status(&ItemId::new(builtins::EXAMPLE_STATUS).unwrap())
                    .unwrap()
                    .initial
                    .text,
                "工作区概览"
            );
            app.selected_workspace = None;
            app.selected_session = None;
            app.store = Store::default();
            app.sync_plugin_context();
            assert!(app.status_entries().is_empty());
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("toolbar-more").is_none());
    assert!(cx.debug_bounds("toolbar-workspace").is_none());
    assert!(cx.debug_bounds("notification-bell").is_some());
}
