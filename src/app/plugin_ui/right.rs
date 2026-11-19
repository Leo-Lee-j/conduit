//! Native right-side adapters; the registry owns visibility, identity and selection.
use super::*;
use conduit::plugins::state::{ViewInstanceKey, ViewRequest};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

pub(in crate::app) struct DockLayout {
    pub width: f32,
}
impl Default for DockLayout {
    fn default() -> Self {
        Self { width: 340. }
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

pub(super) fn encourager_renderer() -> RightAdapter {
    RightAdapter {
        render: ConduitApp::render_encourager_view,
        icon: FullIconName::Cat,
        close_id: "close-programmer-encourager",
        with_branches: true,
        activate: |_, _, _| {},
        deactivate: ConduitApp::deactivate_encourager,
    }
}

pub(super) fn core_renderers() -> BTreeMap<String, RightAdapter> {
    [
        (
            builtins::SESSION_DATA.into(),
            RightAdapter {
                render: ConduitApp::render_session_data,
                icon: FullIconName::FileChartColumn,
                close_id: "close-session-data",
                with_branches: false,
                activate: |_, _, _| {},
                deactivate: |_, _, _| {},
            },
        ),
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
        (
            builtins::WORKFLOW.into(),
            RightAdapter {
                render: ConduitApp::render_workflow_view,
                icon: FullIconName::ListTodo,
                close_id: "close-workflow",
                with_branches: true,
                activate: ConduitApp::activate_workflow,
                deactivate: |_, _, _| {},
            },
        ),
        (
            builtins::AUTOMATIONS.into(),
            RightAdapter {
                render: ConduitApp::render_automation_view,
                icon: FullIconName::Clock,
                close_id: "close-automations",
                with_branches: true,
                activate: ConduitApp::activate_automations,
                deactivate: |_, _, _| {},
            },
        ),
        (builtins::ENCOURAGER_VIEW.into(), encourager_renderer()),
    ]
    .into_iter()
    .collect()
}

/// Selection and eligibility used by the independently cached sidebar header.
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
            // Its old title must never survive under a retained header.
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
            .map(|v| super::contribution_title(v.id.as_str(), &v.title))
            .unwrap_or_else(|| conduit::i18n::text_static("plugin_ui.right.sidebar").into())
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
                title: super::contribution_title(v.id.as_str(), &v.title),
                icon: self
                    .right_renderers
                    .get(&v.renderer)
                    .map_or(FullIconName::PanelRight, |a| a.icon),
                selected: self.right_is(v.id.as_str()),
                request: (self.right_renderers.contains_key(&v.renderer)
                    && self.renderer_bound(&v.renderer))
                .then(|| self.registry.prepare_view(&v.id, &self.snapshot).ok())
                .flatten(),
            })
            .collect()
    }
}

impl ConduitApp {
    pub(in crate::app) fn session_data_available(&self) -> bool {
        !self.settings_ui.open
            && self.active_file.is_none()
            && self.active_diff.is_none()
            && self.usage_ui.active.is_none()
            && self.github_ui.active.is_none()
    }

    pub(in crate::app) fn right_view_visible(&self) -> bool {
        self.plugins.right_visible(false)
            && (self.plugins.right().as_str() != builtins::SESSION_DATA
                || self.session_data_available())
    }

    pub(in crate::app) fn render_right_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        self.sync_plugin_context();
        let (notifications, mut choices): (Vec<_>, Vec<_>) = self
            .plugins
            .right_choices(false)
            .into_iter()
            .filter(|choice| {
                choice.id.as_str() != builtins::SESSION_DATA || self.session_data_available()
            })
            .partition(|choice| choice.id.as_str() == builtins::NOTIFICATIONS);
        choices.sort_by_key(|choice| match choice.id.as_str() {
            builtins::SESSION_DATA => 0,
            builtins::WORKFLOW => 1,
            builtins::AUTOMATIONS => 2,
            builtins::TOOL_DETAILS => 3,
            _ => 4,
        });
        v_flex()
            .id("right-navigation-rail")
            .test_support()
            .debug_selector(|| "right-navigation-rail".into())
            .w(px(38.))
            .h_full()
            .flex_shrink_0()
            .bg(chrome_background(palette().sidebar_bg))
            .border_l_1()
            .border_color(rgb(palette().border))
            .child(
                v_flex()
                    .id("right-navigation-views")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap_1()
                    .items_center()
                    .overflow_y_scroll()
                    .children(
                        choices
                            .into_iter()
                            .map(|choice| self.right_navigation_button(choice, cx)),
                    ),
            )
            .when(!notifications.is_empty(), |rail| {
                rail.child(
                    h_flex()
                        .w_full()
                        .h(px(STATUS_BAR_HEIGHT))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .children(
                            notifications
                                .into_iter()
                                .map(|choice| self.right_navigation_button(choice, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    fn right_navigation_button(&self, choice: RightChoice, cx: &mut Context<Self>) -> AnyElement {
        let selector = match choice.id.as_str() {
            builtins::WORKFLOW => "project-workflow".to_owned(),
            builtins::AUTOMATIONS => "project-automations".to_owned(),
            builtins::NOTIFICATIONS => "notification-bell".to_owned(),
            _ => format!("right-choice-{}", choice.id),
        };
        let notification = choice.id.as_str() == builtins::NOTIFICATIONS;
        let unread = if notification {
            self.notifications.center.unread()
        } else {
            0
        };
        let tooltip = if notification {
            conduit::i18n::format(
                "notifications.notifications_unread",
                &[("unread", unread.to_string())],
            )
        } else {
            choice.title.clone()
        };
        let label = if notification {
            conduit::i18n::format(
                "notifications.notifications_unread_a9fb74",
                &[("unread", unread.to_string())],
            )
        } else {
            choice.title
        };
        let request = choice.request;
        let button = Button::new(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .ghost()
            .small()
            .compact()
            .focus_ring(true)
            .w(px(super::RAIL_BUTTON_SIZE))
            .h(px(if notification {
                28.
            } else {
                super::RAIL_BUTTON_SIZE
            }))
            .flex_shrink_0()
            .icon(choice.icon)
            .selected(choice.selected)
            .tooltip(tooltip)
            .accessibility_label(label)
            .disabled(request.is_none())
            .cursor_pointer()
            .on_click(cx.listener(move |app, _, window, cx| {
                if let Some(request) = &request {
                    app.toggle_right_view(request, window, cx);
                }
            }));
        super::navigation_button_row()
            .h(px(if notification { 28. } else { TAB_BAR_HEIGHT }))
            .child(button)
            .when(unread > 0, |row| {
                row.child(
                    h_flex()
                        .debug_selector(|| "right-notifications-badge".into())
                        .absolute()
                        .top_0()
                        .right(px(2.))
                        .min_w(px(12.))
                        .h(px(12.))
                        .px(px(2.))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(palette().text))
                        .text_color(rgb(palette().on_text))
                        .text_size(ui_text_size(9.))
                        .line_height(relative(1.))
                        .child(if unread > 99 {
                            "99+".to_owned()
                        } else {
                            unread.to_string()
                        }),
                )
            })
            .into_any_element()
    }

    fn toggle_right_view(
        &mut self,
        request: &ViewRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_plugin_context();
        if self.plugins.right_open() && self.plugins.right_key() == Some(request.key()) {
            if self
                .plugins
                .registry
                .validate_view(request, &self.plugins.snapshot)
                .is_err()
            {
                return false;
            }
            self.close_right_view(window, cx);
            true
        } else {
            self.dispatch_right_view(request, window, cx)
        }
    }

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
        cx.notify();
    }
}
