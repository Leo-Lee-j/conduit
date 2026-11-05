//! Static, bounded declarations. This module neither reads files nor evaluates JS.
mod schema;
use crate::plugins::{
    commands::{ArgumentPolicy, Capability, CommandDescriptor},
    context::{Scope, When},
    contributions::{
        ActivityAction, ActivityItem, Alignment, ContributionSet, Location, StatusItem,
        StatusValue, ViewDescriptor,
    },
    ids::{CommandId, ItemId, Owner, PluginId, ViewId},
    registry::{HostCatalog, Registry, RendererSpec},
};
use anyhow::{Result, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
pub const HOST_API: &str = "0.1.0";
pub const SHELL_VERSION: &str = "0.8.0";
pub const SHELL_REVISION: &str = "11345ec002d9f182a266b5d5dd3a217090700a87";
pub const ICONS: &[&str] = &["folder", "file", "git-fork", "layout-dashboard"];

#[derive(Clone, Debug)]
pub struct CommandBinding {
    pub view: ViewId,
    pub command: String,
}

#[derive(Clone, Debug)]
pub struct Manifest {
    name: String,
    version: String,
    contributions: ContributionSet,
    permissions: BTreeSet<Capability>,
    entries: BTreeMap<ViewId, String>,
    commands: BTreeMap<CommandId, CommandBinding>,
    activation_events: BTreeSet<String>,
}

impl Manifest {
    pub fn parse(shell: &[u8], conduit: &[u8]) -> Result<Self> {
        ensure!(
            shell.len() <= MAX_MANIFEST_BYTES && conduit.len() <= MAX_MANIFEST_BYTES,
            "manifest exceeds 256 KiB"
        );
        let shell: schema::Shell = serde_json::from_slice(shell)?;
        let host: schema::Conduit = serde_json::from_slice(conduit)?;
        ensure!(
            host.schema_version == 1,
            "unsupported conduit schemaVersion"
        );
        ensure!(
            semver::VersionReq::parse(&host.host_api)?.matches(&semver::Version::parse(HOST_API)?),
            "incompatible hostApi"
        );
        semver::Version::parse(&shell.version)?;
        ensure!(
            shell.shell_version == SHELL_VERSION,
            "development runtime requires shell-version 0.8.0"
        );
        ensure!(
            !shell.capabilities.storage && shell.dependencies.is_empty(),
            "system capabilities and dependencies are disabled"
        );
        ensure!(
            !shell.name.trim().is_empty()
                && shell.name.len() <= 160
                && !shell.name.chars().any(char::is_control),
            "invalid plugin name"
        );
        entry_path(&shell.entry)?;
        let owner = Owner::external(PluginId::new(shell.id)?)?;
        let permissions = parse_permissions(&host.permissions)?;
        let mut set = ContributionSet::empty(owner);
        let mut entries = BTreeMap::new();
        for view in host.contributes.views {
            entry_path(&view.entry)?;
            let id = ViewId::new(view.id)?;
            ensure!(
                entries.insert(id.clone(), view.entry).is_none(),
                "duplicate view binding"
            );
            set.views.push(ViewDescriptor {
                renderer: id.to_string(),
                id,
                title: view.title,
                location: match view.location {
                    schema::Location::Navigation => Location::Navigation,
                    schema::Location::Right => Location::Right,
                },
                scope: scope(view.scope),
                when: When::parse(view.when.as_deref())?,
            });
        }
        ensure!(
            !entries.is_empty() && entries.values().any(|entry| entry == &shell.entry),
            "Shell entry must bind a declared view"
        );
        let mut commands = BTreeMap::new();
        for command in host.contributes.commands {
            let id = CommandId::new(command.id)?;
            let view = ViewId::new(command.binding.view)?;
            let target =
                set.views.iter().find(|v| v.id == view).ok_or_else(|| {
                    anyhow::anyhow!("command targets an undeclared or foreign view")
                })?;
            ensure!(
                target.scope == scope(command.scope),
                "command and target view scopes differ"
            );
            let name = command.binding.command;
            ensure!(
                !name.is_empty()
                    && name.len() <= 128
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b)),
                "invalid script command name"
            );
            let required = parse_permissions(&command.required_permissions)?;
            ensure!(
                required.is_subset(&permissions),
                "command requires an unrequested permission"
            );
            ensure!(
                commands
                    .insert(
                        id.clone(),
                        CommandBinding {
                            view,
                            command: name
                        }
                    )
                    .is_none(),
                "duplicate command binding"
            );
            set.commands.push(CommandDescriptor {
                id,
                title: command.title,
                scope: scope(command.scope),
                arguments: match command.arguments {
                    schema::Arguments::None => ArgumentPolicy::None,
                    schema::Arguments::Object {
                        max_bytes,
                        max_depth,
                    } => ArgumentPolicy::Object {
                        max_bytes,
                        max_depth,
                    },
                },
                required_capabilities: required,
                requires_writable: true,
            });
        }
        for item in host.contributes.activity_items {
            set.activity_items.push(ActivityItem {
                id: ItemId::new(item.id)?,
                title: item.title,
                icon: item.icon,
                action: match item.action {
                    schema::Action::OpenView { view } => {
                        ActivityAction::OpenView(ViewId::new(view)?)
                    }
                    schema::Action::Command { command } => {
                        ActivityAction::ExecuteCommand(CommandId::new(command)?)
                    }
                },
                order: item.order,
                when: When::parse(item.when.as_deref())?,
            });
        }
        for item in host.contributes.status_items {
            set.status_items.push(StatusItem {
                id: ItemId::new(item.id)?,
                scope: scope(item.scope),
                alignment: match item.alignment {
                    schema::Alignment::Left => Alignment::Left,
                    schema::Alignment::Right => Alignment::Right,
                },
                priority: item.priority,
                initial: StatusValue {
                    text: item.text,
                    tooltip: item.tooltip,
                },
                icon: item.icon,
                command: item.command.map(CommandId::new).transpose()?,
                when: When::parse(item.when.as_deref())?,
            });
        }
        let expected: BTreeSet<_> = entries
            .keys()
            .map(|id| format!("onView:{id}"))
            .chain(commands.keys().map(|id| format!("onCommand:{id}")))
            .collect();
        if let Some(events) = host.activation_events {
            let supplied: BTreeSet<_> = events.iter().cloned().collect();
            ensure!(
                supplied.len() == events.len() && supplied == expected,
                "activationEvents must exactly cover declared views and commands"
            );
        }
        let manifest = Self {
            name: shell.name,
            version: shell.version,
            contributions: set,
            permissions,
            entries,
            commands,
            activation_events: expected,
        };
        // Reuse every P1 identity, reference, text, icon and argument-budget check.
        let mut catalog = Self::host_catalog()?;
        manifest.allow_renderers(&mut catalog)?;
        Registry::new(catalog)
            .register(manifest.contributions.clone(), manifest.permissions.clone())?;
        Ok(manifest)
    }

    pub fn owner(&self) -> &PluginId {
        self.contributions.owner.id()
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn contributions(&self) -> &ContributionSet {
        &self.contributions
    }
    pub fn permissions(&self) -> &BTreeSet<Capability> {
        &self.permissions
    }
    pub fn entry(&self, view: &ViewId) -> Option<&str> {
        self.entries.get(view).map(String::as_str)
    }
    pub fn entries(&self) -> impl Iterator<Item = &str> {
        self.entries.values().map(String::as_str)
    }
    pub fn binding(&self, command: &CommandId) -> Option<&CommandBinding> {
        self.commands.get(command)
    }
    pub fn activation_events(&self) -> &BTreeSet<String> {
        &self.activation_events
    }
    pub fn host_catalog() -> Result<HostCatalog> {
        let mut catalog = HostCatalog::default();
        for icon in ICONS {
            catalog.allow_icon(*icon)?;
        }
        Ok(catalog)
    }
    pub fn allow_renderers(&self, catalog: &mut HostCatalog) -> Result<()> {
        for view in &self.contributions.views {
            catalog.allow_renderer(
                &view.renderer,
                RendererSpec {
                    owner: self.owner().clone(),
                    location: view.location,
                    scope: view.scope,
                },
            )?;
        }
        Ok(())
    }
}

fn scope(scope: schema::Scope) -> Scope {
    match scope {
        schema::Scope::Global => Scope::Global,
        schema::Scope::Workspace => Scope::Workspace,
        schema::Scope::Session => Scope::Session,
    }
}

fn parse_permissions(names: &[String]) -> Result<BTreeSet<Capability>> {
    let mut out = BTreeSet::new();
    for name in names {
        let permission = match name.as_str() {
            "workspace.metadata.read" => Capability::WorkspaceSummary,
            "session.summary.read" => Capability::SessionSummary,
            _ => bail!("unsupported permission: {name}"),
        };
        ensure!(out.insert(permission), "duplicate permission");
    }
    Ok(out)
}

/// Portable package names avoid Windows aliases, device names and ADS on every OS.
pub fn relative_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= 240 && !path.contains('\\'),
        "invalid package path"
    );
    for part in path.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "invalid package path component"
        );
        let stem = part.split('.').next().unwrap().to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        ensure!(!device, "reserved Windows device name");
    }
    Ok(())
}

fn entry_path(path: &str) -> Result<()> {
    relative_path(path)?;
    ensure!(
        path.ends_with(".js"),
        "view entry must be a package-relative .js file"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
