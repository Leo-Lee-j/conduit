use super::{ActiveRun, ChatRuntime, ConduitApp, Event, Store, Workspace};
use gpui_kit::InputEvent as _;
use gpui_kit::{
    AppContext, Entity, Pixels, Point, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext, point, px, size,
};
use std::time::Duration;

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.background_executor
            .advance_clock(Duration::from_millis(100));
        draw(cx);
    }
}

fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = crate::app::tests::session();
            session.messages = (0..80)
                .map(|index| {
                    let (kind, text) = if index % 2 == 0 {
                        ("User", format!("检查第 {index} 段聊天内容。"))
                    } else {
                        (
                            "Assistant",
                            format!(
                                "## 回复 {index}\n\n{}\n```rust\nfn main() {{\n    println!(\"scroll\");\n}}\n```\n\n结束段落。",
                                "连续上下滚动时，长消息应该跟随滚轮，停止操作后保持阅读位置。\n\n"
                                    .repeat(4 + index % 9)
                            ),
                        )
                    };
                    serde_json::from_value(serde_json::json!({
                        "key": format!("scroll-{index}"), "kind": kind, "text": text
                    }))
                    .unwrap()
                })
                .collect();
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir().join("conduit-scroll-regression"),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.startup = None;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(1200.), px(800.)));
    settle(cx);
    (app, cx)
}

fn position(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) -> (usize, Pixels) {
    cx.update(|_, cx| {
        let top = app.read(cx).scroll.logical_scroll_top();
        (top.item_ix, top.offset_in_item)
    })
}

/// Dispatch multiple packets without allowing a paint between them. Ordinary
/// simulate_event calls redraw between packets and hide the stale-anchor bug.
fn wheel_burst(
    app: &Entity<ConduitApp>,
    cx: &mut VisualTestContext,
    pointer: Point<Pixels>,
    deltas: &[ScrollDelta],
) {
    let expected = cx.update(|window, cx| {
        let scroll = app.read(cx).scroll.clone();
        let max = scroll.max_offset_for_scrollbar().y;
        let mut expected = scroll
            .scroll_px_offset_for_scrollbar()
            .y
            .clamp(-max, px(0.));
        for &delta in deltas {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta,
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            expected = (expected + delta.pixel_delta(px(20.)).y).clamp(-max, px(0.));
        }
        expected
    });
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).scroll.scroll_px_offset_for_scrollbar().y,
            expected
        )
    });
    let after = position(app, cx);
    settle(cx);
    assert_eq!(position(app, cx), after, "must remain still without input");
}

fn pixels(delta: f32) -> ScrollDelta {
    ScrollDelta::Pixels(point(px(0.), px(delta)))
}

