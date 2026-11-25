use super::{draw, fixture, pixels, position, settle};
use crate::app::shell::TimelineSource;
use gpui_kit::{
    InputEvent as _, ListOffset, Modifiers, MouseButton, ScrollWheelEvent, TestAppContext, px, size,
};

#[gpui_kit::test]
fn split_chat_scroll_and_stream_updates_preserve_the_other_pane(cx: &mut TestAppContext) {
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
    let TimelineSource::Session(state) = &source else {
        unreachable!()
    };
    let scroll = state.borrow().scroll.clone();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.jump_to_message(1, "scroll-20", cx);
            source.jump_to_outline_message(2, "scroll-20", app, cx);
        })
    });
    settle(cx);
    let left = position(&app, cx);
    let right = scroll.logical_scroll_top();
    let pointer = scroll.viewport_bounds().center();
    cx.update(|window, cx| {
        for delta in [-120., -120., -120., 30., -60.] {
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
    let after = scroll.logical_scroll_top();
    assert!(after.item_ix > right.item_ix || after.offset_in_item > right.offset_in_item);
    assert!(!state.borrow().at_bottom);
    assert_eq!(
        position(&app, cx),
        left,
        "right wheel moved the left transcript"
    );
    settle(cx);
    assert_eq!(scroll.logical_scroll_top().item_ix, after.item_ix);
    assert_eq!(
        scroll.logical_scroll_top().offset_in_item,
        after.offset_in_item
    );

    // In-place streaming changes have the same row count. They still invalidate
    // right layout, without dragging either pane away from the reading anchor.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(2)
                .unwrap()
                .messages
                .last_mut()
                .unwrap()
                .text
                .push_str(&"\n\nStreaming paragraph updated at the bottom.".repeat(16));
            cx.notify();
        })
    });
    settle(cx);
    assert_eq!(scroll.logical_scroll_top().item_ix, after.item_ix);
    assert_eq!(
        scroll.logical_scroll_top().offset_in_item,
        after.offset_in_item
    );
    assert_eq!(position(&app, cx), left);

    // The same list/rendering implementation must reveal expanded tool groups
    // and follow new content only after an explicit return to the bottom.
    cx.update(|_, cx| app.update(cx, |app, cx| source.jump_to_end(app, cx)));
    settle(cx);
    assert!(state.borrow().at_bottom);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(2).unwrap().messages.push(
                serde_json::from_value(serde_json::json!({
                    "key": "new-stream-row", "kind": "Assistant", "text": "Fresh tail message."
                }))
                .unwrap(),
            );
            cx.notify();
        })
    });
    settle(cx);
    assert!(
        scroll.is_scrolled_to_end() == Some(true)
            || scroll.logical_scroll_top().item_ix == scroll.item_count()
    );
    assert_eq!(position(&app, cx), left);

    let right = scroll.logical_scroll_top();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.scroll.scroll_to(ListOffset {
                item_ix: 4,
                offset_in_item: px(0.),
            });
            app.at_bottom = false;
            cx.notify();
        })
    });
    draw(cx);
    assert_eq!(scroll.logical_scroll_top().item_ix, right.item_ix);

    // Expanding a right-side error changes its list-row height, not the main list.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(2).unwrap().messages = vec![serde_json::from_value(serde_json::json!({
            "key": "error-details", "kind": "Error", "text": "A synthetic failure.\n".repeat(30)
        })).unwrap()];
            source.jump_to_end(app, cx);
        })
    });
    settle(cx);
    let left = position(&app, cx);
    let height = cx
        .debug_bounds("cli-error-0")
        .expect("right error card")
        .size
        .height;
    let toggle = cx.debug_bounds("cli-error-toggle-0").unwrap().center();
    cx.simulate_mouse_down(toggle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(toggle, MouseButton::Left, Modifiers::default());
    settle(cx);
    assert!(cx.debug_bounds("cli-error-0").unwrap().size.height > height);
    assert_eq!(position(&app, cx), left);
}
