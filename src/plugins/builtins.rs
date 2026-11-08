//! Declarations only. Native adapters still own renderers and command handlers.
//! The example is explicitly opt-in; declaring a catalog never executes a renderer.
use super::{Result, commands::*, context::*, contributions::*, ids::*, registry::*};
use std::collections::BTreeSet;

pub const CORE: &str = "conduit.core";
pub const WORKSPACES: &str = "conduit.core.workspaces";
pub const FILES: &str = "conduit.core.files";
pub const GIT: &str = "conduit.core.git";
pub const BRANCHES: &str = "conduit.core.branches";
pub const TOOL_DETAILS: &str = "conduit.core.tool-details";
pub const NOTIFICATIONS: &str = "conduit.core.notifications";
pub const SHOW_TOOL_DETAILS: &str = "conduit.core.show-tool-details";
pub const TOGGLE_TERMINAL: &str = "conduit.core.toggle-terminal";
pub const NEW_TERMINAL: &str = "conduit.core.new-terminal";
pub const WORKSPACE_STATUS: &str = "conduit.core.workspace-status";
pub const EXAMPLE: &str = "example.workspace-overview";
pub const EXAMPLE_INDEX: &str = "example.workspace-overview.index";
pub const EXAMPLE_SUMMARY: &str = "example.workspace-overview.summary";
pub const EXAMPLE_COMMAND: &str = "example.workspace-overview.show-summary";
pub const EXAMPLE_ACTIVITY: &str = "example.workspace-overview.activity";
pub const EXAMPLE_STATUS: &str = "example.workspace-overview.status";

