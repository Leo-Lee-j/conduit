use super::{ActiveRun, ChatRuntime, ConduitApp, Event, Store, Workspace};
use gpui_kit::InputEvent as _;
use gpui_kit::{
    AppContext, Entity, ListOffset, Pixels, Point, ScrollDelta, ScrollWheelEvent, TestAppContext,
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

fn scroll_to(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, item_ix: usize, offset: f32) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.scroll.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(offset),
            });
            app.at_bottom = false;
            cx.notify();
        });
    });
    settle(cx);
}

/// Dispatch multiple packets without allowing a paint between them. Ordinary
/// simulate_event calls redraw between packets and hide the stale-anchor bug.
fn wheel_burst(
    app: &Entity<ConduitApp>,
    cx: &mut VisualTestContext,
    pointer: Point<Pixels>,
    deltas: &[ScrollDelta],
) {
    cx.update(|window, cx| {
        let scroll = app.read(cx).scroll.clone();
        let max = scroll.max_offset_for_scrollbar().y;
        let mut expected = scroll.scroll_px_offset_for_scrollbar().y;
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
            assert_eq!(
                scroll.scroll_px_offset_for_scrollbar().y,
                expected,
                "wheel delta {delta:?}"
            );
        }
    });
    draw(cx);
    let after = position(app, cx);
    settle(cx);
    assert_eq!(position(app, cx), after, "must remain still without input");
}

fn pixels(delta: f32) -> ScrollDelta {
    ScrollDelta::Pixels(point(px(0.), px(delta)))
}

#[gpui_kit::test]
fn chat_continuous_wheel_preserves_direction_and_idle_position(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    for delta in [120., -120., 120., -120.] {
        let start = position(&app, cx);
        for step in 0..80 {
            let before = position(&app, cx);
            let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
            cx.simulate_event(ScrollWheelEvent {
                position: pointer,
                delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                ..Default::default()
            });
            draw(cx);
            let after = position(&app, cx);
            assert!(
                if delta > 0. {
                    after <= before
                } else {
                    after >= before
                },
                "wheel {delta}, step {step} reversed direction: {before:?} -> {after:?}"
            );
            settle(cx);
            assert_eq!(
                position(&app, cx),
                after,
                "wheel {delta}, step {step} moved again without input"
            );
        }
        assert_ne!(
            position(&app, cx),
            start,
            "wheel {delta} must move the chat"
        );
    }
}

#[gpui_kit::test]
fn chat_wheel_burst_applies_each_delta_once(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    scroll_to(&app, cx, 41, 200.);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    wheel_burst(
        &app,
        cx,
        pointer,
        &[
            pixels(20.),
            pixels(20.),
            pixels(-5.),
            pixels(-10.),
            pixels(5.),
        ],
    );
    assert_eq!(position(&app, cx), (41, px(170.)));
}

#[gpui_kit::test]
fn chat_wheel_leaves_bottom_and_clamps_each_packet_at_boundaries(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    let pointer = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
    let before = cx.debug_bounds("message-row-79").unwrap().origin.y;
    wheel_burst(&app, cx, pointer, &[pixels(20.)]);
    assert_eq!(
        cx.debug_bounds("message-row-79").unwrap().origin.y,
        before + px(20.)
    );
    cx.update(|_, cx| assert!(!app.read(cx).at_bottom));
    wheel_burst(
        &app,
        cx,
        pointer,
        &[
            pixels(1_000_000.),
            pixels(20.),
            pixels(-5.),
            pixels(-1_000_000.),
            pixels(-20.),
            pixels(5.),
        ],
    );
    cx.update(|_, cx| assert!(!app.read(cx).at_bottom));
    wheel_burst(&app, cx, pointer, &[pixels(-1_000_000.)]);
    cx.update(|_, cx| assert!(app.read(cx).at_bottom));
}

#[gpui_kit::test]
fn chat_wheel_handles_lines_and_blank_padding(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    scroll_to(&app, cx, 41, 200.);
    let viewport = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds());
    let pointer = point(viewport.center().x, viewport.top() - px(2.));
    wheel_burst(
        &app,
        cx,
        pointer,
        &[
            ScrollDelta::Lines(point(0., 1.)),
            ScrollDelta::Lines(point(0., 1.)),
            ScrollDelta::Lines(point(0., -0.25)),
        ],
    );
    assert_eq!(position(&app, cx), (41, px(165.)));
}

#[gpui_kit::test]
fn chat_wheel_handles_collapsed_groups_and_keeps_tool_scroll_independent(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            for index in [42, 43] {
                app.session_mut(1).unwrap().messages[index] =
                    serde_json::from_value(serde_json::json!({
                        "key": format!("scroll-{index}"), "kind": "Tool", "text": "Read",
                        "detail": "tool output\n".repeat(200), "status": "完成"
                    }))
                    .unwrap();
            }
            app.session_mut(1).unwrap().messages[44].kind = crate::domain::ItemKind::Assistant;
            app.scroll.remeasure();
            app.outline.indexed_len = usize::MAX;
            cx.notify();
        });
    });
    scroll_to(&app, cx, 42, 0.);
    let pointer = cx.debug_bounds("tool-group-header-1-42").unwrap().center();
    wheel_burst(&app, cx, pointer, &[pixels(20.), pixels(20.), pixels(-5.)]);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.expanded_tool_groups.insert((1, "scroll-42".into()));
            app.expanded_tools.insert((1, "scroll-42".into()));
            app.scroll.remeasure_items(42..44);
            cx.notify();
        });
    });
    scroll_to(&app, cx, 42, 0.);
    let inner = cx.update(|_, cx| app.read(cx).tool_scrolls[&(1, "scroll-42".into())].clone());
    assert!(inner.max_offset().y > px(500.));
    let chat_before = position(&app, cx);
    let inner_before = inner.offset().y;
    cx.simulate_event(ScrollWheelEvent {
        position: inner.bounds().center(),
        delta: pixels(-80.),
        ..Default::default()
    });
    settle(cx);
    assert!(inner.offset().y < inner_before);
    assert_eq!(position(&app, cx), chat_before);
    let pointer =
        cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().origin + point(px(2.), px(2.)));
    let inner_before = inner.offset();
    wheel_burst(&app, cx, pointer, &[pixels(-20.), pixels(-20.), pixels(5.)]);
    assert_eq!(inner.offset(), inner_before);
    assert_ne!(position(&app, cx), chat_before);
}

#[gpui_kit::test]
fn chat_wheel_during_stream_preserves_history_and_resumes_follow_at_bottom(
    cx: &mut TestAppContext,
) {
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
    wheel_burst(&app, cx, pointer, &[pixels(-1_000_000.)]);
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
