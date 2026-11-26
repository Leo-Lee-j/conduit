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

#[gpui_kit::test]
fn child_navigation_preserves_running_root_and_rejects_desktop_writes(cx: &mut TestAppContext) {
    use crate::app::{Event, Provider};
    use gpui_kit::Focusable as _;
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "child-navigation");
    cx.simulate_resize(size(px(850.), px(500.)));
    let sender = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            let root = app.session_mut(1).unwrap();
            root.provider = Provider::Codex;
            root.native_id = Some("root".into());
            root.subagents = vec![
                Agent {
                    id: "a".into(),
                    parent_id: "root".into(),
                    run: "run".into(),
                    title: "Child A".into(),
                    task: "Saved task A".into(),
                    status: Status::Running,
                    ..Default::default()
                },
                Agent {
                    id: "b".into(),
                    parent_id: "root".into(),
                    run: "run".into(),
                    title: "Child B".into(),
                    status: Status::Running,
                    ..Default::default()
                },
            ];
            app.input.update(cx, |input, cx| {
                input.set_value("root draft", window, cx);
                input.set_selected_range(2..5, cx);
            });
            crate::app::queue::tests::running(app, 1)
        })
    });
    draw(cx);
    focus_button(cx, "subagent-open-run:a");
    press(cx, "enter");
    assert!(cx.debug_bounds("subagent-page").is_some());
    assert!(cx.debug_bounds("subagent-history-skeleton").is_some());
    assert!(cx.debug_bounds("subagent-summary").is_none());
    assert!(button(cx, "subagent-back").visible());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.subagent_route().unwrap().agent, "a");
            assert!(!app.session_is_visible(1));
            assert!(!app.input.read(cx).focus_handle(cx).is_focused(window));
            assert!(app.visible_subagent_transcript().is_none());
            app.send_prompt(window, cx);
            app.send_ordinary_prompt(window, cx);
            assert!(!app.send_text("must not send".into(), true, window, cx));
            app.stop_reply(window, cx);
            app.regenerate(0, window, cx);
            app.retry_error(0, window, cx);
            assert!(app.submit_chat_command(window, cx));
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
            assert!(app.queued_messages.is_empty());
            assert!(app.session(1).unwrap().messages.is_empty());
            assert_eq!(app.input.read(cx).value(), "root draft");
            assert_eq!(app.input.read(cx).selected_range(), 2..5);
            app.open_subagent(2, "run".into(), "a".into(), window, cx);
            assert_eq!(app.subagent_route().unwrap().root, 1);
        })
    });
    sender
        .send(Event::Delta {
            key: "root-output".into(),
            text: "root keeps working".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll(window, cx)));
    draw(cx);
    focus_button(cx, "subagent-next");
    press(cx, "space");
    cx.update(|_, cx| assert_eq!(app.read(cx).subagent_route().unwrap().agent, "b"));
    focus_button(cx, "subagent-back");
    press(cx, "enter");
    assert!(cx.debug_bounds("subagent-page").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.viewing_subagent());
            assert_eq!(app.input.read(cx).value(), "root draft");
            assert_eq!(app.input.read(cx).selected_range(), 2..5);
            assert!(app.input.read(cx).focus_handle(cx).is_focused(window));
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
            assert!(
                app.session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .any(|item| item.text == "root keeps working")
            );
            // Explicitly addressed controls retain their root target even in a child view.
            app.open_subagent(1, "run".into(), "a".into(), window, cx);
            assert!(app.stop_session_run(1));
            assert!(app.session_control.runs[&1].runtime.is_cancelled());
        })
    });
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