fn view(id: &str, title: &str, location: Location, scope: Scope) -> Result<ViewDescriptor> {
    Ok(ViewDescriptor {
        id: ViewId::new(id)?,
        title: title.into(),
        location,
        scope,
        renderer: id.into(),
        when: When::default(),
    })
}
fn activity(id: &str, title: &str, icon: &str, view: &str, order: i32) -> Result<ActivityItem> {
    Ok(ActivityItem {
        id: ItemId::new(id)?,
        title: title.into(),
        icon: icon.into(),
        action: ActivityAction::OpenView(ViewId::new(view)?),
        order,
        when: When::default(),
    })
}
pub fn core() -> Result<ContributionSet> {
    let mut set = ContributionSet::empty(Owner::host(PluginId::new(CORE)?));
    // Core navigation containers remain accessible without a workspace. Their
    // existing adapters own per-workspace file/Git caches; extension views retain
    // their declared workspace/session scope.
    set.views = vec![
        view(
            WORKSPACES,
            conduit::i18n::text_static("plugins.builtins.workspaces_and_sessions"),
            Location::Navigation,
            Scope::Global,
        )?,
        view(
            FILES,
            conduit::i18n::text_static("plugins.builtins.files"),
            Location::Navigation,
            Scope::Global,
        )?,
        view(
            GIT,
            conduit::i18n::text_static("plugins.builtins.git_changes"),
            Location::Navigation,
            Scope::Global,
        )?,
        view(
            BRANCHES,
            conduit::i18n::text_static("plugin_ui.git_branches_and_log"),
            Location::Navigation,
            Scope::Global,
        )?,
        view(
            TOOL_DETAILS,
            conduit::i18n::text_static("plugins.builtins.tool_details"),
            Location::Right,
            Scope::Session,
        )?,
        view(
            NOTIFICATIONS,
            conduit::i18n::text_static("plugins.builtins.notifications"),
            Location::Right,
            Scope::Global,
        )?,
    ];
    set.activity_items = vec![
        activity(
            "conduit.core.workspaces-entry",
            conduit::i18n::text_static("plugins.builtins.workspaces_and_sessions"),
            "folder",
            WORKSPACES,
            0,
        )?,
        activity(
            "conduit.core.files-entry",
            conduit::i18n::text_static("plugins.builtins.files"),
            "file",
            FILES,
            10,
        )?,
        activity(
            "conduit.core.git-entry",
            conduit::i18n::text_static("plugins.builtins.git_changes"),
            "git-fork",
            GIT,
            20,
        )?,
    ];
    set.commands.push(CommandDescriptor {
        id: CommandId::new(SHOW_TOOL_DETAILS)?,
        title: conduit::i18n::text_static("plugins.builtins.open_tool_details").into(),
        scope: Scope::Session,
        arguments: ArgumentPolicy::None,
        required_capabilities: BTreeSet::from([Capability::SessionSummary]),
        requires_writable: false,
    });
    for (id, title, scope, requires_writable) in [
        (
            TOGGLE_TERMINAL,
            conduit::i18n::text_static("plugins.builtins.open_or_collapse_terminal"),
            Scope::Global,
            false,
        ),
        (
            NEW_TERMINAL,
            conduit::i18n::text_static("plugins.builtins.new_terminal_in_current_workspace"),
            Scope::Workspace,
            true,
        ),
    ] {
        set.commands.push(CommandDescriptor {
            id: CommandId::new(id)?,
            title: title.into(),
            scope,
            arguments: ArgumentPolicy::None,
            required_capabilities: BTreeSet::new(),
            requires_writable,
        });
    }
    set.status_items.push(StatusItem {
        id: ItemId::new(WORKSPACE_STATUS)?,
        scope: Scope::Workspace,
        alignment: Alignment::Left,
        priority: 100,
        initial: StatusValue {
            text: conduit::i18n::text_static("git_compare.workspace").into(),
            tooltip: None,
        },
        icon: Some("folder".into()),
        command: None,
        when: When::default(),
    });
    Ok(set)
}
pub fn workspace_overview() -> Result<ContributionSet> {
    let mut set = ContributionSet::empty(Owner::host(PluginId::new(EXAMPLE)?));
    set.commands.push(CommandDescriptor {
        id: CommandId::new(EXAMPLE_COMMAND)?,
        title: conduit::i18n::text_static("plugins.builtins.open_workspace_overview").into(),
        scope: Scope::Workspace,
        arguments: ArgumentPolicy::None,
        required_capabilities: BTreeSet::from([Capability::WorkspaceSummary]),
        requires_writable: false,
    });
    set.views = vec![
        view(
            EXAMPLE_INDEX,
            conduit::i18n::text_static("plugin_ui.example.workspace_overview"),
            Location::Navigation,
            Scope::Workspace,
        )?,
        view(
            EXAMPLE_SUMMARY,
            conduit::i18n::text_static("plugin_ui.example.workspace_summary"),
            Location::Right,
            Scope::Workspace,
        )?,
    ];
    set.activity_items.push(activity(
        EXAMPLE_ACTIVITY,
        conduit::i18n::text_static("plugin_ui.example.workspace_overview"),
        "layout-dashboard",
        EXAMPLE_INDEX,
        100,
    )?);
    set.status_items.push(StatusItem {
        id: ItemId::new(EXAMPLE_STATUS)?,
        scope: Scope::Workspace,
        alignment: Alignment::Right,
        priority: 10,
        initial: StatusValue {
            text: conduit::i18n::text_static("plugin_ui.example.workspace_overview").into(),
            tooltip: Some(
                conduit::i18n::text_static("plugins.builtins.view_current_workspace_summary")
                    .into(),
            ),
        },
        icon: None,
        command: Some(CommandId::new(EXAMPLE_COMMAND)?),
        when: When::parse(Some("workspace.open"))?,
    });
    Ok(set)
}
pub struct Builtins {
    pub registry: Registry,
    pub core: Registration,
    pub example: Option<Registration>,
}
pub fn register(include_example: bool) -> Result<Builtins> {
    let core = core()?;
    let example = include_example.then(workspace_overview).transpose()?;
    let mut catalog = HostCatalog::default();
    for icon in ["folder", "file", "git-fork", "layout-dashboard"] {
        catalog.allow_icon(icon)?;
    }
    for set in std::iter::once(&core).chain(example.iter()) {
        for view in &set.views {
            catalog.allow_renderer(
                &view.renderer,
                RendererSpec {
                    owner: set.owner.id().clone(),
                    location: view.location,
                    scope: view.scope,
                },
            )?;
        }
    }
    let mut registry = Registry::new(catalog);
    let core = registry.register(core, BTreeSet::from([Capability::SessionSummary]))?;
    let example = example
        .map(|set| registry.register(set, BTreeSet::from([Capability::WorkspaceSummary])))
        .transpose()?;
    Ok(Builtins {
        registry,
        core,
        example,
    })
}
