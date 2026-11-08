//! Opt-in native example: metadata only, with separate state for each view/scope.
use super::summary::WorkspaceSummary;
use super::*;
use conduit::plugins::{
    commands::CommandInvocation, context::ScopeKey, contributions::StatusValue,
    instances::RequestTicket, state::ViewInstanceKey,
};
use std::sync::Arc;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

pub(super) struct OverviewInstance {
    view: Entity<OverviewView>,
    desired: Arc<WorkspaceSummary>,
}
struct OverviewView {
    app: WeakEntity<ConduitApp>,
    data: Arc<WorkspaceSummary>,
    scroll: ScrollHandle,
    summary: bool,
    command: Option<CommandInvocation>,
    controls: (u64, u64, bool, u32, u32, u32),
    #[cfg(test)]
    renders: usize,
}

pub(super) fn bind(ui: &mut PluginUi) {
    let owner = conduit::plugins::ids::PluginId::new(builtins::EXAMPLE).unwrap();
    let Some(_) = ui.registry.registration(&owner) else {
        return;
    };
    ui.bind_renderer(builtins::EXAMPLE_INDEX);
    ui.bind_renderer(builtins::EXAMPLE_SUMMARY);
    ui.renderers.insert(
        builtins::EXAMPLE_INDEX.into(),
        NavigationAdapter {
            render: ConduitApp::render_overview_navigation,
            activate: |_, _| {},
            element_id: builtins::EXAMPLE_ACTIVITY,
            task_badge: false,
            refresh_on_notify: false,
        },
    );
    ui.right_renderers.insert(
        builtins::EXAMPLE_SUMMARY.into(),
        right::RightAdapter {
            render: |app, key, _, cx| {
                right::RightBody::Content(app.render_overview(key, true, cx)).into()
            },
            icon: FullIconName::LayoutDashboard,
            close_id: "close-workspace-overview",
            with_branches: true,
            activate: |_, _, _| {},
            deactivate: |_, _, _| {},
        },
    );
    ui.bind_command(builtins::EXAMPLE_COMMAND, |app, _, window, cx| {
        app.open_right_view(&ViewId::new(builtins::EXAMPLE_SUMMARY).unwrap(), window, cx)
    });
    ui.bind_status(builtins::EXAMPLE_STATUS, None, |app, scope| {
        let workspace = app.overview_workspace(scope);
        StatusValue {
            text: conduit::i18n::format(
                "plugin_ui.example.overview_sessions",
                &[(
                    "arg0",
                    format!("{}", workspace.map_or(0, |w| w.sessions.len())),
                )],
            ),
            tooltip: workspace.map(|w| {
                conduit::i18n::format(
                    "plugin_ui.example.view_workspace_summary",
                    &[("arg0", display_path(&w.path).to_string())],
                )
            }),
        }
    });
}

impl ConduitApp {
    fn overview_workspace(&self, scope: &ScopeKey) -> Option<&Workspace> {
        let ScopeKey::Workspace(key) = scope else {
            return None;
        };
        self.store.workspaces.iter().find(|w| w.path == key.path())
    }