#[gpui_kit::test]
fn chat_wheel_at_bottom_keeps_viewport_and_latest_button_stable(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    let viewport = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds());
    let bottom = position(&app, cx);
    let composer = cx.debug_bounds("composer").unwrap();
    assert!(cx.debug_bounds("scroll-to-latest").is_none());
    for delta in [
        pixels(-1.),
        pixels(-20.),
        pixels(-120.),
        ScrollDelta::Lines(point(0., -3.)),
        pixels(0.),
        ScrollDelta::Pixels(point(px(120.), px(0.))),
    ] {
        cx.simulate_event(ScrollWheelEvent {
            position: viewport.center(),
            delta,
            ..Default::default()
        });
        // Check the first painted frame, before polling can hide a transient
        // latest-message button and restore the transcript's original height.
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                app.scroll.viewport_bounds(),
                viewport,
                "overscroll resized the transcript"
            );
            assert!(app.at_bottom);
            assert!(!app.timeline.read(cx).wheel_pending(app));
        });
        assert_eq!(position(&app, cx), bottom);
        assert_eq!(cx.debug_bounds("composer").unwrap(), composer);
        assert!(cx.debug_bounds("scroll-to-latest").is_none());
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        draw(cx);
        cx.update(|_, cx| assert_eq!(app.read(cx).scroll.viewport_bounds(), viewport));
    }
    // Excess downward packets must not build a debt that swallows the first
    // upward packet, including when the entire gesture arrives before redraw.
    cx.update(|window, cx| {
        for delta in std::iter::repeat_n(-120., 20).chain([60.]) {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: viewport.center(),
                    delta: pixels(delta),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    settle(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(
            !app.at_bottom,
            "the first upward packet must release bottom following"
        );
        assert!(app.scroll.logical_scroll_top().item_ix < app.scroll.item_count());
    });
    assert!(cx.debug_bounds("scroll-to-latest").is_some());
}

#[gpui_kit::test]
fn chat_wheel_through_unmeasured_history_preserves_both_directions(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-2", cx)));
    settle(cx);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    for _ in 0..3 {
        let before = position(&app, cx);
        cx.update(|window, cx| {
            let scroll = app.read(cx).scroll.clone();
            assert_eq!(
                scroll.is_scrolled_to_end(),
                None,
                "history is still virtualized"
            );
            // Run beyond the measured overdraw before a frame can measure the
            // next rows, then reverse direction while that layout is pending.
            for delta in std::iter::repeat_n(-120., 12).chain([30., 30., -60.]) {
                window.dispatch_event(
                    ScrollWheelEvent {
                        position: pointer,
                        delta: pixels(delta),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            assert!(!app.read(cx).at_bottom);
        });
        settle(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.poll_streams(window, cx));
            let app = app.read(cx);
            assert!(
                !app.at_bottom,
                "reading history must not enable tail following"
            );
            assert!(app.scroll.logical_scroll_top().item_ix < app.scroll.item_count() / 2);
            assert!(app.scroll.logical_scroll_top().item_ix >= before.0);
            assert_eq!(
                app.outline.active,
                app.outline.active_for_item(
                    app.chat_rows
                        .message(app.scroll.logical_scroll_top().item_ix)
                )
            );
        });
    }
    // Start at a cold section with history above it that has never been measured.
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-60", cx)));
    settle(cx);
    let before_up = position(&app, cx);
    cx.update(|window, cx| {
        for delta in std::iter::repeat_n(120., 12).chain([-30., -30., 60.]) {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta: pixels(delta),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    settle(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        let top = app.scroll.logical_scroll_top();
        assert!(
            top.offset_in_item >= px(0.),
            "upward movement must be resolved by layout: {top:?}"
        );
        assert!(
            (before_up.0.saturating_sub(8)..=before_up.0).contains(&top.item_ix),
            "upward wheel skipped unseen history: {before_up:?} -> {top:?}"
        );
        assert!(!app.at_bottom);
    });
    let after_up = position(&app, cx);
    // Reversing after the layout must return to the same message and offset.
    cx.update(|window, cx| {
        for _ in 0..12 {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta: pixels(-120.),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    settle(cx);
    assert_eq!(
        position(&app, cx),
        before_up,
        "up/down round trip from {after_up:?}"
    );
    // Reversing before a layout must cancel pending movement too.
    cx.update(|window, cx| {
        for delta in std::iter::repeat_n(120., 12).chain(std::iter::repeat_n(-120., 12)) {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta: pixels(delta),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    settle(cx);
    assert_eq!(position(&app, cx), before_up);
}

#[gpui_kit::test]
fn chat_wheel_crosses_async_markdown_without_skipping_it(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            // Cross the parser's asynchronous threshold, including numbered code
            // blocks whose extension changes the rendered height.
            app.store.ui.code_line_numbers = true;
            for index in [1, 39, 59] {
                let item = &mut app.session_mut(1).unwrap().messages[index];
                item.text = format!(
                    "## Long reply\n\n{}\n```rust\nfn main() {{}}\n```",
                    "A paragraph that must remain readable in both scroll directions.\n\n"
                        .repeat(100)
                );
                assert!(item.text.len() > 4096);
            }
            app.timeline_session = None;
            cx.notify();
        })
    });
    settle(cx);
    for index in [2, 40, 60] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.jump_to_message(1, &format!("scroll-{index}"), cx);
            })
        });
        settle(cx);
        let origin = position(&app, cx);
        assert_eq!(origin, (index, px(0.)));
        let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: pixels(120.),
            ..Default::default()
        });
        settle(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            let top = app.scroll.logical_scroll_top();
            assert_eq!(
                top.item_ix,
                index - 1,
                "must enter the preceding long reply"
            );
            let height = app.scroll.bounds_for_item(index - 1).unwrap().size.height;
            assert_eq!(top.offset_in_item, height - px(120.));
            assert!(!app.at_bottom);
        });
        cx.simulate_event(ScrollWheelEvent {
            position: pointer,
            delta: pixels(-120.),
            ..Default::default()
        });
        settle(cx);
        assert_eq!(
            position(&app, cx),
            origin,
            "round trip across long Markdown"
        );
    }
    // Cross a cold long reply downwards too. Its initial empty parse must not
    // let the gesture run through to later messages.
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-38", cx)));
    settle(cx);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    cx.update(|window, cx| {
        for _ in 0..10 {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta: pixels(-120.),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    settle(cx);
    let down = position(&app, cx);
    assert_eq!(down.0, 39);
    assert!(down.1 > px(0.));
    settle(cx);
    assert_eq!(
        position(&app, cx),
        down,
        "parsing must settle without drifting"
    );
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(1200.),
        ..Default::default()
    });
    settle(cx);
    assert_eq!(position(&app, cx), (38, px(0.)));
    // Start from a long reply that has fallen out of the keyed layout cache,
    // reverse the gesture while parsing is pending, then navigate elsewhere.
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-40", cx)));
    settle(cx);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(120.),
        ..Default::default()
    });
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(-120.),
        ..Default::default()
    });
    settle(cx);
    assert_eq!(position(&app, cx), (40, px(0.)));
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(120.),
        ..Default::default()
    });
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-20", cx)));
    settle(cx);
    assert_eq!(position(&app, cx), (20, px(0.)));
}

