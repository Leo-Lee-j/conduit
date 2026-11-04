use super::{
    builtins as b, commands::*, context::*, contributions::*, ids::*, registry::*, state::*,
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::PathBuf};

fn workspace(name: &str) -> WorkspaceKey {
    WorkspaceKey::from_host_path(std::env::temp_dir().join("conduit-plugin-tests").join(name))
        .unwrap()
}
fn input(name: &str) -> ContextInput {
    let workspace = workspace(name);
    ContextInput {
        workspace: Some(workspace.clone()),
        session: Some(SessionKey { workspace, id: 7 }),
        writable: true,
        ..Default::default()
    }
}
fn context(tracker: &mut ContextTracker, name: &str) -> PluginContext {
    tracker.update(input(name)).unwrap().clone()
}
fn external(owner: &str) -> ContributionSet {
    let mut set = ContributionSet::empty(Owner::external(PluginId::new(owner).unwrap()).unwrap());
    set.views.push(ViewDescriptor {
        id: ViewId::new(format!("{owner}.index")).unwrap(),
        title: "Index".into(),
        location: Location::Navigation,
        scope: Scope::Workspace,
        renderer: format!("{owner}.renderer"),
        when: When::default(),
    });
    set.commands.push(CommandDescriptor {
        id: CommandId::new(format!("{owner}.show")).unwrap(),
        title: "Show".into(),
        scope: Scope::Workspace,
        arguments: ArgumentPolicy::None,
        required_capabilities: BTreeSet::from([Capability::WorkspaceSummary]),
        requires_writable: false,
    });
    set.activity_items.push(ActivityItem {
        id: ItemId::new(format!("{owner}.activity")).unwrap(),
        title: "Activity".into(),
        icon: "folder".into(),
        action: ActivityAction::OpenView(set.views[0].id.clone()),
        order: 0,
        when: When::default(),
    });
    set.status_items.push(StatusItem {
        id: ItemId::new(format!("{owner}.status")).unwrap(),
        scope: Scope::Workspace,
        alignment: Alignment::Right,
        priority: 0,
        initial: StatusValue {
            text: "Summary".into(),
            tooltip: None,
        },
        icon: None,
        command: Some(set.commands[0].id.clone()),
        when: When::default(),
    });
    set
}
fn catalog(sets: &[&ContributionSet]) -> HostCatalog {
    let mut catalog = HostCatalog::default();
    catalog.allow_icon("folder").unwrap();
    for set in sets {
        for view in &set.views {
            catalog
                .allow_renderer(
                    &view.renderer,
                    RendererSpec {
                        owner: set.owner.id().clone(),
                        location: view.location,
                        scope: view.scope,
                    },
                )
                .unwrap();
        }
    }
    catalog
}
fn grants() -> BTreeSet<Capability> {
    BTreeSet::from([Capability::WorkspaceSummary])
}

