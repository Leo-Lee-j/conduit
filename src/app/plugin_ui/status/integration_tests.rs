use crate::app::{
    ConduitApp,
    plugin_ui::{
        PluginUi,
        tests::{fixture, store},
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
use gpui_kit::{Context, TestAppContext, Window};
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
fn saved_commands_reject_scope_readonly_revocation_missing_capability_and_closing(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
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

#[cfg(conduit_shell_desktop)]
mod cache_tests {
    use super::*;
    use crate::app::chat::profiling;
    use gpui_kit::{Focusable as _, Role, VisualTestContext};

    fn frame(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
}