    fn render_overview_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(key) = self.plugins.state.navigation_key().cloned() else {
            return div()
                .child(conduit::i18n::text_static(
                    "plugin_ui.example.select_a_workspace",
                ))
                .into_any_element();
        };
        self.render_overview(&key, false, cx)
    }

    fn render_overview(
        &mut self,
        key: &ViewInstanceKey,
        summary: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(data) = self.overview_data(key) else {
            return div()
                .p_3()
                .child(conduit::i18n::text_static(
                    "plugin_ui.example.workspace_closed",
                ))
                .into_any_element();
        };
        let Ok(request) = self
            .plugins
            .registry
            .prepare_view(&key.view, &self.plugins.snapshot)
        else {
            return div().into_any_element();
        };
        if request.key() != key {
            return div().into_any_element();
        }
        let command = self.plugins.view_command(key, builtins::EXAMPLE_COMMAND);
        let controls = self.overview_controls(command.is_some());
        let app = cx.weak_entity();
        let scroll = ScrollHandle::new();
        let data = Arc::new(data);
        let instance = self.plugins.instances.acquire(
            &self.plugins.registry,
            &self.plugins.snapshot,
            &request,
            || OverviewInstance {
                desired: data.clone(),
                view: cx.new(|_| OverviewView {
                    app,
                    data,
                    scroll: scroll.clone(),
                    summary,
                    command,
                    controls,
                    #[cfg(test)]
                    renders: 0,
                }),
            },
        );
        let Ok(instance) = instance else {
            return div()
                .child(conduit::i18n::text_static(
                    "plugin_ui.example.view_cache_full",
                ))
                .into_any_element();
        };
        let view = instance.view.clone();
        self.refresh_overview(key, cx);
        view.cached(StyleRefinement::default().size_full().min_h_0().min_w_0())
            .into_any_element()
    }

    fn overview_data(&self, key: &ViewInstanceKey) -> Option<WorkspaceSummary> {
        self.overview_workspace(&key.scope)
            .map(|workspace| WorkspaceSummary::capture(workspace, &self.session_control))
    }
    fn overview_controls(&self, enabled: bool) -> (u64, u64, bool, u32, u32, u32) {
        (
            self.plugins.snapshot.version(),
            self.plugins.registry.revision(),
            enabled,
            palette().text,
            palette().muted,
            palette().border,
        )
    }
    pub(in crate::app) fn refresh_plugin_views(&mut self, cx: &mut Context<Self>) {
        #[cfg(conduit_shell_desktop)]
        self.refresh_shell_context(cx);
        // Only mounted instances observe metadata; hidden ones refresh on re-open.
        for key in self.plugins.instances.visible_keys() {
            self.refresh_overview(&key, cx);
        }
    }
    fn refresh_overview(&mut self, key: &ViewInstanceKey, cx: &mut Context<Self>) {
        let Some(data) = self.overview_data(key) else {
            return;
        };
        let command = self.plugins.view_command(key, builtins::EXAMPLE_COMMAND);
        let controls = self.overview_controls(command.is_some());
        let pending = self.plugins.instances.has_pending(key);
        let Some(instance) = self.plugins.instances.get_mut(key) else {
            return;
        };
        instance.view.update(cx, |view, cx| {
            if view.controls != controls {
                view.controls = controls;
                view.command = command;
                cx.notify();
            }
        });
        if *instance.desired == data && (pending || *instance.view.read(cx).data == data) {
            return;
        }
        let data = Arc::new(data);
        instance.desired = data.clone();
        let Ok(ticket) =
            self.plugins
                .instances
                .begin(&self.plugins.registry, &self.plugins.snapshot, key)
        else {
            return;
        };
        let request = ticket.clone();
        // Coalesce metadata bursts. Owned Task cancellation drops the timer and its
        // bounded snapshot; the callback still validates if cancellation races it.
        let task = cx.spawn(async move |app, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            if request.is_cancelled() {
                return;
            }
            let _ = app.update(cx, |app, cx| {
                app.complete_overview(&request, data, cx);
            });
        });
        self.plugins.instances.attach(&ticket, task);
    }
    fn complete_overview(
        &mut self,
        ticket: &RequestTicket,
        data: Arc<WorkspaceSummary>,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_plugin_context();
        // Metadata may change before the next notification/render. Never publish
        // an older snapshot just because its route version still matches.
        if self.overview_data(ticket.key()).as_ref() != Some(data.as_ref()) {
            if self.plugins.instances.is_visible(ticket.key()) {
                self.refresh_overview(ticket.key(), cx);
            }
            return false;
        }
        let Some(instance) =
            self.plugins
                .instances
                .complete(&self.plugins.registry, &self.plugins.snapshot, ticket)
        else {
            return false;
        };
        instance.view.update(cx, |view, cx| {
            if view.data != data {
                view.data = data;
                cx.notify();
            }
        });
        true
    }
}

impl Render for OverviewView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let WorkspaceSummary {
            name,
            path,
            total,
            archived,
            running,
            sessions,
        } = self.data.as_ref();
        let summary = self.summary;
        let location = if summary { "summary" } else { "index" };
        let mut content = v_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .p_3()
            .gap_3()
            .child(
                div()
                    .text_size(ui_text_size(14.))
                    .font_semibold()
                    .child(if summary {
                        conduit::i18n::text_static("plugin_ui.example.workspace_summary")
                    } else {
                        conduit::i18n::text_static("plugin_ui.example.workspace_overview")
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_semibold()
                    .child(name.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(gpui_kit::base::SelectableText::new(
                        SharedString::from(format!("overview-{location}-path")),
                        path.clone(),
                    )),
            )
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::format(
                        "plugin_ui.example.sessions_archived_running",
                        &[
                            ("total", format!("{}", total)),
                            ("archived", format!("{}", archived)),
                            ("running", format!("{}", running)),
                        ],
                    )),
            );
        if !summary {
            let request = self.command.clone();
            content = content.child(
                Button::new("overview-show-summary")
                    .debug_selector(|| "overview-show-summary".into())
                    .small()
                    .label(conduit::i18n::text_static(
                        "plugin_ui.example.view_summary_on_the_right",
                    ))
                    .disabled(request.is_none())
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if let Some(request) = &request {
                            let _ = view.app.update(cx, |app, cx| {
                                app.dispatch_plugin_command(request, window, cx);
                            });
                        }
                    })),
            );
        }
        if sessions.is_empty() {
            content = content.child(div().text_color(rgb(palette().muted)).child(
                conduit::i18n::text_static("plugin_ui.example.no_sessions_in_this_workspace_yet"),
            ));
        }
        for super::summary::SessionSummary {
            id,
            title,
            provider,
            archived,
        } in sessions
        {
            content = content.child(
                v_flex()
                    .min_w_0()
                    .flex_shrink_0()
                    .gap_1()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .child(title.clone()),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "{provider} · #{id}{}",
                                if *archived {
                                    conduit::i18n::text_static("plugin_ui.example.archived")
                                } else {
                                    ""
                                }
                            )),
                    ),
            );
        }
        if *total > super::summary::SESSION_LIMIT {
            content = content.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(
                        "plugin_ui.example.showing_the_first_100_sessions",
                    )),
            );
        }
        div()
            .id(SharedString::from(format!("overview-{location}-{path}")))
            .test_support()
            .role(Role::Group)
            .aria_label(if summary {
                conduit::i18n::text_static("plugin_ui.example.workspace_summary")
            } else {
                conduit::i18n::text_static("plugin_ui.example.workspace_overview")
            })
            .debug_selector(move || format!("workspace-overview-{location}"))
            .relative()
            .h_full()
            .flex_1()
            .min_h_0()
            .w_full()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(content)
            .vertical_scrollbar(&self.scroll)
            .into_any_element()
    }
}
