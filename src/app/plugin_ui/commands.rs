//! Host-only bindings. A saved UI callback is a request, never an executable closure.
use super::*;
use conduit::plugins::{
    commands::{CommandInvocation, CommandOrigin},
    ids::CommandId,
    registry::Registration,
    state::ViewInstanceKey,
};

type Handler =
    fn(&mut ConduitApp, &CommandInvocation, &mut Window, &mut Context<ConduitApp>) -> bool;

pub(super) struct NativeCommand {
    pub registration: Registration,
    handler: Handler,
}

pub(super) fn bind_core(ui: &mut PluginUi) {
    ui.bind_command(builtins::SHOW_TOOL_DETAILS, |app, _, window, cx| {
        app.open_right_view(&ViewId::new(builtins::TOOL_DETAILS).unwrap(), window, cx)
    });
}

impl PluginUi {
    pub(in crate::app) fn host_command(&self, id: &str) -> Option<CommandInvocation> {
        let id = CommandId::new(id).ok()?;
        if !self.command_bound(&id) {
            return None;
        }
        let owner = self.registry.owner_for(id.as_str())?;
        self.registry
            .prepare_command(
                self.registry.registration(owner)?,
                &id,
                serde_json::Value::Null,
                CommandOrigin::Host,
                &self.snapshot,
            )
            .ok()
    }

    pub(super) fn bind_command(&mut self, id: &str, handler: Handler) {
        let id = CommandId::new(id).expect("native command id");
        if self.registry.command(&id).is_none() {
            return;
        }
        let owner = self.registry.owner_for(id.as_str()).unwrap();
        let registration = self.registry.registration(owner).unwrap().clone();
        self.commands.insert(
            id,
            NativeCommand {
                registration,
                handler,
            },
        );
    }

    pub(super) fn command_bound(&self, id: &CommandId) -> bool {
        self.commands
            .get(id)
            .is_some_and(|binding| self.registry.is_registered(&binding.registration))
    }

    pub(super) fn view_command(
        &self,
        key: &ViewInstanceKey,
        id: &str,
    ) -> Option<CommandInvocation> {
        let view = self.registry.prepare_view(&key.view, &self.snapshot).ok()?;
        if view.key() != key {
            return None;
        }
        let registration = self.registry.registration(&key.owner)?;
        let id = CommandId::new(id).ok()?;
        if !self.command_bound(&id) {
            return None;
        }
        self.registry
            .prepare_command(
                registration,
                &id,
                serde_json::Value::Null,
                CommandOrigin::View,
                &self.snapshot,
            )
            .ok()
    }
}

impl ConduitApp {
    pub(in crate::app) fn dispatch_plugin_command(
        &mut self,
        invocation: &CommandInvocation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_plugin_context();
        if self
            .plugins
            .registry
            .validate_command(invocation, &self.plugins.snapshot)
            .is_err()
        {
            return false;
        }
        let Some(binding) = self.plugins.commands.get(invocation.command()) else {
            return false;
        };
        if !self.plugins.registry.is_registered(&binding.registration) {
            return false;
        }
        (binding.handler)(self, invocation, window, cx)
    }

    pub(super) fn dispatch_activity(
        &mut self,
        request: &ActionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(conduit_shell_desktop)]
        if self.dispatch_shell_activity(request, window, cx) {
            return;
        }
        match request {
            ActionRequest::OpenView(_) => self.dispatch_navigation(request, cx),
            ActionRequest::Command(command) => {
                self.dispatch_plugin_command(command, window, cx);
            }
        }
    }
}