#[gpui_kit::test]
fn chat_wheel_reverses_before_layout_without_enabling_tail_follow(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.scroll = gpui_kit::ListState::new(0, gpui_kit::ListAlignment::Bottom, px(240.))
                .measure_all();
            let owner = cx.weak_entity();
            app.timeline = cx.new(|_| super::ChatTimeline::new(owner, app.scroll.clone()));
            app.timeline_session = None;
            cx.notify();
        })
    });
    settle(cx);
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-40", cx)));
    settle(cx);
    let origin = position(&app, cx);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    cx.update(|window, cx| {
        for delta in [-1_000_000., 1_000_000.] {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta: pixels(delta),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            app.update(cx, |app, cx| app.poll_streams(window, cx));
            assert!(
                !app.read(cx).at_bottom,
                "pending distance is not a bottom anchor"
            );
        }
    });
    settle(cx);
    assert_eq!(position(&app, cx), origin);
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(1_000_000.),
        ..Default::default()
    });
    settle(cx);
    assert_eq!(position(&app, cx), (0, px(0.)));
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(-120.),
        ..Default::default()
    });
    settle(cx);
    assert_ne!(
        position(&app, cx),
        (0, px(0.)),
        "the start clamp must release"
    );
}

#[gpui_kit::test]
fn chat_wheel_during_stream_preserves_history_and_resumes_follow_at_bottom(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    let (runtime, sender) = ChatRuntime::test_channel();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    app.store.workspaces[0].path.clone(),
                    "scroll".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            cx.notify();
        });
    });
    settle(cx);
    // Navigate to history, then use the wheel to release the one-time jump.
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "scroll-40", cx)));
    settle(cx);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    wheel_burst(&app, cx, pointer, &[pixels(-20.), pixels(-20.), pixels(5.)]);
    cx.update(|_, cx| {
        assert!(app.read(cx).pending_message_jump.is_none());
        assert!(!app.read(cx).at_bottom);
    });
    let before = position(&app, cx);
    // Exercise both in-place growth and new segments that remeasure prior rows.
    for key in ["79", "80", "80", "81"] {
        sender
            .send(Event::Delta {
                key: key.into(),
                text: "\n\n新增的流式回复。".repeat(20),
            })
            .unwrap();
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        settle(cx);
        assert_eq!(position(&app, cx), before);
        cx.update(|_, cx| assert!(!app.read(cx).at_bottom));
    }
    cx.simulate_event(ScrollWheelEvent {
        position: pointer,
        delta: pixels(-1_000_000.),
        ..Default::default()
    });
    settle(cx);
    let previous_end = position(&app, cx);
    for key in ["81", "82", "82"] {
        sender
            .send(Event::Delta {
                key: key.into(),
                text: "\n\n在底部继续跟随回复。".repeat(20),
            })
            .unwrap();
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        settle(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(app.at_bottom);
            assert_eq!(
                app.scroll.logical_scroll_top().item_ix,
                app.scroll.item_count()
            );
        });
    }
    assert!(position(&app, cx) > previous_end);
    // A fresh upward wheel must also disengage following after new text arrives.
    wheel_burst(&app, cx, pointer, &[pixels(60.)]);
    let before = position(&app, cx);
    sender
        .send(Event::Delta {
            key: "82".into(),
            text: "\n\n停留在阅读位置。".repeat(10),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    settle(cx);
    assert_eq!(position(&app, cx), before);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.at_bottom);
        assert_eq!(app.session(1).unwrap().messages.len(), 83);
    });
}

#[cfg(conduit_shell_desktop)]
fn measured(
    cx: &mut VisualTestContext,
    action: impl FnOnce(&mut VisualTestContext),
) -> serde_json::Value {
    crate::app::chat::profiling::begin();
    action(cx);
    draw(cx);
    crate::app::chat::profiling::finish()
}
