//! Conversation status, workspace label, and panel controls.

use super::*;

impl ConduitApp {
    pub(super) fn render_toolbar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_plugin_context();
        // The toolbar belongs to the conversation; workspace navigation and
        // the independent right dock use their full height.
        let toolbar_hint: Option<String> = if self.closing {
            Some(conduit::i18n::text_static("toolbar.cleaning_up_processes").into())
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
            .child(self.terminal_button(cx))
            .child(
                self.environments_button(cx, None)
                    .debug_selector(|| "toolbar-environments".into()),
            );
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
    fn right_sidebar_entries_share_the_rail_and_toggle_without_dialogs(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "task-sidebar-bounds");
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
            assert!(cx.debug_bounds("task-tools").is_none());
            assert!(cx.debug_bounds("toggle-sidebar").is_none());
            let rail = cx.debug_bounds("right-navigation-rail").unwrap();
            assert_eq!(rail.right(), px(width));
            for (trigger, content, close) in [
                (
                    "project-workflow",
                    "project-workflow-panel",
                    "close-workflow",
                ),
                (
                    "project-automations",
                    "automation-panel",
                    "close-automations",
                ),
                (
                    "right-choice-conduit.core.tool-details",
                    "detail-empty",
                    "close-details",
                ),
                (
                    "notification-bell",
                    "notification-panel",
                    "close-notifications",
                ),
            ] {
                let button = cx.debug_bounds(trigger).unwrap();
                assert!(button.left() >= rail.left() && button.right() <= rail.right());
                click(cx, trigger);
                assert!(cx.debug_bounds("right-sidebar-tabs").is_none());
                assert!(cx.debug_bounds("right-sidebar-title").is_some());
                let panel = cx.debug_bounds("detail-panel").unwrap();
                let content = cx.debug_bounds(content).unwrap();
                assert!(panel.left() >= px(0.) && panel.right() <= rail.left());
                assert!(content.left() >= panel.left() && content.right() <= panel.right());
                assert!(content.top() >= panel.top() && content.bottom() <= panel.bottom());
                cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
                click(cx, close);
                assert!(cx.debug_bounds("detail-panel").is_none());
                cx.update(|_, cx| {
                    assert!(
                        app.read(cx)
                            .plugins
                            .right_choices(false)
                            .iter()
                            .all(|choice| !choice.selected)
                    );
                });
                click(cx, trigger);
                click(cx, trigger);
                assert!(cx.debug_bounds("detail-panel").is_none());
            }
        }
    }

    #[gpui_kit::test]
    fn direct_entries_fit_the_minimum_window(cx: &mut TestAppContext) {
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
            let connections = cx.debug_bounds("toolbar-environments").unwrap();
            assert!(connections.left() >= px(0.) && connections.right() <= px(width));
            assert!(connections.top() >= px(0.) && connections.bottom() <= px(height));
            let usage = cx.debug_bounds("navigation-usage").unwrap();
            assert!(usage.right() <= px(width) && usage.bottom() <= px(height));
            assert!(cx.debug_bounds("terminal-options").is_none());
            let workspace = cx.debug_bounds("toolbar-workspace").unwrap();
            let branch = cx.debug_bounds("git-branch-trigger").unwrap();
            assert!(cx.debug_bounds("branch-pr").is_none());
            let toolbar = cx.debug_bounds("conversation-toolbar").unwrap();
            assert!(workspace.right() <= branch.left());
            assert!(branch.right() <= connections.left());
            for bounds in [workspace, branch] {
                assert!(bounds.top() >= toolbar.top() && bounds.bottom() <= toolbar.bottom());
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
    fn direct_entries_open_their_panels(cx: &mut TestAppContext) {
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
        draw(cx);
        click(cx, "toolbar-environments");
        cx.update(|window, cx| {
            assert!(window.has_active_dialog(cx));
            window.close_all_dialogs(cx);
        });
        draw(cx);
        click(cx, "git-branch-trigger");
        cx.update(|_, cx| assert!(app.read(cx).branch_navigation_open()));
        click(cx, "git-branch-trigger");
        cx.update(|_, cx| assert!(!app.read(cx).branch_navigation_open()));
        click(cx, "nav-git");
        click(cx, "context-pr");
        cx.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            let app = app.read(cx);
            assert!(app.git_navigation_open());
            assert!(app.github_ui.sidebar_active);
            assert!(app.github_ui.active.is_none());
        });
        draw(cx);
        for (navigation, selector) in [
            ("nav-git", "context-pr"),
            ("nav-git", "context-review"),
            ("nav-files", "context-file-tools"),
        ] {
            click(cx, navigation);
            if selector == "context-review" {
                click(cx, "git-commit-tab");
            }
            click(cx, selector);
            cx.update(|window, cx| {
                if selector == "context-pr" {
                    assert!(!window.has_active_dialog(cx));
                    assert!(app.read(cx).github_ui.sidebar_active);
                    assert!(app.read(cx).github_ui.active.is_none());
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
