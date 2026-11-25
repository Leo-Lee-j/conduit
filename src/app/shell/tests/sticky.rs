use super::{fixture, pixels, position, settle};
use crate::app::{ConduitApp, shell::TimelineSource};
use gpui_kit::{
    Entity, ListOffset, Modifiers, MouseButton, ScrollWheelEvent, TestAppContext,
    VisualTestContext, px, size,
};

fn seek(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, index: usize, offset: f32) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.at_bottom = false;
            app.pending_message_jump = None;
            app.scroll.scroll_to(ListOffset {
                item_ix: app.chat_rows.row(index).unwrap(),
                offset_in_item: px(offset),
            });
            cx.notify();
        })
    });
    settle(cx);
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let center = cx.debug_bounds(selector).expect("pinned prompt").center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    settle(cx);
}

#[gpui_kit::test]
fn sticky_prompt_follows_the_reading_turn_and_click_reveals_original(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    seek(&app, cx, 0, 0.);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_none());
    let viewport = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds());

    seek(&app, cx, 1, 24.);
    let pinned = cx.debug_bounds("sticky-prompt-1-scroll-0").unwrap();
    assert!(
        pinned.bottom() <= viewport.top(),
        "prompt must not cover transcript"
    );
    assert_eq!(
        cx.update(|_, cx| app.read(cx).scroll.viewport_bounds()),
        viewport
    );

    // A new question entering the bottom does not replace the reading context.
    let height = cx.update(|_, cx| {
        let app = app.read(cx);
        app.scroll
            .bounds_for_item(app.chat_rows.row(1).unwrap())
            .unwrap()
            .size
            .height
    });
    seek(
        &app,
        cx,
        1,
        f32::from((height - viewport.size.height / 2.).max(px(0.))),
    );
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_some());
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-2").is_none());

    seek(&app, cx, 2, 8.);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_none());
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-2").is_some());
    seek(&app, cx, 1, 24.);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_some());
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-2").is_none());

    click(cx, "sticky-prompt-1-scroll-0");
    assert_eq!(position(&app, cx), (0, px(0.)));
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_none());
    assert_eq!(
        cx.update(|_, cx| app.read(cx).scroll.viewport_bounds()),
        viewport
    );
}

#[gpui_kit::test]
fn sticky_prompt_accepts_wheel_and_keeps_long_prompts_compact(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages[0].text = "A long multiline prompt.\n".repeat(300);
            cx.notify();
        })
    });
    seek(&app, cx, 0, 20.);
    let pinned = cx.debug_bounds("sticky-prompt-1-scroll-0").unwrap();
    assert!(pinned.size.height <= px(40.));
    let before = position(&app, cx);
    cx.simulate_event(ScrollWheelEvent {
        position: pinned.center(),
        delta: pixels(-100.),
        ..Default::default()
    });
    settle(cx);
    let after = position(&app, cx);
    assert!(after.0 > before.0 || after.1 > before.1);
    assert_eq!(
        cx.debug_bounds("sticky-prompt-1-scroll-0")
            .unwrap()
            .size
            .height,
        pinned.size.height
    );
    click(cx, "sticky-prompt-1-scroll-0");
    assert_eq!(position(&app, cx), (0, px(0.)));
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_none());
}

#[gpui_kit::test]
fn sticky_prompt_uses_visible_rows_at_tail_and_during_streaming(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let session = app.session_mut(1).unwrap();
            session.messages.truncate(2);
            session.messages[1].text = "Short response.".into();
            TimelineSource::Main.jump_to_end(app, cx);
        })
    });
    settle(cx);
    assert!(
        cx.debug_bounds("sticky-prompt-1-scroll-0").is_none(),
        "short conversation already shows the prompt"
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages[1].text = "Streaming paragraph.\n\n".repeat(100);
            cx.notify();
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.at_bottom);
        assert_eq!(
            app.scroll.logical_scroll_top().item_ix,
            app.scroll.item_count()
        );
    });

    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let session = app.session_mut(1).unwrap();
            for (key, kind, text) in [
                ("next", "User", "Next question"),
                ("reply", "Assistant", "Short reply"),
            ] {
                session.messages.push(
                    serde_json::from_value(serde_json::json!({
                        "key": key, "kind": kind, "text": text
                    }))
                    .unwrap(),
                );
            }
            cx.notify();
        })
    });
    settle(cx);
    assert!(
        cx.debug_bounds("sticky-prompt-1-scroll-0").is_some(),
        "tail viewport still begins in the previous answer"
    );
    assert!(cx.debug_bounds("sticky-prompt-1-next").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages[3]
                .text
                .push_str(&"\n\nMore streamed content.".repeat(100));
            cx.notify();
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("sticky-prompt-1-next").is_some());
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-0").is_none());
    cx.update(|_, cx| assert!(app.read(cx).at_bottom));
}

