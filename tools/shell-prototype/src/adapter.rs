//! Development loading through P1 registrations, scoped instances and completion tickets.
use crate::{
    manifest::Manifest,
    package::PackageSnapshot,
    plugins::{
        commands::{Capability, CommandInvocation, CommandOrigin},
        context::{ContextInput, ContextTracker, PluginContext},
        contributions::{ActivityItem, Alignment, Location, StatusItem, ViewDescriptor},
        ids::{CommandId, ItemId, PluginId, ViewId},
        instances::{InstanceCache, RequestTicket},
        registry::{ActionRequest, Registry},
        state::ViewInstanceKey,
    },
};
use anyhow::{Context, Result, ensure};
use gpui_kit::{App, Entity, Window};
use gpui_shell::{
    CommandCall, CommandReply, HostError, HostModule, HostValue, RuntimeLimits, ShellRuntime,
    policy::Policy, view::ScriptView,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::{Rc, Weak},
};

/// Explicit, session-only approval supplied by the host. A manifest is only a request.
pub struct ApprovedPackage {
    snapshot: Rc<PackageSnapshot>,
    grants: BTreeSet<Capability>,
}
impl ApprovedPackage {
    pub fn manifest(&self) -> &Manifest {
        self.snapshot.manifest()
    }
    pub fn new(snapshot: PackageSnapshot, grants: BTreeSet<Capability>) -> Result<Self> {
        ensure!(
            &grants == snapshot.manifest().permissions(),
            "all requested business permissions must be explicitly approved"
        );
        Ok(Self {
            snapshot: Rc::new(snapshot),
            grants,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceMetadata {
    pub id: String,
    pub name: String,
    pub session_count: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostData {
    pub workspace: Option<WorkspaceMetadata>,
}
impl HostData {
    fn validate(&self, input: &ContextInput) -> Result<()> {
        ensure!(
            self.workspace.is_some() == input.workspace.is_some(),
            "workspace summary and context differ"
        );
        if let Some(workspace) = &self.workspace {
            for value in [&workspace.id, &workspace.name] {
                ensure!(
                    !value.trim().is_empty()
                        && value.len() <= 160
                        && !value.chars().any(char::is_control),
                    "invalid workspace summary"
                );
            }
        }
        Ok(())
    }
}

struct State {
    registry: Registry,
    context: PluginContext,
    data: HostData,
    active: BTreeMap<PluginId, u64>,
    epochs: BTreeMap<PluginId, Rc<()>>,
    instances: InstanceCache<Entity<ScriptView>>,
}
struct OwnerRuntime {
    policy: Rc<Policy>,
    runtime: Rc<ShellRuntime>,
}

pub struct DevelopmentHost {
    state: Rc<RefCell<State>>,
    tracker: ContextTracker,
    packages: BTreeMap<PluginId, ApprovedPackage>,
    runtimes: BTreeMap<PluginId, OwnerRuntime>,
    generation: u64,
}

pub struct PendingCommand {
    ticket: RequestTicket,
    invocation: CommandInvocation,
    call: CommandCall,
}
/// A host-issued action. Stop invalidates saved actions even if context is unchanged.
pub struct PreparedCommand {
    invocation: CommandInvocation,
    epoch: Rc<()>,
}
pub enum PreparedActivity {
    OpenView(crate::plugins::state::ViewRequest),
    Command(PreparedCommand),
}
pub struct CompletedCommand {
    ticket: RequestTicket,
    invocation: CommandInvocation,
    reply: CommandReply,
}
impl PendingCommand {
    pub fn target(&self) -> &ViewInstanceKey {
        self.ticket.key()
    }
    pub async fn wait(self) -> CompletedCommand {
        CompletedCommand {
            ticket: self.ticket,
            invocation: self.invocation,
            reply: self.call.wait().await,
        }
    }
}

impl DevelopmentHost {
    /// Registration is atomic and entirely static. It does not create a VM.
    pub fn new(
        packages: Vec<ApprovedPackage>,
        input: ContextInput,
        data: HostData,
    ) -> Result<Self> {
        let mut catalog = Manifest::host_catalog()?;
        for package in &packages {
            package.snapshot.manifest().allow_renderers(&mut catalog)?;
        }
        let mut registry = Registry::new(catalog);
        let mut approved = BTreeMap::new();
        for package in packages {
            let manifest = package.snapshot.manifest();
            registry.register(manifest.contributions().clone(), package.grants.clone())?;
            approved.insert(manifest.owner().clone(), package);
        }
        data.validate(&input)?;
        let epochs = approved
            .keys()
            .map(|owner| (owner.clone(), Rc::new(())))
            .collect();
        let mut tracker = ContextTracker::default();
        let context = tracker.update(input)?.clone();
        Ok(Self {
            state: Rc::new(RefCell::new(State {
                registry,
                context,
                data,
                active: BTreeMap::new(),
                epochs,
                instances: InstanceCache::new(32),
            })),
            tracker,
            packages: approved,
            runtimes: BTreeMap::new(),
            generation: 0,
        })
    }

    pub fn runtime_count(&self) -> usize {
        self.runtimes.len()
    }
    pub fn instance_count(&self) -> usize {
        self.state.borrow().instances.len()
    }
    pub fn context(&self) -> PluginContext {
        self.state.borrow().context.clone()
    }
    pub fn views(&self) -> Vec<ViewDescriptor> {
        self.state.borrow().registry.views().cloned().collect()
    }
    pub fn activities(&self) -> Vec<ActivityItem> {
        let state = self.state.borrow();
        state
            .registry
            .activities(&state.context)
            .into_iter()
            .cloned()
            .collect()
    }
    pub fn statuses(&self) -> Vec<StatusItem> {
        let state = self.state.borrow();
        [Alignment::Left, Alignment::Right]
            .into_iter()
            .flat_map(|alignment| {
                state
                    .registry
                    .statuses(&state.context, alignment)
                    .into_iter()
                    .cloned()
            })
            .collect()
    }
    pub fn view_title(&self, id: &ViewId) -> Result<String> {
        Ok(self
            .state
            .borrow()
            .registry
            .view(id)
            .context("unknown view")?
            .title
            .clone())
    }
    pub fn prepare_activity(&self, id: &ItemId) -> Result<PreparedActivity> {
        let state = self.state.borrow();
        Ok(match state.registry.prepare_activity(id, &state.context)? {
            ActionRequest::OpenView(request) => PreparedActivity::OpenView(request),
            ActionRequest::Command(invocation) => {
                PreparedActivity::Command(Self::prepared(&state, invocation)?)
            }
        })
    }
    pub fn prepare_status(&self, id: &ItemId) -> Result<PreparedCommand> {
        let state = self.state.borrow();
        Self::prepared(
            &state,
            state.registry.prepare_status_command(id, &state.context)?,
        )
    }
    pub fn prepare_command(
        &self,
        owner: &PluginId,
        id: &CommandId,
        args: serde_json::Value,
    ) -> Result<PreparedCommand> {
        let state = self.state.borrow();
        let registration = state
            .registry
            .registration(owner)
            .context("owner is not registered")?;
        Self::prepared(
            &state,
            state.registry.prepare_command(
                registration,
                id,
                args,
                CommandOrigin::Host,
                &state.context,
            )?,
        )
    }

    fn prepared(state: &State, invocation: CommandInvocation) -> Result<PreparedCommand> {
        let owner = state
            .registry
            .owner_for(invocation.command().as_str())
            .context("command owner missing")?;
        Ok(PreparedCommand {
            invocation,
            epoch: state.epochs[owner].clone(),
        })
    }
    pub fn location(&self, id: &ViewId) -> Result<Location> {
        Ok(self
            .state
            .borrow()
            .registry
            .view(id)
            .context("unknown view")?
            .location)
    }

    fn activate(&mut self, owner: &PluginId) -> Result<()> {
        if self.runtimes.contains_key(owner) {
            return Ok(());
        }
        ensure!(
            self.state.borrow().context.writable(),
            "host is read-only or closing"
        );
        let package = self
            .packages
            .get(owner)
            .context("unregistered development package")?;
        package.snapshot.verify()?;
        self.generation = self
            .generation
            .checked_add(1)
            .context("runtime generation exhausted")?;
        let generation = self.generation;
        let module = host_module(
            Rc::downgrade(&self.state),
            owner.clone(),
            generation,
            package.grants.clone(),
        );
        let policy = Rc::new(
            Policy::new()
                .with_application(owner.as_str())
                .with_host_module(module)?,
        );
        let runtime = ShellRuntime::new_isolated_with_components_and_limits(
            gpui_shell::components::components()?,
            RuntimeLimits::default(),
        )?;
        runtime.set_editor_files(false);
        self.state
            .borrow_mut()
            .active
            .insert(owner.clone(), generation);
        self.runtimes
            .insert(owner.clone(), OwnerRuntime { runtime, policy });
        Ok(())
    }

    /// Only call after a host-validated explicit open/command action.
    pub fn open_view(
        &mut self,
        id: &ViewId,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<(ViewInstanceKey, Entity<ScriptView>)> {
        self.reap_faults();
        let request = {
            let state = self.state.borrow();
            state.registry.prepare_view(id, &state.context)?
        };
        let key = request.key().clone();
        let cached = self.state.borrow().instances.get(&key).cloned();
        if let Some(view) = cached {
            let mut state = self.state.borrow_mut();
            let State {
                registry,
                context,
                instances,
                ..
            } = &mut *state;
            instances.acquire(registry, context, &request, || view.clone())?;
            drop(state);
            // Hidden snapshots are not refreshed on metadata-only updates.
            view.update(cx, |view, cx| view.refresh(cx));
            return Ok((key, view));
        }
        let owner = key.owner.clone();
        self.activate(&owner)?;
        let package = &self.packages[&owner].snapshot;
        let entry = package
            .manifest()
            .entry(id)
            .context("view implementation missing")?;
        let runtime = &self.runtimes[&owner];
        let loaded = runtime.runtime.load_view_with_policy(
            package.root(),
            entry,
            runtime.policy.clone(),
            window,
            cx,
        );
        let view = match loaded {
            Ok(view) => view,
            Err(error) => {
                self.stop_owner(&owner);
                return Err(error);
            }
        };
        let result = {
            let mut state = self.state.borrow_mut();
            let State {
                registry,
                context,
                instances,
                ..
            } = &mut *state;
            instances
                .acquire(registry, context, &request, || view.clone())
                .map(|_| ())
        };
        if let Err(error) = result {
            self.stop_owner(&owner);
            return Err(error.into());
        }
        Ok((key, view))
    }

    pub fn dispatch(
        &mut self,
        command: PreparedCommand,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<PendingCommand> {
        self.reap_faults();
        let invocation = command.invocation;
        let (owner, binding) = {
            let state = self.state.borrow();
            state
                .registry
                .validate_command(&invocation, &state.context)?;
            let owner = state
                .registry
                .owner_for(invocation.command().as_str())
                .context("command owner missing")?
                .clone();
            ensure!(
                state
                    .epochs
                    .get(&owner)
                    .is_some_and(|epoch| Rc::ptr_eq(epoch, &command.epoch)),
                "saved command was invalidated by stop"
            );
            let binding = self.packages[&owner]
                .snapshot
                .manifest()
                .binding(invocation.command())
                .context("command binding missing")?
                .clone();
            (owner, binding)
        };
        let (key, view) = self.open_view(&binding.view, window, cx)?;
        ensure!(
            &key.scope == invocation.scope(),
            "command target scope differs"
        );
        let ticket = {
            let mut state = self.state.borrow_mut();
            let State {
                registry,
                context,
                instances,
                ..
            } = &mut *state;
            instances.begin(registry, context, &key)?
        };
        let runtime = &self.runtimes[&owner].runtime;
        let call = runtime.command_target(&view, cx).and_then(|target| {
            target.dispatch(
                &binding.command,
                host_value(invocation.args().clone()),
                window,
                cx,
            )
        });
        match call {
            Ok(call) => Ok(PendingCommand {
                ticket,
                invocation,
                call,
            }),
            Err(error) => {
                let mut state = self.state.borrow_mut();
                let State {
                    registry,
                    context,
                    instances,
                    ..
                } = &mut *state;
                instances.complete(registry, context, &ticket);
                drop(state);
                self.reap_faults();
                Err(error)
            }
        }
    }

    /// Expired results are discarded; current script failures remain visible to the host.
    pub fn accept(&mut self, command: CompletedCommand, cx: &App) -> Result<Option<HostValue>> {
        let current = {
            let mut state = self.state.borrow_mut();
            if state
                .registry
                .validate_command(&command.invocation, &state.context)
                .is_err()
            {
                false
            } else {
                let State {
                    registry,
                    context,
                    instances,
                    ..
                } = &mut *state;
                instances
                    .complete(registry, context, &command.ticket)
                    .is_some()
            }
        };
        let faults = self.reap_faults();
        if !current {
            return Ok(None);
        }
        if let Some((_, error)) = faults
            .iter()
            .find(|(owner, _)| owner == &command.ticket.key().owner)
        {
            anyhow::bail!("{error}");
        }
        command.reply.accept(cx).map(Some)
    }

    pub fn set_context(&mut self, input: ContextInput, data: HostData, cx: &mut App) -> Result<()> {
        data.validate(&input)?;
        let context = self.tracker.update(input)?.clone();
        if context.version() != self.state.borrow().context.version() {
            self.stop_all();
        }
        let mut state = self.state.borrow_mut();
        state.context = context;
        state.data = data;
        for key in state.instances.visible_keys() {
            if let Some(view) = state.instances.get(&key) {
                view.update(cx, |view, cx| view.refresh(cx));
            }
        }
        Ok(())
    }

    pub fn set_visible(&mut self, visible: &[ViewInstanceKey]) {
        let mut state = self.state.borrow_mut();
        let State {
            registry,
            context,
            instances,
            ..
        } = &mut *state;
        instances.reconcile(registry, context, visible, |_| true);
    }
    pub fn view(&self, key: &ViewInstanceKey) -> Option<Entity<ScriptView>> {
        self.state.borrow().instances.get(key).cloned()
    }
    pub fn runtime_weak(&self, owner: &PluginId) -> Option<Weak<ShellRuntime>> {
        self.runtimes
            .get(owner)
            .map(|vm| Rc::downgrade(&vm.runtime))
    }

    pub fn stop_owner(&mut self, owner: &PluginId) {
        {
            let mut state = self.state.borrow_mut();
            state.active.remove(owner);
            if let Some(epoch) = state.epochs.get_mut(owner) {
                *epoch = Rc::new(());
            }
        }
        if let Some(runtime) = self.runtimes.remove(owner) {
            runtime.runtime.stop();
        }
        self.state.borrow_mut().instances.remove_owner(owner);
    }
    pub fn stop_all(&mut self) {
        let owners: Vec<_> = self.packages.keys().cloned().collect();
        for owner in owners {
            self.stop_owner(&owner);
        }
    }
    pub fn unregister(&mut self, owner: &PluginId) -> Result<()> {
        self.stop_owner(owner);
        let mut state = self.state.borrow_mut();
        let registration = state
            .registry
            .registration(owner)
            .context("owner is not registered")?
            .clone();
        state.registry.unregister(&registration)?;
        state.epochs.remove(owner);
        self.packages.remove(owner);
        Ok(())
    }
    pub fn reap_faults(&mut self) -> Vec<(PluginId, String)> {
        let faults: Vec<_> = self
            .runtimes
            .iter()
            .filter_map(|(owner, vm)| {
                vm.runtime
                    .resource_error()
                    .map(|error| (owner.clone(), error))
            })
            .collect();
        for (owner, _) in &faults {
            self.stop_owner(owner);
        }
        faults
    }
}
impl Drop for DevelopmentHost {
    fn drop(&mut self) {
        self.stop_all();
    }
}

fn host_module(
    state: Weak<RefCell<State>>,
    owner: PluginId,
    generation: u64,
    grants: BTreeSet<Capability>,
) -> HostModule {
    let context_version = state.upgrade().unwrap().borrow().context.version();
    let mut module = HostModule::new("conduit");
    for api in ["getContext", "getWorkspaceMetadata", "getSessionSummary"] {
        let state = state.clone();
        let owner = owner.clone();
        let grants = grants.clone();
        module = module.function(api, move |_| {
            let state = state.upgrade().ok_or_else(|| HostError::new("host closed"))?;
            let state = state.borrow();
            if state.active.get(&owner) != Some(&generation) || state.context.version() != context_version || !state.context.writable() {
                return Err(HostError::new("plugin generation or context is inactive"));
            }
            let workspace = state.data.workspace.as_ref();
            let value = match api {
                "getContext" => serde_json::json!({"pluginId":owner.as_str(), "generation":generation.to_string(), "contextVersion":context_version.to_string(),
                    "workspaceId":workspace.map(|w| w.id.as_str()), "sessionId":state.context.input().session.as_ref().map(|s| s.id.to_string())}),
                "getWorkspaceMetadata" if grants.contains(&Capability::WorkspaceSummary) => serde_json::json!(workspace.map(|w| serde_json::json!({"id":w.id,"name":w.name}))),
                "getSessionSummary" if grants.contains(&Capability::SessionSummary) => serde_json::json!(workspace.map(|w| serde_json::json!({"workspaceId":w.id,"count":w.session_count}))),
                _ => return Err(HostError::new("business permission not granted")),
            };
            Ok(host_value(value))
        });
    }
    module
}

fn host_value(value: serde_json::Value) -> HostValue {
    match value {
        serde_json::Value::Null => HostValue::Null,
        serde_json::Value::Bool(v) => HostValue::Bool(v),
        serde_json::Value::Number(v) => HostValue::Number(v.as_f64().unwrap_or_default()),
        serde_json::Value::String(v) => HostValue::Str(v),
        serde_json::Value::Array(v) => HostValue::Array(v.into_iter().map(host_value).collect()),
        serde_json::Value::Object(v) => {
            HostValue::Object(v.into_iter().map(|(k, v)| (k, host_value(v))).collect())
        }
    }
}
