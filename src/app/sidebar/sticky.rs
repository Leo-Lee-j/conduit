//! Paint after the virtual list has applied its live scroll offset. This keeps
//! the header in sync without rebuilding the sidebar on every wheel event.
use super::*;

impl ConduitApp {
    pub(super) fn sticky_workspace_header(&self, cx: &Context<Self>) -> AnyElement {
        let app = cx.entity();
        div()
            .id("sticky-workspace-header")
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(ROW_HEIGHT))
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let (mut header, offset) = app.update(cx, |app, cx| {
                            let (row, offset) = app.sidebar.sticky_workspace()?;
                            let nested = matches!(&row, cache::Row::Workspace { path, .. }
                                if app.sidebar.projects.parent(path).is_some());
                            let element = match &row {
                                cache::Row::Project { .. } => app.sidebar_project(&row, cx),
                                cache::Row::Workspace { path, count, last } => {
                                    let index = app.sidebar.workspace_index(path)?;
                                    app.sidebar_workspace(index, *count, *last, cx)
                                }
                                _ => return None,
                            };
                            Some((
                                div()
                                    .id("sticky-workspace-row")
                                    .debug_selector(|| "sticky-workspace-row".into())
                                    .relative()
                                    .w_full()
                                    .h(px(ROW_HEIGHT))
                                    .px_2()
                                    // Opaque even when the sidebar uses a translucent wallpaper.
                                    .bg(rgb(palette().sidebar_bg))
                                    .on_scroll_wheel(cx.listener(
                                        |app, event: &ScrollWheelEvent, window, cx| {
                                            let scroll = &app.sidebar.scroll;
                                            let delta =
                                                event.delta.pixel_delta(window.line_height());
                                            let y = (scroll.offset().y + delta.y)
                                                .clamp(-scroll.max_offset().y, px(0.));
                                            scroll.set_offset(point(px(0.), y));
                                            app.sidebar_view.update(cx, |_, cx| cx.notify());
                                            cx.stop_propagation();
                                        },
                                    ))
                                    .child(element)
                                    .when(nested, |row| {
                                        row.child(
                                            div()
                                                .absolute()
                                                .left(px(46.))
                                                .top_0()
                                                .bottom_0()
                                                .w(px(1.))
                                                .bg(rgb(palette().border)),
                                        )
                                    })
                                    .into_any_element(),
                                offset,
                            ))
                        })?;
                        header.layout_as_root(
                            size(
                                AvailableSpace::Definite(bounds.size.width),
                                AvailableSpace::Definite(px(ROW_HEIGHT)),
                            ),
                            window,
                            cx,
                        );
                        header.prepaint_at(bounds.origin + point(px(0.), px(offset)), window, cx);
                        Some(header)
                    },
                    |_, header, window, cx| {
                        if let Some(mut header) = header {
                            header.paint(window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use gpui_kit::{
        Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext,
        point, px, size,
    };

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let center = cx.debug_bounds(selector).expect(selector).center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        draw(cx);
    }

    fn assert_workspace_focus_ring(cx: &mut VisualTestContext, visible: bool) {
        let bounds = cx.debug_bounds("workspace-item-0").unwrap();
        cx.update(|window, _| {
            let outlines: Vec<_> = window
                .painted_quads()
                .into_iter()
                .filter(|quad| {
                    quad.border_color == gpui_kit::rgb(crate::app::palette().focus).into()
                        && quad.border_widths.top > px(0.).scale(window.scale_factor())
                })
                .collect();
            let ring = outlines
                .iter()
                .find(|quad| quad.bounds == bounds.scale(window.scale_factor()));
            assert_eq!(
                ring.is_some(),
                visible,
                "workspace keyboard focus outline at {bounds:?}; painted {outlines:?}"
            );
            if let Some(ring) = ring {
                assert!(ring.corner_radii.top_left > px(0.).scale(window.scale_factor()));
            }
        });
    }

    #[gpui_kit::test]
    fn sticky_header_accepts_scroll_and_collapse_without_changing_chat(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "sticky-workspace");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.startup = None;
                app.writable = false;
                app.store.ui.auto_sort_workspaces = false;
                app.store.ui.auto_collapse_inactive_workspaces = false;
                app.store.ui.sessions_per_workspace = 3;
                app.store.workspaces[0].sessions.extend((3..=101).map(|id| {
                    let mut session = crate::app::tests::session();
                    session.id = id;
                    session.created_at = id;
                    session.pinned = id == 101;
                    session.title = format!("Conversation {id}");
                    session
                }));
                app.input.update(cx, |input, cx| {
                    input.set_value("unfinished draft", window, cx)
                });
                cx.notify();
            })
        });
        for viewport in [size(px(850.), px(500.)), size(px(1100.), px(700.))] {
            cx.simulate_resize(viewport);
            draw(cx);
            click(cx, "workspace-more-0");
            let controls = cx
                .debug_bounds("workspace-controls")
                .expect("fixed workspace controls");
            let position = cx.debug_bounds("sidebar").unwrap().center();
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(-1600.))),
                ..Default::default()
            });
            draw(cx);
            assert_eq!(cx.debug_bounds("workspace-controls"), Some(controls));
            let sticky = cx
                .debug_bounds("sticky-workspace-row")
                .expect("sticky title is visible");
            let before = cx.update(|_, cx| app.read(cx).sidebar.scroll.offset().y);
            cx.simulate_event(ScrollWheelEvent {
                position: sticky.center(),
                delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
                ..Default::default()
            });
            draw(cx);
            cx.update(|_, cx| assert!(app.read(cx).sidebar.scroll.offset().y < before));
            let collapse = cx.debug_bounds("workspace-show-fewer-0").unwrap();
            assert_eq!(collapse.size, size(px(24.), px(24.)));
            click(cx, "workspace-show-fewer-0");
            cx.update(|window, cx| {
                let app = app.read(cx);
                assert!(!app.expanded_workspaces.contains(&0));
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.input.read(cx).value(), "unfinished draft");
                assert!(app.sidebar.focus.as_ref().unwrap().is_focused(window));
            });
            assert_workspace_focus_ring(cx, false);
            let workspace_bounds = cx.debug_bounds("workspace-item-0").unwrap();
            cx.simulate_keystrokes("left");
            draw(cx);
            assert_workspace_focus_ring(cx, true);
            assert_eq!(
                cx.debug_bounds("workspace-item-0").unwrap(),
                workspace_bounds
            );
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.workspace_collapsed(&app.store.workspaces[0].path));
                assert_eq!(app.selected_session, Some(1));
            });
            cx.simulate_keystrokes("right");
            draw(cx);
            assert_workspace_focus_ring(cx, true);
            assert_eq!(
                cx.debug_bounds("workspace-item-0").unwrap(),
                workspace_bounds
            );
            click(cx, "workspace-more-0");
            assert_workspace_focus_ring(cx, false);
            cx.simulate_keystrokes("down");
            draw(cx);
            let focused = cx
                .debug_bounds("session-row-101")
                .expect("focused session is rendered");
            let sticky = cx
                .debug_bounds("sticky-workspace-row")
                .expect("parent remains visible");
            assert!(
                focused.top() >= sticky.bottom(),
                "sticky parent must not cover keyboard focus"
            );
            cx.simulate_keystrokes("left");
            draw(cx);
            click(cx, "workspace-show-fewer-0");
            assert_workspace_focus_ring(cx, false);

            // The footer uses the same compact action and restores the workspace
            // from the very bottom, both by pointer and keyboard activation.
            for keyboard in [false, true] {
                click(cx, "workspace-more-0");
                // The fixture has a second workspace and one session after this footer.
                cx.simulate_keystrokes("end up up");
                draw(cx);
                cx.update(|_, cx| {
                    let app = app.read(cx);
                    assert!(matches!(app.sidebar.keyboard_row.as_ref(),
                        Some(super::cache::Row::More { path, expanded: true, .. })
                            if *path == app.store.workspaces[0].path));
                });
                let footer = cx.debug_bounds("workspace-more-0").unwrap();
                assert_eq!(footer.size.height, px(24.));
                assert!(
                    footer.size.width > px(28.),
                    "compact action includes its label"
                );
                assert!(footer.right() <= cx.debug_bounds("sidebar").unwrap().right());
                if keyboard {
                    cx.simulate_keystrokes("enter");
                    draw(cx);
                } else {
                    click(cx, "workspace-more-0");
                }
                cx.update(|window, cx| {
                    let app = app.read(cx);
                    assert!(app.sidebar.focus.as_ref().unwrap().is_focused(window));
                    assert!(matches!(
                        app.sidebar.keyboard_row.as_ref(),
                        Some(super::cache::Row::Workspace { .. })
                    ));
                    assert_eq!(window.last_input_was_keyboard(), keyboard);
                });
                assert_workspace_focus_ring(cx, keyboard);
                assert_eq!(
                    cx.debug_bounds("workspace-item-0").unwrap(),
                    workspace_bounds
                );
                cx.update(|_, cx| {
                    let app = app.read(cx);
                    assert!(!app.expanded_workspaces.contains(&0));
                    assert_eq!(app.selected_session, Some(1));
                    assert_eq!(app.input.read(cx).value(), "unfinished draft");
                });
            }
        }
    }
}