#[gpui_kit::test]
fn sticky_prompts_navigate_split_panes_independently(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    cx.simulate_resize(size(px(1800.), px(1000.)));
    let source = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut second = app.session(1).unwrap().clone();
            second.id = 2;
            app.store.workspaces[0].sessions.push(second);
            app.show_chat_beside(2, window, cx);
            app.reference_chat_view(2).unwrap().read(cx).source.clone()
        })
    });
    settle(cx);
    seek(&app, cx, 21, 20.);
    let TimelineSource::Session(state) = &source else {
        unreachable!()
    };
    cx.update(|_, cx| {
        app.update(cx, |_, cx| {
            let mut state = state.borrow_mut();
            state.at_bottom = false;
            state.pending_message_jump = None;
            state.scroll.scroll_to(ListOffset {
                item_ix: state.rows.row(41).unwrap(),
                offset_in_item: px(20.),
            });
            cx.notify();
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-20").is_some());
    assert!(cx.debug_bounds("sticky-prompt-2-scroll-40").is_some());
    let left = position(&app, cx);
    click(cx, "sticky-prompt-2-scroll-40");
    assert_eq!(position(&app, cx), left);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-20").is_some());
    assert!(cx.debug_bounds("sticky-prompt-2-scroll-40").is_none());
    assert_eq!(state.borrow().pending_message_jump, Some(40));
    cx.update(|_, cx| assert_eq!(app.read(cx).selected_session, Some(1)));

    // Narrow windows show one pane at a time. Restoring the main pane must
    // restore its prompt and anchor, rather than displaying the other prompt.
    cx.simulate_resize(size(px(850.), px(700.)));
    settle(cx);
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-20").is_none());
    click(cx, "show-main-pane");
    assert!(cx.debug_bounds("sticky-prompt-1-scroll-20").is_some());
    assert_eq!(position(&app, cx), left);
}

#[gpui_kit::test]
fn sticky_prompt_handles_consecutive_users_and_clears_on_session_switch(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx);
    let item = |key: &str, kind: &str, text: String| {
        serde_json::from_value(serde_json::json!({"key": key, "kind": kind, "text": text})).unwrap()
    };
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages = vec![
                item(
                    "intro",
                    "Assistant",
                    "Introduction before any user message.\n\n".repeat(100),
                ),
                item("first", "User", "First question".into()),
                item("empty", "User", String::new()),
                item(
                    "answer",
                    "Assistant",
                    "Answer to consecutive prompts.\n\n".repeat(100),
                ),
            ];
            let mut second = app.session(1).unwrap().clone();
            second.id = 2;
            second.messages = vec![
                item("other", "User", "Another conversation".into()),
                item("short", "Assistant", "Short answer".into()),
            ];
            app.store.workspaces[0].sessions.push(second);
            cx.notify();
        })
    });
    settle(cx);
    seek(&app, cx, 0, 40.);
    assert!(
        cx.debug_bounds("sticky-prompt-1-first").is_none(),
        "no future question above the first user message"
    );
    assert!(cx.debug_bounds("sticky-prompt-1-empty").is_none());
    seek(&app, cx, 1, 8.);
    assert!(cx.debug_bounds("sticky-prompt-1-first").is_some());
    seek(&app, cx, 3, 40.);
    assert!(cx.debug_bounds("sticky-prompt-1-first").is_none());
    assert!(
        cx.debug_bounds("sticky-prompt-1-empty").is_some(),
        "empty prompt still has a navigation target"
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_session = Some(2);
            TimelineSource::Main.jump_to_end(app, cx);
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("sticky-prompt-1-empty").is_none());
    assert!(
        cx.debug_bounds("sticky-prompt-2-other").is_none(),
        "short replacement session shows its original prompt"
    );
}