#[gpui_kit::test]
fn saved_child_identity_loading_failure_and_tab_restore_are_isolated(cx: &mut TestAppContext) {
    use crate::app::{DocumentTab, Event, Provider};
    let _locale = crate::test_support::chinese();
    let (app, cx, disk) = crate::app::queue::tests::fixture(cx, "child-saved-history");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            let root = app.session_mut(1).unwrap();
            root.provider = Provider::Codex;
            root.native_id = Some("root".into());
            root.subagents.push(Agent {
                id: "child".into(),
                parent_id: "root".into(),
                run: "run".into(),
                title: "Saved child".into(),
                ..Default::default()
            });
            let mut child = crate::app::tests::session();
            child.id = 3;
            child.provider = Provider::Codex;
            child.native_id = Some("child".into());
            child.is_subagent = true;
            crate::app::events::apply_event(
                &mut child,
                "child",
                Event::Text {
                    key: "reply".into(),
                    text: "saved child reply".into(),
                },
            );
            crate::transcript::save(&child, &disk.join("bodies")).unwrap();
            assert!(crate::transcript::unload(&mut child));
            app.store.workspaces[0].sessions.push(child);
            app.input.update(cx, |input, cx| {
                input.set_value("retained root draft", window, cx)
            });
            app.open_subagent(1, "run".into(), "child".into(), window, cx);
            assert_eq!(app.visible_subagent_transcript(), Some(3));
            assert_eq!(app.selected_session, Some(1));
        })
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let skeleton_id = String::from_utf8(vec![
        115, 117, 98, 97, 103, 101, 110, 116, 45, 104, 105, 115, 116, 111, 114, 121, 45, 115, 107,
        101, 108, 101, 116, 111, 110,
    ])
    .unwrap();
    let skeleton_id = Box::leak(skeleton_id.into_boxed_str());
    assert!(cx.debug_bounds(skeleton_id).is_some());
    draw(cx);
    assert!(cx.debug_bounds("subagent-page").is_some());
    let message_list_bounds = cx
        .debug_bounds("subagent-message-list")
        .expect("saved child message list is rendered");
    assert!(message_list_bounds.size.height > px(0.));
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let child = app.session(3).unwrap();
            assert!(child.transcript.loaded);
            assert_eq!(child.messages.len(), 1);
            assert_eq!(child.messages[0].text, "saved child reply");
        })
    });
    assert!(button(cx, "subagent-back").visible());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_control.unread_completed.insert(1);
            app.switch(1, Some(2), window, cx);
            app.select_document(DocumentTab::Chat(1), window, cx);
            assert_eq!(app.visible_subagent_transcript(), Some(3));
            assert!(app.session_control.unread_completed.contains(&1));
            assert_eq!(app.input.read(cx).value(), "retained root draft");
            let child = app.session_mut(3).unwrap();
            crate::transcript::save(child, &disk.join("bodies")).unwrap();
            assert!(crate::transcript::unload(child));
            child.transcript.reference.as_mut().unwrap().directory = disk.join("missing-bodies");
            app.request_transcript(3, cx);
        })
    });
    draw(cx);
    assert!(button(cx, "subagent-back").visible());
    assert!(button(cx, "subagent-retry-history").visible());
    focus_button(cx, "subagent-back");
    press(cx, "enter");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.viewing_subagent());
            assert_eq!(app.input.read(cx).value(), "retained root draft");
            app.open_subagent(1, "run".into(), "child".into(), window, cx);
            app.switch(0, Some(1), window, cx); // Explicit sidebar navigation opens the root.
            assert!(!app.viewing_subagent());
            let mut alias = app.session(3).unwrap().clone();
            alias.id = 4;
            app.store.workspaces[0].sessions.push(alias);
            app.open_subagent(1, "run".into(), "child".into(), window, cx);
            assert!(app.visible_subagent_transcript().is_none());
            app.return_from_subagent(1, window, cx);
            app.store.workspaces[0]
                .sessions
                .retain(|session| session.id != 4);
            app.session_mut(1).unwrap().subagents[0].parent_id = "unrelated".into();
            app.open_subagent(1, "run".into(), "child".into(), window, cx);
            assert!(app.visible_subagent_transcript().is_none());
            app.return_from_subagent(1, window, cx);
            app.session_mut(1).unwrap().provider = Provider::Claude;
            app.session_mut(3).unwrap().provider = Provider::Claude;
            app.session_mut(1).unwrap().subagents[0].parent_id = "root".into();
            app.open_subagent(1, "run".into(), "child".into(), window, cx);
            assert!(app.visible_subagent_transcript().is_none()); // Claude task IDs are not sessions.
            app.store.workspaces[0]
                .sessions
                .retain(|session| session.id != 1);
            app.return_from_subagent(1, window, cx);
            assert!(app.viewing_subagent());
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.input.read(cx).value(), "retained root draft");
            assert!(!app.send_text("orphan must remain read-only".into(), true, window, cx));
        })
    });
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}
