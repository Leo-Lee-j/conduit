//! Native right-side adapters; the registry owns visibility, identity and selection.
use super::*;
use conduit::plugins::state::{ViewInstanceKey, ViewRequest};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

pub(in crate::app) struct DockLayout {
    pub width: f32,
    pub drag: Option<(f32, f32)>,
}
impl Default for DockLayout {
    fn default() -> Self {
        Self {
            width: 340.,
            drag: None,
        }
    }
}

pub(in crate::app) enum RightBody {
    Content(AnyElement),
    Loading,
    Empty {
        title: &'static str,
        description: &'static str,
    },
    Failed(String),
}
pub(in crate::app) struct RightView {
    pub body: RightBody,
    pub actions: Vec<AnyElement>,
}
pub(in crate::app) struct RightChoice {
    pub id: ViewId,
    pub title: String,
    pub icon: FullIconName,
    pub selected: bool,
    pub request: Option<ViewRequest>,
}
impl From<RightBody> for RightView {
    fn from(body: RightBody) -> Self {
        Self {
            body,
            actions: vec![],
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::app) struct RightAdapter {
    pub render:
        fn(&mut ConduitApp, &ViewInstanceKey, &Window, &mut Context<ConduitApp>) -> RightView,
    pub icon: FullIconName,
    pub close_id: &'static str,
    pub with_branches: bool,
    pub activate: fn(&mut ConduitApp, &mut Window, &mut Context<ConduitApp>),
    pub deactivate: fn(&mut ConduitApp, &mut Window, &mut Context<ConduitApp>),
}

pub(super) fn core_renderers() -> BTreeMap<String, RightAdapter> {
    [
        (
            builtins::TOOL_DETAILS.into(),
            RightAdapter {
                render: ConduitApp::render_tool_details,
                icon: FullIconName::Wrench,
                close_id: "close-details",
                with_branches: false,
                activate: |_, _, _| {},
                deactivate: |_, _, _| {},
            },
        ),
        (
            builtins::NOTIFICATIONS.into(),
            RightAdapter {
                render: ConduitApp::render_notification_view,
                icon: FullIconName::Bell,
                close_id: "close-notifications",
                with_branches: true,
                activate: ConduitApp::activate_notifications,
                deactivate: ConduitApp::deactivate_notifications,
            },
        ),
    ]
    .into_iter()
    .collect()
}

/// Selection and eligibility used by the independently cached dock tab strip.
pub(in crate::app) struct RightHeaderRevision {
    registration: Option<conduit::plugins::registry::Registration>,
    registry: u64,
    context: conduit::plugins::context::ContextInput,
    context_version: u64,
    selected: ViewId,
}

impl PluginUi {
    pub(in crate::app) fn right_header_revision(&self) -> RightHeaderRevision {
        RightHeaderRevision {
            registration: self
                .registry
                .owner_for(self.right().as_str())
                .and_then(|owner| self.registry.registration(owner))
                .cloned(),
            registry: self.registry.revision(),
            context: self.snapshot.input().clone(),
            context_version: self.snapshot.version(),
            selected: self.right().clone(),
        }
    }

    pub(in crate::app) fn right_header_matches(
        &self,
        previous: Option<&RightHeaderRevision>,
    ) -> bool {
        previous.is_some_and(|previous| {
            // A replacement registry can have the same numeric generation.
            // Its old requests must never survive under a retained tab strip.
            previous
                .registration
                .as_ref()
                .is_some_and(|handle| self.registry.is_registered(handle))
                && previous.registry == self.registry.revision()
                && &previous.context == self.snapshot.input()
                && previous.context_version == self.snapshot.version()
                && &previous.selected == self.right()
        })
    }

    pub(in crate::app) fn right(&self) -> &ViewId {
        self.state.right()
    }
    pub(in crate::app) fn right_open(&self) -> bool {
        self.state.right_open()
    }
    pub(in crate::app) fn right_is(&self, id: &str) -> bool {
        self.right_open() && self.right().as_str() == id
    }
    pub(in crate::app) fn right_adapter(&self) -> Option<RightAdapter> {
        self.registry
            .view(self.right())
            .filter(|v| self.renderer_bound(&v.renderer))
            .and_then(|v| self.right_renderers.get(&v.renderer))
            .copied()
    }
    pub(in crate::app) fn right_visible(&self, branches: bool) -> bool {
        self.right_open() && (!branches || self.right_adapter().is_none_or(|a| a.with_branches))
    }
    pub(in crate::app) fn session_key(&self) -> Option<SessionKey> {
        self.snapshot.input().session.clone()
    }
    pub(in crate::app) fn right_key(&self) -> Option<&ViewInstanceKey> {
        self.state.right_key()
    }
    pub(in crate::app) fn right_title(&self) -> String {
        self.registry
            .view(self.right())
            .map(|v| v.title.clone())
            .unwrap_or_else(|| "侧边栏".into())
    }
    pub(in crate::app) fn right_choices(&self, branches: bool) -> Vec<RightChoice> {
        self.registry
            .views()
            .filter(|v| {
                v.location == Location::Right
                    && v.when.matches(&self.snapshot)
                    && self.snapshot.scope(v.scope).is_some()
                    && (!branches
                        || self
                            .right_renderers
                            .get(&v.renderer)
                            .is_none_or(|a| a.with_branches))
            })
            .map(|v| RightChoice {
                id: v.id.clone(),
                title: v.title.clone(),
                icon: self
                    .right_renderers
                    .get(&v.renderer)
                    .map_or(FullIconName::PanelRight, |a| a.icon),
                selected: &v.id == self.right(),
                request: (self.right_renderers.contains_key(&v.renderer)
                    && self.renderer_bound(&v.renderer))
                .then(|| self.registry.prepare_view(&v.id, &self.snapshot).ok())
                .flatten(),
            })
            .collect()
    }
}

impl ConduitApp {
    pub(in crate::app) fn open_right_view(
        &mut self,
        id: &ViewId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_plugin_context();
        match self
            .plugins
            .registry
            .prepare_view(id, &self.plugins.snapshot)
        {
            Ok(request) => self.dispatch_right_view(&request, window, cx),
            Err(_) => false,
        }
    }

    pub(in crate::app) fn dispatch_right_view(
        &mut self,
        request: &ViewRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_plugin_context();
        let Ok(descriptor) = self
            .plugins
            .registry
            .validate_view(request, &self.plugins.snapshot)
        else {
            return false;
        };
        if descriptor.location != Location::Right {
            return false;
        }
        if !self.plugins.renderer_bound(&descriptor.renderer) {
            return false;
        }
        let Some(adapter) = self
            .plugins
            .right_renderers
            .get(&descriptor.renderer)
            .copied()
        else {
            return false;
        };
        if self.branch_navigation_open() && !adapter.with_branches {
            return false;
        }
        let changed = !self.plugins.right_open() || self.plugins.right_key() != Some(request.key());
        if changed
            && self.plugins.right_open()
            && let Some(previous) = self.plugins.right_adapter()
        {
            (previous.deactivate)(self, window, cx);
        }
        if self
            .plugins
            .state
            .open(&self.plugins.registry, request, &self.plugins.snapshot)
            .is_err()
        {
            return false;
        }
        if changed {
            (adapter.activate)(self, window, cx);
        }
        self.plugins.reconcile_instance_visibility();
        cx.notify();
        true
    }

    pub(in crate::app) fn close_right_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.plugins.right_open()
            && let Some(adapter) = self.plugins.right_adapter()
        {
            (adapter.deactivate)(self, window, cx);
        }
        self.plugins.state.close_right();
        self.plugins.reconcile_instance_visibility();
        self.plugins.dock.drag = None;
        cx.notify();
    }
}
