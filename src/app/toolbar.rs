//! Conversation status, workspace label, and panel controls.

use super::*;

struct WorkspaceTools {
    host: WeakEntity<ConduitApp>,
}
impl Render for WorkspaceTools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        host.update(cx, |app, cx| {
            let items = [
                ("workflow.title", app.workflow_button(cx).into_any_element()),
                (
                    "review.inbox",
                    app.review_inbox_button(cx).into_any_element(),
                ),
                ("github.title", app.github_button(cx).into_any_element()),
                ("usage.title", app.usage_button(cx).into_any_element()),
                (
                    "automation.title",
                    app.automation_button(cx).into_any_element(),
                ),
                (
                    "artifact.title",
                    app.project_files_button(cx).into_any_element(),
                ),
                (
                    "environment.title",
                    app.environments_button(cx).into_any_element(),
                ),
            ];
            v_flex()
                .debug_selector(|| "workspace-tool-list".into())
                .p_2()
                .gap_1()
                .children(items.into_iter().map(|(key, button)| {
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(button)
                        .child(conduit::i18n::text(key))
                }))
                .into_any_element()
        })
    }
}

impl ConduitApp {
    fn workspace_tools_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let host = cx.weak_entity();
        Popover::new("workspace-tools")
            .anchor(Anchor::BottomRight)
            .trigger(
                Button::new("workspace-tools-button")
                    .debug_selector(|| "workspace-tools-button".into())
                    .small()
                    .compact()
                    .ghost()
                    .icon(FullIconName::PanelsTopLeft)
                    .tooltip(conduit::i18n::text_static("workflow.tools"))
                    .accessibility_label(conduit::i18n::text_static("workflow.tools"))
                    .disabled(self.selected_workspace_path().is_none()),
            )
            .content(move |_, _, cx| cx.new(|_| WorkspaceTools { host: host.clone() }))
    }
    pub(super) fn render_toolbar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_plugin_context();
        let active = self
            .selected_session
            .is_some_and(|id| self.session_control.runs.contains_key(&id));
        let status = self
            .selected_session
            .and_then(|id| self.session_control.runs.get(&id))
            .map(|run| run.status.clone());
        // The toolbar belongs to the conversation; workspace navigation and
        // the independent right dock use their full height.
        let toolbar_hint: Option<String> = if self.closing {
            Some(conduit::i18n::text_static("toolbar.cleaning_up_processes").into())
        } else if active {
            Some(
                status
                    .clone()
                    .unwrap_or_else(|| conduit::i18n::text_static("stream_events.running").into()),
            )
        } else {
            None
        };
        let naming_status: Option<Result<(), String>> = self.selected_session.and_then(|id| {
            if let Some(error) = self.session_control.naming_errors.get(&id) {
                Some(Err(error.clone()))
            } else if !self.closing {
                self.session_control
                    .namings
                    .get(&id)
                    .map(|naming| naming.error.clone().map_or(Ok(()), Err))
            } else {
                None
            }
        });
        let has_hints = toolbar_hint.is_some() || naming_status.is_some();
        let hints = h_flex()
            .id("toolbar-hints")
            .test_support()
            .debug_selector(|| "toolbar-hints".into())
            .min_w_0()
            .max_w(relative(0.35))
            .items_center()
            .gap_2()
            .when_some(toolbar_hint, |bar, hint| {
                bar.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(hint),
                )
            })
            .when_some(naming_status, |bar, status| {
                let failed = status.is_err();
                let text = match &status {
                    Ok(()) => {
                        conduit::i18n::text_static("toolbar.renaming_automatically").to_string()
                    }
                    Err(reason) => conduit::i18n::format(
                        "toolbar.automatic_renaming_failed",
                        &[("reason", reason.to_string())],
                    ),
                };
                let tooltip = SharedString::from(text.clone());
                bar.child(
                    h_flex()
                        .id("session-naming-status")
                        .min_w_0()
                        .max_w(px(360.))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                                .build(window, cx)
                        })
                        .items_center()
                        .gap_1()
                        .when(failed, |row| {
                            row.child(
                                Icon::new(IconName::TriangleAlert)
                                    .small()
                                    .text_color(rgb(palette().danger_text)),
                            )
                        })
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(if failed {
                                    palette().danger_text
                                } else {
                                    palette().muted
                                }))
                                .child(text),
                        )
                        .when(!failed, |row| {
                            row.child(Spinner::new().small().color(rgb(palette().loading).into()))
                        }),
                )
            });
        let controls = h_flex()
            .id("toolbar-controls")
            .test_support()
            .debug_selector(|| "toolbar-controls".into())
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .when(self.store.ui.performance_monitor, |bar| {
                bar.child({
                    let hover_monitor = self.performance.clone();
                    let pinned_monitor = self.performance.clone();
                    div()
                        .id("performance-tooltip")
                        .flex_shrink_0()
                        .when(!self.performance_open, |trigger| {
                            trigger.tooltip(move |_, _| hover_monitor.clone().into())
                        })
                        .child(
                            Popover::new("performance-popover")
                                .anchor(Anchor::BottomRight)
                                .appearance(false)
                                .open(self.performance_open)
                                .on_open_change(cx.listener(|this, open, _, cx| {
                                    this.performance_open = *open;
                                    cx.notify();
                                }))
                                .trigger(
                                    Button::new("performance-monitor")
                                        .ghost()
                                        .small()
                                        .compact()
                                        .icon(gpui_kit::assets::IconName::Activity)
                                        .accessibility_label(conduit::i18n::text_static(
                                            "performance_view.performance_monitor",
                                        ))
                                        .cursor_pointer(),
                                )
                                .content(move |_, _, _| pinned_monitor.clone()),
                        )
                })
            })
            .child(self.workspace_tools_button(cx))
            .child(self.terminal_button(cx))
            .child(self.notification_bell(cx));
        let statuses = self.render_status_contributions(window, cx);
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let statuses = chat::profiling::host_element(
            statuses,
            ["status_layout", "status_prepaint", "status_paint"],
        );
        let toolbar = h_flex()
            .id("conversation-toolbar")
            .test_support()
            .debug_selector(|| "conversation-toolbar".into())
            .h(px(34.))
            .min_w_0()
            .flex_shrink_0()
            .px_2()
            .gap_2()
            .items_center()
            .bg(chrome_background(palette().canvas))
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(statuses)
            .when(has_hints, |bar| bar.child(hints))
            .child(controls);
        toolbar.into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};
    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.run_until_parked();
    }
    #[gpui_kit::test]
    fn workspace_tools_render_inside_the_minimum_window_without_borrowing_the_host_twice(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "workspace-tools");
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.startup = None;
                app.writable = false;
                cx.notify();
            })
        });
        for (width, height) in [(850., 500.), (1200., 800.)] {
            cx.simulate_resize(size(px(width), px(height)));
            draw(cx);
            let button = cx.debug_bounds("workspace-tools-button").unwrap();
            assert!(button.right() <= px(width) && button.bottom() <= px(height));
            cx.simulate_mouse_down(button.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(button.center(), MouseButton::Left, Modifiers::default());
            draw(cx);
            let menu = cx.debug_bounds("workspace-tool-list").unwrap();
            assert!(menu.left() >= px(0.) && menu.top() >= px(0.));
            assert!(menu.right() <= px(width) && menu.bottom() <= px(height));
            cx.simulate_keystrokes("escape");
            draw(cx);
            assert!(cx.debug_bounds("workspace-tool-list").is_none());
        }
    }
}