#[test]
fn reserved_owner_cannot_be_obtained_from_an_external_identifier() {
    assert!(Owner::external(PluginId::new("conduit.core").unwrap()).is_err());
    assert!(Owner::external(PluginId::new("example.valid-name").unwrap()).is_ok());
    for invalid in [
        "", "one", "a..b", ".a", "a.b.", "A.b", "a.b/c", "a.b\\c", "a.中文",
    ] {
        assert!(PluginId::new(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn registration_is_atomic_across_kinds_and_references() {
    let source = external("example.alpha");
    let mut registry = Registry::new(catalog(&[&source]));
    let mut duplicate = source.clone();
    duplicate.status_items[0].id = ItemId::new(duplicate.views[0].id.as_str()).unwrap();
    assert!(registry.register(duplicate, grants()).is_err());
    let mut missing = source.clone();
    missing.status_items[0].command = Some(CommandId::new("example.alpha.missing").unwrap());
    assert!(registry.register(missing, grants()).is_err());
    assert_eq!(registry.contribution_count(), 0);
    assert_eq!(registry.revision(), 0);
    let handle = registry.register(source.clone(), grants()).unwrap();
    assert!(registry.register(source, grants()).is_err());
    assert_eq!(registry.contribution_count(), 4);
    assert!(registry.is_registered(&handle));
}

#[test]
fn owner_renderer_scope_icon_and_navigation_references_are_checked() {
    let source = external("example.alpha");
    let other = external("example.beta");
    let mut registry = Registry::new(catalog(&[&source, &other]));
    registry.register(other.clone(), grants()).unwrap();
    let mut attempts = Vec::new();
    let mut invalid = source.clone();
    invalid.views[0].renderer = "missing.renderer".into();
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.views[0].renderer = other.views[0].renderer.clone();
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.views[0].scope = Scope::Session;
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.activity_items[0].icon = "https://image.invalid".into();
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.activity_items[0].action = ActivityAction::OpenView(other.views[0].id.clone());
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.status_items[0].command = Some(other.commands[0].id.clone());
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.views[0].id = ViewId::new("conduit.core.impersonation").unwrap();
    attempts.push(invalid);
    let mut invalid = source.clone();
    invalid.status_items[0].scope = Scope::Global;
    attempts.push(invalid);
    for set in attempts {
        assert!(registry.register(set, grants()).is_err());
        assert_eq!(registry.owner_count(), 1);
        assert_eq!(registry.contribution_count(), 4);
    }
    registry.register(source, grants()).unwrap();
}

#[test]
fn activities_cannot_directly_open_right_views() {
    let mut set = external("example.alpha");
    set.views[0].location = Location::Right;
    let mut registry = Registry::new(catalog(&[&set]));
    assert!(registry.register(set, grants()).is_err());
    assert_eq!(registry.contribution_count(), 0);
}

#[test]
fn conditions_are_restricted_and_do_not_grant_execution() {
    for invalid in [
        "",
        "workspace.unknown",
        "workspace.open || session.selected",
        "!!host.writable",
        "! host.writable",
        "(workspace.open)",
        "workspace.open &&",
        "true",
    ] {
        assert!(When::parse(Some(invalid)).is_err(), "{invalid}");
    }
    let mut tracker = ContextTracker::default();
    let cx = context(&mut tracker, "a");
    assert!(When::parse(None).unwrap().matches(&cx));
    assert!(
        When::parse(Some(
            " workspace.open && !session.running && host.writable "
        ))
        .unwrap()
        .matches(&cx)
    );
    let set = external("example.alpha");
    let mut registry = Registry::new(catalog(&[&set]));
    registry.register(set.clone(), BTreeSet::new()).unwrap();
    assert_eq!(registry.activities(&cx).len(), 1);
    assert_eq!(registry.statuses(&cx, Alignment::Right).len(), 1);
    assert!(
        registry
            .prepare_status_command(&set.status_items[0].id, &cx)
            .is_err()
    );
}

#[test]
fn activity_and_status_order_is_deterministic() {
    let a = external("example.alpha");
    let mut z = external("example.zeta");
    let mut registry = Registry::new(catalog(&[&a, &z]));
    z.status_items[0].priority = 10;
    registry.register(z.clone(), grants()).unwrap();
    registry.register(a.clone(), grants()).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    assert_eq!(
        registry
            .activities(&cx)
            .iter()
            .map(|i| &i.id)
            .collect::<Vec<_>>(),
        vec![&a.activity_items[0].id, &z.activity_items[0].id]
    );
    assert_eq!(
        registry
            .statuses(&cx, Alignment::Right)
            .iter()
            .map(|i| &i.id)
            .collect::<Vec<_>>(),
        vec![&z.status_items[0].id, &a.status_items[0].id]
    );
    let empty = ContextTracker::default()
        .update(ContextInput::default())
        .unwrap()
        .clone();
    assert!(registry.activities(&empty).is_empty());
    assert!(registry.statuses(&empty, Alignment::Right).is_empty());
}

#[test]
fn foreign_owner_and_foreign_registry_cannot_dispatch_or_revoke() {
    let a = external("example.alpha");
    let z = external("example.zeta");
    let mut registry = Registry::new(catalog(&[&a, &z]));
    let handle = registry.register(a.clone(), grants()).unwrap();
    registry.register(z.clone(), grants()).unwrap();
    let mut other_registry = Registry::new(catalog(&[&a]));
    let other_handle = other_registry.register(a.clone(), grants()).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    assert!(
        registry
            .prepare_command(
                &handle,
                &z.commands[0].id,
                Value::Null,
                CommandOrigin::Host,
                &cx
            )
            .is_err()
    );
    assert!(
        registry
            .prepare_command(
                &other_handle,
                &a.commands[0].id,
                Value::Null,
                CommandOrigin::Host,
                &cx
            )
            .is_err()
    );
    assert!(registry.unregister(&other_handle).is_err());
    assert!(registry.is_registered(&handle));
}

#[test]
fn unregister_invalidates_saved_actions_even_after_reregistration() {
    let set = external("example.alpha");
    let mut registry = Registry::new(catalog(&[&set]));
    let first = registry.register(set.clone(), grants()).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    let command = registry
        .prepare_status_command(&set.status_items[0].id, &cx)
        .unwrap();
    let view = registry.prepare_view(&set.views[0].id, &cx).unwrap();
    let removed = registry.unregister(&first).unwrap();
    assert_eq!(removed.views, vec![set.views[0].id.clone()]);
    assert_eq!(removed.statuses.len(), 1);
    assert!(registry.activities(&cx).is_empty());
    let second = registry.register(set, grants()).unwrap();
    assert_ne!(first.generation(), second.generation());
    assert!(registry.unregister(&first).is_err());
    assert!(registry.validate_command(&command, &cx).is_err());
    assert!(registry.validate_view(&view, &cx).is_err());
    assert!(registry.is_registered(&second));
}

#[test]
fn context_versions_follow_identity_not_workspace_order_or_summary_refresh() {
    let mut tracker = ContextTracker::default();
    let a = context(&mut tracker, "a");
    let same = context(&mut tracker, "a");
    assert_eq!(a.version(), same.version());
    let mut running = input("a");
    running.session_running = true;
    let running = tracker.update(running).unwrap().clone();
    assert_eq!(running.version(), a.version());
    let b = context(&mut tracker, "b");
    assert!(b.version() > a.version());
    let a_again = context(&mut tracker, "a");
    assert!(a_again.version() > b.version());
    let mut document = input("a");
    document.document = Some("document-2".into());
    assert!(tracker.update(document).unwrap().version() > a_again.version());
    assert!(WorkspaceKey::from_host_path(PathBuf::from("relative/path")).is_err());
    assert_ne!(
        workspace("Case"),
        workspace("case"),
        "do not lowercase host path identities"
    );
}

#[test]
fn foreign_session_is_removed_from_context() {
    let mut value = input("a");
    value.session = Some(SessionKey {
        workspace: workspace("b"),
        id: 7,
    });
    value.session_running = true;
    let cx = ContextTracker::default().update(value).unwrap().clone();
    assert!(cx.scope(Scope::Session).is_none());
    assert!(!When::parse(Some("session.running")).unwrap().matches(&cx));
}

#[test]
fn late_commands_and_view_requests_cannot_cross_context_or_readonly() {
    let set = external("example.alpha");
    let mut registry = Registry::new(catalog(&[&set]));
    registry.register(set.clone(), grants()).unwrap();
    let mut tracker = ContextTracker::default();
    let a = context(&mut tracker, "a");
    let command = registry
        .prepare_status_command(&set.status_items[0].id, &a)
        .unwrap();
    let view = registry.prepare_view(&set.views[0].id, &a).unwrap();
    let b = context(&mut tracker, "b");
    assert!(registry.validate_command(&command, &b).is_err());
    assert!(registry.validate_view(&view, &b).is_err());
    let mut readonly = input("a");
    readonly.writable = false;
    let readonly = tracker.update(readonly).unwrap().clone();
    assert!(
        registry
            .prepare_status_command(&set.status_items[0].id, &readonly)
            .is_err()
    );
    assert!(registry.prepare_view(&set.views[0].id, &readonly).is_err());
    assert!(registry.validate_command(&command, &readonly).is_err());
    let mut closing = input("a");
    closing.closing = true;
    let closing = tracker.update(closing).unwrap().clone();
    assert!(registry.prepare_view(&set.views[0].id, &closing).is_err());
}

#[test]
fn command_arguments_and_mutation_policy_are_bounded() {
    let mut set = external("example.alpha");
    set.commands[0].arguments = ArgumentPolicy::Object {
        max_bytes: 64,
        max_depth: 2,
    };
    set.status_items[0].command = None;
    let mut registry = Registry::new(catalog(&[&set]));
    let handle = registry.register(set.clone(), grants()).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    let prepare = |args| {
        registry.prepare_command(&handle, &set.commands[0].id, args, CommandOrigin::View, &cx)
    };
    assert!(prepare(json!({"name":"ok"})).is_ok());
    assert!(prepare(json!(["not-object"])).is_err());
    assert!(prepare(json!({"large":"a".repeat(100)})).is_err());
    assert!(prepare(json!({"one":{"two":{"three":1}}})).is_err());
    let mut invalid = external("example.invalid");
    invalid.commands[0]
        .required_capabilities
        .insert(Capability::HostMutation);
    let mut registry = Registry::new(catalog(&[&invalid]));
    assert!(registry.register(invalid, grants()).is_err());
}

#[test]
fn display_text_limits_reject_control_characters_and_large_payloads() {
    let set = external("example.alpha");
    let mut registry = Registry::new(catalog(&[&set]));
    for value in ["x".repeat(257), "line\nbreak".into()] {
        let mut invalid = set.clone();
        invalid.status_items[0].initial.text = value;
        assert!(registry.register(invalid, grants()).is_err());
    }
    let mut invalid = set.clone();
    invalid.status_items[0].initial.tooltip = Some("x".repeat(1025));
    assert!(registry.register(invalid, grants()).is_err());
    assert_eq!(registry.contribution_count(), 0);
    registry.register(set, grants()).unwrap();
}

#[test]
fn example_is_opt_in_and_all_three_contributions_share_one_owner() {
    let normal = b::register(false).unwrap();
    assert_eq!(normal.registry.owner_count(), 1);
    assert!(normal.example.is_none());
    let demo = b::register(true).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    let activity = demo
        .registry
        .prepare_activity(&ItemId::new(b::EXAMPLE_ACTIVITY).unwrap(), &cx)
        .unwrap();
    let ActionRequest::OpenView(navigation) = activity else {
        panic!("expected navigation")
    };
    let command = demo
        .registry
        .prepare_status_command(&ItemId::new(b::EXAMPLE_STATUS).unwrap(), &cx)
        .unwrap();
    assert_eq!(command.command().as_str(), b::EXAMPLE_COMMAND);
    demo.registry.validate_command(&command, &cx).unwrap();
    // Native command adapter will resolve this declaration to the summary renderer.
    let right = demo
        .registry
        .prepare_view(&ViewId::new(b::EXAMPLE_SUMMARY).unwrap(), &cx)
        .unwrap();
    assert_eq!(navigation.key().owner, right.key().owner);
    assert_eq!(navigation.key().scope, right.key().scope);
    assert_ne!(navigation.key(), right.key());
}

#[test]
fn unregister_restores_valid_fallbacks_and_preserves_collapse() {
    let mut demo = b::register(true).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    let mut state = WorkbenchState::new(
        &demo.registry,
        ViewId::new(b::WORKSPACES).unwrap(),
        ViewId::new(b::TOOL_DETAILS).unwrap(),
    )
    .unwrap();
    for id in [b::EXAMPLE_INDEX, b::EXAMPLE_SUMMARY] {
        state
            .open(
                &demo.registry,
                &demo
                    .registry
                    .prepare_view(&ViewId::new(id).unwrap(), &cx)
                    .unwrap(),
                &cx,
            )
            .unwrap();
    }
    state.set_collapsed(true);
    demo.registry
        .unregister(demo.example.as_ref().unwrap())
        .unwrap();
    state.reconcile(&demo.registry, &cx).unwrap();
    assert_eq!(state.navigation().as_str(), b::WORKSPACES);
    assert_eq!(state.right().as_str(), b::TOOL_DETAILS);
    assert!(state.collapsed());
    assert!(!state.right_open());
    assert!(
        state
            .navigation_key()
            .is_some_and(|k| k.owner.as_str() == b::CORE)
    );
    assert!(
        state
            .right_key()
            .is_some_and(|k| k.owner.as_str() == b::CORE)
    );
}

#[test]
fn close_preserves_selection_and_scope_changes_use_distinct_instance_keys() {
    let demo = b::register(true).unwrap();
    let mut tracker = ContextTracker::default();
    let cx = context(&mut tracker, "a");
    let mut state = WorkbenchState::new(
        &demo.registry,
        ViewId::new(b::WORKSPACES).unwrap(),
        ViewId::new(b::TOOL_DETAILS).unwrap(),
    )
    .unwrap();
    let request = demo
        .registry
        .prepare_view(&ViewId::new(b::EXAMPLE_SUMMARY).unwrap(), &cx)
        .unwrap();
    state.open(&demo.registry, &request, &cx).unwrap();
    let old_key = state.right_key().cloned().unwrap();
    state.close_right();
    assert_eq!(state.right().as_str(), b::EXAMPLE_SUMMARY);
    assert_eq!(state.right_key(), Some(&old_key));
    let next = context(&mut tracker, "b");
    state.reconcile(&demo.registry, &next).unwrap();
    assert_ne!(state.right_key(), Some(&old_key));
    assert!(!state.right_open());
    let removed = tracker
        .update(ContextInput {
            writable: true,
            ..Default::default()
        })
        .unwrap();
    state.reconcile(&demo.registry, removed).unwrap();
    assert_eq!(state.right().as_str(), b::TOOL_DETAILS);
    assert!(state.right_key().is_none());
}

#[test]
fn actions_without_payloads_reject_commands_requiring_arguments() {
    let mut set = external("example.alpha");
    set.commands[0].arguments = ArgumentPolicy::Object {
        max_bytes: 64,
        max_depth: 2,
    };
    let mut registry = Registry::new(catalog(&[&set]));
    assert!(registry.register(set.clone(), grants()).is_err());
    set.status_items[0].command = None;
    set.activity_items[0].action = ActivityAction::ExecuteCommand(set.commands[0].id.clone());
    assert!(registry.register(set, grants()).is_err());
    assert_eq!(registry.contribution_count(), 0);
}

#[test]
fn exact_json_budget_accepts_small_payload_and_rejects_escaped_overflow() {
    let mut set = external("example.alpha");
    set.commands[0].arguments = ArgumentPolicy::Object {
        max_bytes: 7,
        max_depth: 1,
    };
    set.status_items[0].command = None;
    let mut registry = Registry::new(catalog(&[&set]));
    let handle = registry.register(set.clone(), grants()).unwrap();
    let cx = context(&mut ContextTracker::default(), "a");
    assert!(
        registry
            .prepare_command(
                &handle,
                &set.commands[0].id,
                json!({"n":0}),
                CommandOrigin::View,
                &cx
            )
            .is_ok()
    );
    assert!(
        registry
            .prepare_command(
                &handle,
                &set.commands[0].id,
                json!({"":"\u{0000}"}),
                CommandOrigin::View,
                &cx
            )
            .is_err()
    );
}
