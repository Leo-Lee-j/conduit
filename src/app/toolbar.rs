//! Conversation status, workspace label, and panel controls.

use super::*;

impl ConduitApp {
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
            Some("正在清理进程…".into())
        } else if active {
            Some(status.clone().unwrap_or_else(|| "正在执行…".into()))
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
                    Ok(()) => "自动重命名中…".to_string(),
                    Err(reason) => format!("自动重命名失败：{reason}"),
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
                                        .accessibility_label("性能检测")
                                        .cursor_pointer(),
                                )
                                .content(move |_, _, _| pinned_monitor.clone()),
                        )
                })
            })
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
            .bg(rgb(palette().canvas))
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(statuses)
            .when(has_hints, |bar| bar.child(hints))
            .child(controls);
        toolbar.into_any_element()
    }
}
