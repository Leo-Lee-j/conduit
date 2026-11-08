use super::{
    Error, Result,
    commands::{
        self, ArgumentPolicy, Capability, CommandDescriptor, CommandInvocation, CommandOrigin,
    },
    context::{PluginContext, Scope},
    contributions::*,
    ids::*,
    require,
    state::{ViewInstanceKey, ViewRequest},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Allowlist supplied by native host adapters, not by contributions.
#[derive(Clone, Debug)]
pub struct RendererSpec {
    pub owner: PluginId,
    pub location: Location,
    pub scope: Scope,
}
#[derive(Default)]
pub struct HostCatalog {
    renderers: BTreeMap<String, RendererSpec>,
    icons: BTreeSet<String>,
}
impl HostCatalog {
    pub fn allow_renderer(&mut self, key: impl Into<String>, spec: RendererSpec) -> Result<()> {
        let key = key.into();
        ItemId::new(key.clone())?;
        require(
            !self.renderers.contains_key(&key),
            "renderer already declared",
        )?;
        self.renderers.insert(key, spec);
        Ok(())
    }
    pub fn allow_icon(&mut self, icon: impl Into<String>) -> Result<()> {
        let icon = icon.into();
        require(
            !icon.is_empty()
                && icon.len() <= 64
                && icon.bytes().all(|c| c.is_ascii_lowercase() || c == b'-'),
            "invalid host icon name",
        )?;
        self.icons.insert(icon);
        Ok(())
    }
}
/// Explicit host-owned revocation handle. Drop does not silently unregister UI state.
#[derive(Clone, Debug)]
pub struct Registration {
    registry: Arc<()>,
    owner: PluginId,
    generation: u64,
}
impl Registration {
    pub fn owner(&self) -> &PluginId {
        &self.owner
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
}
struct Entry {
    set: ContributionSet,
    handle: Registration,
    grants: BTreeSet<Capability>,
}
pub struct Registry {
    identity: Arc<()>,
    generation: u64,
    catalog: HostCatalog,
    entries: BTreeMap<PluginId, Entry>,
    ids: BTreeMap<String, PluginId>,
}
#[derive(Clone, Debug)]
pub struct Removed {
    pub owner: PluginId,
    pub views: Vec<ViewId>,
    pub commands: Vec<CommandId>,
    pub activities: Vec<ItemId>,
    pub statuses: Vec<ItemId>,
    pub file_icon_themes: Vec<ItemId>,
}
#[derive(Clone, Debug)]
pub enum ActionRequest {
    OpenView(ViewRequest),
    Command(CommandInvocation),
}

pub(super) fn text(value: &str, max: usize, nonempty: bool) -> Result<()> {
    require(
        (!nonempty || !value.trim().is_empty())
            && value.len() <= max
            && !value.chars().any(char::is_control),
        "invalid or oversized display text",
    )
}
pub(super) fn status_value(value: &StatusValue) -> Result<()> {
    text(&value.text, 256, false)?;
    if let Some(tooltip) = &value.tooltip {
        text(tooltip, 1024, false)?;
    }
    Ok(())
}
impl Registry {
    pub fn new(catalog: HostCatalog) -> Self {
        Self {
            identity: Arc::new(()),
            generation: 0,
            catalog,
            entries: BTreeMap::new(),
            ids: BTreeMap::new(),
        }
    }
    pub(super) fn is_host_owner(&self, owner: &PluginId) -> bool {
        self.entries
            .get(owner)
            .is_some_and(|e| e.set.owner.is_host())
    }
    pub fn owner_count(&self) -> usize {
        self.entries.len()
    }
    pub fn contribution_count(&self) -> usize {
        self.ids.len()
    }
    pub fn revision(&self) -> u64 {
        self.generation
    }
    pub fn owner_for(&self, id: &str) -> Option<&PluginId> {
        self.ids.get(id)
    }
    fn entry_for(&self, id: &str) -> Option<&Entry> {
        self.entries.get(self.ids.get(id)?)
    }
    pub fn registration(&self, owner: &PluginId) -> Option<&Registration> {
        self.entries.get(owner).map(|e| &e.handle)
    }
    pub fn is_registered(&self, handle: &Registration) -> bool {
        Arc::ptr_eq(&self.identity, &handle.registry)
            && self
                .entries
                .get(&handle.owner)
                .is_some_and(|entry| entry.handle.generation == handle.generation)
    }
    pub fn view(&self, id: &ViewId) -> Option<&ViewDescriptor> {
        self.entry_for(id.as_str())?
            .set
            .views
            .iter()
            .find(|v| v.id == *id)
    }
    pub fn command(&self, id: &CommandId) -> Option<&CommandDescriptor> {
        self.entry_for(id.as_str())?
            .set
            .commands
            .iter()
            .find(|v| v.id == *id)
    }
    pub fn status(&self, id: &ItemId) -> Option<&StatusItem> {
        self.entry_for(id.as_str())?
            .set
            .status_items
            .iter()
            .find(|v| v.id == *id)
    }
    pub fn file_icon_themes(&self) -> impl Iterator<Item = &FileIconThemeDescriptor> {
        self.entries
            .values()
            .flat_map(|entry| entry.set.file_icon_themes.iter())
    }
    pub fn views(&self) -> impl Iterator<Item = &ViewDescriptor> {
        self.entries.values().flat_map(|e| &e.set.views)
    }
    pub fn activities(&self, cx: &PluginContext) -> Vec<&ActivityItem> {
        let mut items: Vec<_> = self
            .entries
            .values()
            .flat_map(|e| &e.set.activity_items)
            .filter(|i| {
                i.when.matches(cx)
                    && match &i.action {
                        ActivityAction::OpenView(id) => self
                            .view(id)
                            .is_some_and(|v| v.when.matches(cx) && cx.scope(v.scope).is_some()),
                        ActivityAction::ExecuteCommand(_) => true,
                    }
            })
            .collect();
        items.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        items
    }
    pub fn statuses(&self, cx: &PluginContext, alignment: Alignment) -> Vec<&StatusItem> {
        let mut items: Vec<_> = self
            .entries
            .values()
            .flat_map(|e| &e.set.status_items)
            .filter(|i| {
                i.alignment == alignment && i.when.matches(cx) && cx.scope(i.scope).is_some()
            })
            .collect();
        items.sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
        items
    }
    pub fn register(
        &mut self,
        set: ContributionSet,
        grants: BTreeSet<Capability>,
    ) -> Result<Registration> {
        require(
            !self.entries.contains_key(set.owner.id()),
            "owner already registered",
        )?;
        require(self.entries.len() < 128, "too many owners")?;
        let mut ids = BTreeSet::new();
        for id in set
            .commands
            .iter()
            .map(|i| i.id.as_str())
            .chain(set.views.iter().map(|i| i.id.as_str()))
            .chain(set.activity_items.iter().map(|i| i.id.as_str()))
            .chain(set.status_items.iter().map(|i| i.id.as_str()))
            .chain(set.file_icon_themes.iter().map(|i| i.id.as_str()))
        {
            require(
                set.owner.owns(id),
                format!("contribution outside owner namespace: {id}"),
            )?;
            require(
                !self.ids.contains_key(id) && ids.insert(id.to_owned()),
                format!("duplicate contribution: {id}"),
            )?;
        }
        require(
            !ids.is_empty() && ids.len() <= 512,
            "contribution set must have 1..512 entries",
        )?;
        for theme in &set.file_icon_themes {
            text(&theme.title, 160, true)?;
        }
        for command in &set.commands {
            text(&command.title, 160, true)?;
            if let ArgumentPolicy::Object {
                max_bytes,
                max_depth,
            } = command.arguments
            {
                require(
                    (1..=65536).contains(&max_bytes) && (1..=16).contains(&max_depth),
                    "invalid command argument budget",
                )?;
            }
            require(
                !command
                    .required_capabilities
                    .contains(&Capability::HostMutation)
                    || command.requires_writable,
                "mutating command must require writable host",
            )?;
        }
        for view in &set.views {
            text(&view.title, 160, true)?;
            let spec = self
                .catalog
                .renderers
                .get(&view.renderer)
                .ok_or_else(|| Error(format!("unknown renderer: {}", view.renderer)))?;
            require(
                spec.owner == *set.owner.id()
                    && spec.location == view.location
                    && spec.scope == view.scope,
                "renderer owner, location or scope mismatch",
            )?;
        }
        for item in &set.activity_items {
            text(&item.title, 160, true)?;
            require(self.catalog.icons.contains(&item.icon), "unknown host icon")?;
            match &item.action {
                ActivityAction::OpenView(id) => require(
                    set.views
                        .iter()
                        .any(|v| v.id == *id && v.location == Location::Navigation),
                    "activity must reference an owned navigation view",
                )?,
                ActivityAction::ExecuteCommand(id) => require(
                    set.commands
                        .iter()
                        .any(|c| c.id == *id && matches!(c.arguments, ArgumentPolicy::None)),
                    "activity must reference an owned command accepting no arguments",
                )?,
            }
        }
        for item in &set.status_items {
            status_value(&item.initial)?;
            require(
                item.icon
                    .as_ref()
                    .is_none_or(|i| self.catalog.icons.contains(i)),
                "unknown status icon",
            )?;
            if let Some(id) = &item.command {
                require(
                    set.commands.iter().any(|c| {
                        c.id == *id
                            && c.scope == item.scope
                            && matches!(c.arguments, ArgumentPolicy::None)
                    }),
                    "status must reference an owned command with the same scope accepting no arguments",
                )?;
            }
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error("registration generation exhausted".into()))?;
        let handle = Registration {
            registry: self.identity.clone(),
            owner: set.owner.id().clone(),
            generation,
        };
        // Everything above is fallible validation. Publish the entire set only now.
        for id in ids {
            self.ids.insert(id, set.owner.id().clone());
        }
        self.entries.insert(
            set.owner.id().clone(),
            Entry {
                set,
                handle: handle.clone(),
                grants,
            },
        );
        self.generation = generation;
        Ok(handle)
    }
    pub fn unregister(&mut self, handle: &Registration) -> Result<Removed> {
        require(self.is_registered(handle), "stale or foreign registration")?;
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Error("registration generation exhausted".into()))?;
        let entry = self.entries.remove(&handle.owner).unwrap();
        self.ids.retain(|_, owner| *owner != handle.owner);
        self.generation = generation;
        Ok(Removed {
            owner: handle.owner.clone(),
            views: entry.set.views.into_iter().map(|v| v.id).collect(),
            commands: entry.set.commands.into_iter().map(|v| v.id).collect(),
            activities: entry.set.activity_items.into_iter().map(|v| v.id).collect(),
            statuses: entry.set.status_items.into_iter().map(|v| v.id).collect(),
            file_icon_themes: entry
                .set
                .file_icon_themes
                .into_iter()
                .map(|v| v.id)
                .collect(),
        })
    }
    pub fn prepare_command(
        &self,
        handle: &Registration,
        id: &CommandId,
        args: Value,
        origin: CommandOrigin,
        cx: &PluginContext,
    ) -> Result<CommandInvocation> {
        require(self.is_registered(handle), "stale or foreign registration")?;
        let entry = &self.entries[&handle.owner];
        let command = entry
            .set
            .commands
            .iter()
            .find(|c| c.id == *id)
            .ok_or_else(|| Error("command does not belong to caller".into()))?;
        commands::eligible(&entry.set.owner, &entry.grants, command, cx)?;
        command.arguments.validate(&args)?;
        Ok(CommandInvocation {
            registration: handle.clone(),
            command: id.clone(),
            args,
            origin,
            context_version: cx.version(),
            scope: cx.scope(command.scope).unwrap(),
        })
    }
    /// Recheck immediately before the app handler runs; never execute a saved callback directly.
    pub fn validate_command(
        &self,
        invocation: &CommandInvocation,
        cx: &PluginContext,
    ) -> Result<&CommandDescriptor> {
        require(
            self.is_registered(&invocation.registration),
            "command owner was revoked",
        )?;
        require(
            invocation.context_version == cx.version() && cx.matches_scope(&invocation.scope),
            "stale command context",
        )?;
        let entry = &self.entries[invocation.registration.owner()];
        let command = entry
            .set
            .commands
            .iter()
            .find(|c| c.id == invocation.command)
            .ok_or_else(|| Error("command was removed".into()))?;
        commands::eligible(&entry.set.owner, &entry.grants, command, cx)?;
        command.arguments.validate(&invocation.args)?;
        Ok(command)
    }
    /// Host navigation entry point. A future external API must bind the caller owner
    /// before invoking this method; contributions already enforce same-owner references.
    pub fn prepare_view(&self, id: &ViewId, cx: &PluginContext) -> Result<ViewRequest> {
        let entry = self
            .entry_for(id.as_str())
            .ok_or_else(|| Error("unknown view".into()))?;
        let view = self
            .view(id)
            .ok_or_else(|| Error("identifier is not a view".into()))?;
        require(
            !cx.input().closing && (entry.set.owner.is_host() || cx.writable()),
            "view activation denied",
        )?;
        require(view.when.matches(cx), "view is not visible in this context")?;
        let scope = cx
            .scope(view.scope)
            .ok_or_else(|| Error("view scope is unavailable".into()))?;
        Ok(ViewRequest {
            registration: entry.handle.clone(),
            key: ViewInstanceKey {
                owner: entry.set.owner.id().clone(),
                view: id.clone(),
                scope,
            },
            context_version: cx.version(),
        })
    }
    pub fn validate_view(
        &self,
        request: &ViewRequest,
        cx: &PluginContext,
    ) -> Result<&ViewDescriptor> {
        require(
            self.is_registered(&request.registration),
            "view owner was revoked",
        )?;
        require(
            request.context_version == cx.version() && cx.matches_scope(&request.key.scope),
            "stale view context",
        )?;
        self.prepare_view(&request.key.view, cx)?;
        self.view(&request.key.view)
            .ok_or_else(|| Error("view was removed".into()))
    }
    pub fn prepare_activity(&self, id: &ItemId, cx: &PluginContext) -> Result<ActionRequest> {
        let entry = self
            .entry_for(id.as_str())
            .ok_or_else(|| Error("unknown activity".into()))?;
        let item = entry
            .set
            .activity_items
            .iter()
            .find(|i| i.id == *id)
            .ok_or_else(|| Error("identifier is not an activity".into()))?;
        require(item.when.matches(cx), "activity is not visible")?;
        match &item.action {
            ActivityAction::OpenView(id) => self.prepare_view(id, cx).map(ActionRequest::OpenView),
            ActivityAction::ExecuteCommand(id) => self
                .prepare_command(&entry.handle, id, Value::Null, CommandOrigin::Activity, cx)
                .map(ActionRequest::Command),
        }
    }
    pub fn prepare_status_command(
        &self,
        id: &ItemId,
        cx: &PluginContext,
    ) -> Result<CommandInvocation> {
        let entry = self
            .entry_for(id.as_str())
            .ok_or_else(|| Error("unknown status item".into()))?;
        let item = self
            .status(id)
            .ok_or_else(|| Error("identifier is not a status item".into()))?;
        require(
            item.when.matches(cx) && cx.scope(item.scope).is_some(),
            "status item is not visible",
        )?;
        let command = item
            .command
            .as_ref()
            .ok_or_else(|| Error("status item has no command".into()))?;
        self.prepare_command(
            &entry.handle,
            command,
            Value::Null,
            CommandOrigin::Status,
            cx,
        )
    }
}
