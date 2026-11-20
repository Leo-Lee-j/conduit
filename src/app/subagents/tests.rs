use super::{Agent, Status};
use gpui_kit::base::test_support::{ElementSnapshot, find};
use gpui_kit::{
    ElementId, KeyDownEvent, KeyUpEvent, Keystroke, SharedString, TestAppContext,
    VisualTestContext, px, size,
};

fn draw(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
}

fn button(cx: &mut VisualTestContext, id: &str) -> ElementSnapshot {
    let id: ElementId = SharedString::from(id.to_owned()).into();
    cx.update(|window, _| find(window, &[], &id))
        .expect("button is rendered")
}

fn focus_button(cx: &mut VisualTestContext, id: &str) {
    for _ in 0..128 {
        if button(cx, id).focused() == Some(true) {
            return;
        }
        cx.update(|window, cx| window.focus_next(cx));
        draw(cx);
    }
    panic!("button is not reachable by keyboard: {id}");
}

fn press(cx: &mut VisualTestContext, key: &str) {
    let keystroke = Keystroke::parse(key).unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke });
    draw(cx);
}

#[gpui_kit::test]
fn keyboard_disclosures_history_and_copy_preserve_the_composer(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "subagent-keyboard");
    cx.simulate_resize(size(px(1100.), px(700.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.session_mut(1).unwrap().subagents = vec![
                Agent {
                    id: "old".into(),
                    run: "previous".into(),
                    title: "历史任务".into(),
                    status: Status::Completed,
                    ..Default::default()
                },
                Agent {
                    id: "current".into(),
                    run: "latest".into(),
                    title: "检查中文文件链接".into(),
                    role: "reviewer".into(),
                    task: "检查文件链接\n保留原始任务内容".into(),
                    status: Status::Failed,
                    ..Default::default()
                },
            ];
            app.input.update(cx, |input, cx| {
                input.set_value("待发送草稿", window, cx);
            });
            cx.notify();
        });
    });
    draw(cx);
    assert_eq!(
        button(cx, "subagents-toggle-1").label(),
        Some("收起 · 子代理 1 · 1 失败")
    );
    assert_eq!(button(cx, "subagents-history").label(), Some("查看历史"));
    focus_button(cx, "subagents-toggle-1");
    press(cx, "enter");
    assert!(cx.debug_bounds("subagent-rows").is_none());
    assert_eq!(
        button(cx, "subagents-toggle-1").label(),
        Some("展开 · 子代理 1 · 1 失败")
    );
    press(cx, "space");
    assert!(cx.debug_bounds("subagent-rows").is_some());

    focus_button(cx, "subagents-history");
    press(cx, "enter");
    assert_eq!(button(cx, "subagents-history").label(), Some("当前任务"));
    assert!(button(cx, "subagent-previous:old").visible());
    press(cx, "space");
    assert_eq!(button(cx, "subagents-history").label(), Some("查看历史"));

    focus_button(cx, "subagent-latest:current");
    assert_eq!(
        button(cx, "subagent-latest:current").label(),
        Some("展开 · reviewer · 失败: 检查中文文件链接")
    );
    press(cx, "enter");
    assert_eq!(
        button(cx, "subagent-latest:current").label(),
        Some("收起 · reviewer · 失败: 检查中文文件链接")
    );
    focus_button(cx, "copy-task-latest:current");
    press(cx, "enter");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("检查文件链接\n保留原始任务内容".into())
    );

    // Completion removes the details; the same keyboard control must still
    // let the user reopen results and retain that choice on the next paint.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().subagents[1].status = Status::Completed;
            cx.notify();
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("subagent-rows").is_none());
    focus_button(cx, "subagents-toggle-1");
    press(cx, "enter");
    draw(cx);
    assert!(cx.debug_bounds("subagent-rows").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.input.read(cx).value(), "待发送草稿");
        assert!(app.session_control.runs.is_empty());
        assert!(app.queued_messages.is_empty());
        assert!(app.session(1).unwrap().messages.is_empty());
    });
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

fn copy_detail_after_composer_focus(cx: &mut TestAppContext, activity: bool) {
    use gpui_kit::{ClipboardItem, Focusable, Modifiers, MouseButton, point};

    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "subagent-selection-copy");
    cx.simulate_resize(size(px(1100.), px(700.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            let text = "abcdefghijklmnopqrstuvwxyz\nabcdefghijklmnopqrstuvwxyz\nabcdefghijklmnopqrstuvwxyz";
            app.session_mut(1).unwrap().subagents = vec![Agent {
                id: "child".into(),
                run: "latest".into(),
                title: "检查复制".into(),
                task: if activity { String::new() } else { text.into() },
                activity: if activity { text.into() } else { String::new() },
                status: Status::Failed,
                ..Default::default()
            }];
            app.session_control
                .expanded_agents
                .insert((1, "latest:child".into()));
            app.input.update(cx, |input, cx| {
                input.set_value("待发送草稿", window, cx);
            });
            app.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        });
    });
    draw(cx);

    // Select part of the last line in already-open details while the composer
    // owns keyboard focus. Copying the whole task with its button misses this.
    let rows = cx.debug_bounds("subagent-rows").unwrap();
    let start = point(rows.left() + px(20.), rows.bottom() - px(20.));
    let end = start + point(px(65.), px(0.));
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    draw(cx);
    let selected = cx.update(gpui_kit::base::TextSelection::selected_text);
    assert!(
        !selected.is_empty() && selected.len() < 26,
        "must select part of the detail text: {selected:?}"
    );
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into())));
    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some(selected)
    );
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert!(!app.input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(app.input.read(cx).value(), "待发送草稿");
        assert!(app.session_control.runs.is_empty());
        assert!(app.queued_messages.is_empty());
        assert!(app.session(1).unwrap().messages.is_empty());
        app.input.read(cx).focus_handle(cx).focus(window, cx);
    });
    // Returning to the composer still routes editing shortcuts to the input.
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some("待发送草稿".into())
    );
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn task_selection_copies_after_composer_focus(cx: &mut TestAppContext) {
    copy_detail_after_composer_focus(cx, false);
}

#[gpui_kit::test]
fn activity_selection_copies_after_composer_focus(cx: &mut TestAppContext) {
    copy_detail_after_composer_focus(cx, true);
}
