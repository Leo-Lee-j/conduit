//! Conversation status, workspace label, and panel controls.

use super::*;

#[derive(Clone, Copy)]
enum ToolGroup {
    Tasks,
    Terminal,
}

impl ToolGroup {
    fn id(self) -> &'static str {
        match self {
            Self::Tasks => "task-tools",
            Self::Terminal => "terminal-options",
        }
    }
    fn title(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Tasks => "navigation.tasks",
            Self::Terminal => "navigation.terminal_options",
        })
    }
}

struct ContextTools {
    host: WeakEntity<ConduitApp>,
    popover: WeakEntity<gpui_kit::component::popover::PopoverState>,
    group: ToolGroup,
    availability: Option<(Option<PathBuf>, bool, bool, bool)>,
    _subscription: Subscription,
}
impl Render for ContextTools {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let popover = self.popover.clone();
        let group = self.group;
        host.update(cx, |app, cx| {
            let items = match group {
                ToolGroup::Tasks => vec![
                    (
                        "workflow.title",
                        app.workflow_button(cx, Some(popover.clone())),
                    ),
                    (
                        "automation.title",
                        app.automation_button(cx, Some(popover.clone())),
                    ),
                ],
                ToolGroup::Terminal => vec![
                    (
                        "shortcuts.new_terminal",
                        Button::new("context-new-terminal")
                            .small()
                            .ghost()
                            .icon(FullIconName::Plus)
                            .disabled(
                                !app.writable
                                    || app.closing
                                    || app.updater.preparing()
                                    || app.selected_workspace_path().is_none(),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let _ =
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                this.terminal_command(true, window, cx);
                            })),
                    ),
                    (
                        "environment.title",
                        app.environments_button(cx, Some(self.popover.clone())),
                    ),
                ],
            };
            v_flex()
                .w(px(260.))
                .p_2()
                .gap_1()
                .debug_selector(move || format!("{}-list", group.id()))
                .children(items.into_iter().enumerate().map(|(index, (key, button))| {
                    button
                        .debug_selector(move || format!("{}-{index}", group.id()))
                        .w_full()
                        .justify_start()
                        .label(conduit::i18n::text(key))
                }))
                .into_any_element()
        })
    }
}

impl ConduitApp {
    fn contextual_tools_button(
        &self,
        group: ToolGroup,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let host = cx.weak_entity();
        Popover::new(group.id())
            .anchor(Anchor::BottomRight)
            .trigger(
                Button::new(group.id())
                    .debug_selector(move || group.id().into())
                    .small()
                    .ghost()
                    .icon(match group {
                        ToolGroup::Tasks => FullIconName::ListTodo,
                        ToolGroup::Terminal => FullIconName::ChevronDown,
                    })
                    .when(matches!(group, ToolGroup::Tasks), |button| {
                        button.label(group.title())
                    })
                    .tooltip(group.title())
                    .accessibility_label(group.title())
                    .disabled(self.selected_workspace_path().is_none()),
            )
            .content(move |_, window, cx| {
                let popover = cx.weak_entity();
                window.use_keyed_state((group.id(), 0usize), cx, |_, cx| ContextTools {
                    _subscription: cx.observe(
                        &host.upgrade().expect("host is alive"),
                        |this: &mut ContextTools, host, cx| {
                            let app = host.read(cx);
                            let availability = (
                                app.selected_workspace_path(),
                                app.writable,
                                app.closing,
                                app.updater.preparing(),
                            );
                            if this.availability.as_ref() != Some(&availability) {
                                this.availability = Some(availability);
                                cx.notify();
                            }
                        },
                    ),
                    host: host.clone(),
                    popover,
                    group,
                    availability: None,
                })
            })
    }

    pub(super) fn task_tools_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.contextual_tools_button(ToolGroup::Tasks, cx)
    }

    pub(super) fn terminal_options_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.contextual_tools_button(ToolGroup::Terminal, cx)
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
            .child(self.branch_toolbar_button(cx))
            .child(
                self.github_button(cx, None)
                    .label(conduit::i18n::text_static("github.short_title"))
                    .debug_selector(|| "branch-pr".into()),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_0()
                    .child(self.terminal_button(cx))
                    .child(self.terminal_options_button(cx)),
            )
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
    use gpui_kit::component::WindowExt as _;
    use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};
    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.run_until_parked();
    }
    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"));
        let point = gpui_kit::point(bounds.right() - px(12.), bounds.center().y);
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        draw(cx);
    }

    #[gpui_kit::test]
    fn contextual_menus_fit_the_minimum_window_and_close_with_escape(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "context-tool-bounds");
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
            assert!(cx.debug_bounds("workspace-tools-button").is_none());
            let usage = cx.debug_bounds("navigation-usage").unwrap();
            assert!(usage.right() <= px(width) && usage.bottom() <= px(height));
            for (trigger, list) in [
                ("task-tools", "task-tools-list"),
                ("terminal-options", "terminal-options-list"),
            ] {
                let button = cx.debug_bounds(trigger).unwrap();
                assert!(button.right() <= px(width) && button.bottom() <= px(height));
                click(cx, trigger);
                let menu = cx.debug_bounds(list).unwrap();
                assert!(menu.left() >= px(0.) && menu.top() >= px(0.));
                assert!(menu.right() <= px(width) && menu.bottom() <= px(height));
                cx.simulate_keystrokes("escape");
                draw(cx);
                assert!(cx.debug_bounds(list).is_none());
            }
            for (navigation, selectors) in [
                ("nav-git", &["context-pr", "context-review"][..]),
                ("nav-files", &["context-file-tools"][..]),
            ] {
                click(cx, navigation);
                for selector in selectors {
                    let bounds = cx.debug_bounds(selector).unwrap();
                    assert!(bounds.left() >= px(0.) && bounds.right() <= px(width));
                    assert!(bounds.top() >= px(0.) && bounds.bottom() <= px(height));
                }
            }
            click(cx, "nav-workspaces");
        }
    }

    #[gpui_kit::test]
    fn contextual_entries_open_their_panels_and_dismiss_the_menu(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "context-tool-clicks");
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.startup = None;
                app.writable = false;
                cx.notify();
            })
        });
        cx.simulate_resize(size(px(1000.), px(800.)));
        for (trigger, list, selector) in [
            ("task-tools", "task-tools-list", "task-tools-0"),
            ("task-tools", "task-tools-list", "task-tools-1"),
            (
                "terminal-options",
                "terminal-options-list",
                "terminal-options-1",
            ),
        ] {
            draw(cx);
            click(cx, trigger);
            click(cx, selector);
            assert!(
                cx.debug_bounds(list).is_none(),
                "menu remained open for {selector}"
            );
            cx.update(|window, cx| {
                assert!(
                    window.has_active_dialog(cx),
                    "{selector} did not open its panel"
                );
                window.close_all_dialogs(cx);
            });
            draw(cx);
        }
        for (navigation, selector) in [
            ("nav-git", "context-pr"),
            ("nav-git", "context-review"),
            ("nav-files", "context-file-tools"),
        ] {
            click(cx, navigation);
            click(cx, selector);
            cx.update(|window, cx| {
                if selector == "context-pr" {
                    assert!(!window.has_active_dialog(cx));
                    app.update(cx, |app, cx| {
                        let id = app.github_ui.active.expect("PR document opened");
                        app.close_documents(
                            &[super::files::DocumentTab::PullRequest(id)],
                            window,
                            cx,
                        );
                    });
                } else {
                    assert!(
                        window.has_active_dialog(cx),
                        "{selector} did not open its panel"
                    );
                    window.close_all_dialogs(cx);
                }
            });
            draw(cx);
        }
        click(cx, "navigation-usage");
        cx.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert!(app.read(cx).usage_ui.active.is_some());
        });
    }
}
