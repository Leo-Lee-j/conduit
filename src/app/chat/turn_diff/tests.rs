use super::{ChatItem, ItemKind, SessionRecord, file_edits, turn_file_stats};

fn message(key: &str, kind: ItemKind, text: &str, input: serde_json::Value) -> ChatItem {
    serde_json::from_value(serde_json::json!({
        "key": key, "kind": kind, "text": text,
        "input": input.to_string(), "status": "completed"
    }))
    .unwrap()
}

fn fixture() -> SessionRecord {
    let mut session = crate::app::tests::session();
    session.messages = vec![
        message(
            "old",
            ItemKind::Tool,
            "Write",
            serde_json::json!({"file_path":"src/a.rs", "content":"old turn"}),
        ),
        message(
            "turn",
            ItemKind::User,
            "修改两个文件",
            serde_json::Value::Null,
        ),
        message(
            "first",
            ItemKind::Tool,
            "Edit",
            serde_json::json!({"file_path":"src/a.rs", "old_string":"before", "new_string":"first"}),
        ),
        message(
            "other-file",
            ItemKind::Tool,
            "Write",
            serde_json::json!({"file_path":"src/z.rs", "content":"other file"}),
        ),
        message(
            "second",
            ItemKind::Tool,
            "修改文件",
            serde_json::json!([{"path":"src/a.rs", "kind":"update", "diff":"@@ -1 +1 @@\n-first\n+second\n"}]),
        ),
    ];
    session
}

#[test]
fn file_preview_keeps_the_clicked_turn_file_and_edit_order() {
    let mut session = fixture();
    let stats = turn_file_stats(&session);
    session.messages.extend([
        message("later", ItemKind::User, "下一轮", serde_json::Value::Null),
        message(
            "new-edit",
            ItemKind::Tool,
            "Write",
            serde_json::json!({"file_path":"src/a.rs", "content":"later turn"}),
        ),
    ]);
    let edits = file_edits(&session, Some("turn"), "src/a.rs");
    assert_eq!(edits.len(), 2);
    assert!(edits[0].patch.contains("+first"));
    assert!(edits[1].patch.contains("+second"));
    assert_eq!(
        edits.iter().map(|edit| edit.added).sum::<usize>(),
        stats.files["src/a.rs"].added
    );
    assert_eq!(
        edits.iter().map(|edit| edit.removed).sum::<usize>(),
        stats.files["src/a.rs"].removed
    );
    assert_eq!(file_edits(&session, None, "src/a.rs").len(), 1);
    assert!(file_edits(&session, Some("missing-turn"), "src/a.rs").is_empty());
    assert!(file_edits(&session, Some("turn"), "src/missing.rs").is_empty());
}

#[cfg(feature = "ui-tests")]
mod ui {
    use super::*;
    use crate::app::{ConduitApp, Store, Workspace};
    use gpui_kit::{
        AppContext, Modifiers, MouseButton, TestAppContext, VisualTestContext,
        component::{Root, WindowExt},
        px, size,
    };
    use std::{rc::Rc, time::Duration};

    fn settle(cx: &mut VisualTestContext) {
        for _ in 0..4 {
            cx.run_until_parked();
            cx.background_executor
                .advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            cx.update(|window, cx| {
                window.simulate_next_frame(cx);
                let _ = window.draw(cx);
            });
        }
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let position = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector}"))
            .center();
        cx.simulate_mouse_move(position, None, Modifiers::default());
        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        settle(cx);
    }

    #[gpui_kit::test]
    fn stats_file_click_opens_diff_and_preserves_chat(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let store = Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir().join("conduit-turn-preview"),
                        sessions: vec![fixture()],
                    }],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.input
                    .update(cx, |input, cx| input.set_value("保留这段草稿", window, cx));
                app
            });
            *output.borrow_mut() = Some(app.clone());
            Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        for (width, height) in [(1280., 900.), (760., 580.)] {
            cx.simulate_resize(size(px(width), px(height)));
            settle(cx);
            let scroll = cx.update(|_, cx| app.read(cx).scroll.logical_scroll_top());
            click(cx, "turn-stats");
            assert!(cx.debug_bounds("turn-stats-file-1").is_some());
            click(cx, "turn-stats-file-0");
            assert!(cx.debug_bounds("turn-stats-details").is_none());
            let preview = cx.debug_bounds("turn-diff-preview").unwrap();
            assert!(preview.left() >= px(0.) && preview.right() <= px(width));
            assert!(preview.top() >= px(0.) && preview.bottom() <= px(height));
            assert!(cx.debug_bounds("turn-diff-edit-0").is_some());
            click(cx, "turn-diff-next-edit");
            assert!(cx.debug_bounds("turn-diff-edit-1").is_some());
            click(cx, "turn-diff-unified");
            assert!(cx.debug_bounds("turn-diff-unified-body").is_some());
            click(cx, "turn-diff-previous-edit");
            assert!(cx.debug_bounds("turn-diff-edit-0").is_some());
            click(cx, "turn-diff-split");
            assert!(cx.debug_bounds("turn-diff-split-body").is_some());
            cx.simulate_keystrokes("escape");
            settle(cx);
            assert!(cx.debug_bounds("turn-diff-preview").is_none());
            cx.update(|window, cx| {
                assert!(!window.has_active_dialog(cx));
                let app = app.read(cx);
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.input.read(cx).value().as_str(), "保留这段草稿");
                let after = app.scroll.logical_scroll_top();
                assert_eq!(after.item_ix, scroll.item_ix);
                assert_eq!(after.offset_in_item, scroll.offset_in_item);
                assert!(app.active_diff.is_none());
            });
            click(cx, "turn-stats");
            click(cx, "turn-stats-file-1");
            assert!(cx.debug_bounds("turn-diff-next-edit").is_none());
            click(cx, "turn-diff-close");
            assert!(cx.debug_bounds("turn-diff-preview").is_none());
        }
    }

    #[gpui_kit::test]
    fn latest_arrow_aligns_with_stats_above_queue_and_scrolls_to_end(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut session = fixture();
                let history = (0..40).map(|index| {
                    message(
                        &format!("history-{index}"),
                        if index % 2 == 0 {
                            ItemKind::User
                        } else {
                            ItemKind::Assistant
                        },
                        &"较早的聊天内容。\n".repeat(6),
                        serde_json::Value::Null,
                    )
                });
                session.messages.splice(0..0, history);
                let store = Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir().join("conduit-latest-arrow"),
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
            Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        for (width, height, full_width, queued) in [
            (1440., 900., false, false),
            (1440., 900., true, true),
            (850., 600., false, true),
        ] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.store.ui.full_width_content = full_width;
                    if queued {
                        app.queued_messages.insert(
                            1,
                            crate::store::QueuedMessage {
                                text: "接着检查".into(),
                                references: Vec::new(),
                                paused: Some("等待发送".into()),
                            },
                        );
                    }
                    cx.notify();
                })
            });
            settle(cx);
            assert!(cx.debug_bounds("scroll-to-latest").is_none());
            cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "history-0", cx)));
            settle(cx);
            let arrow = cx.debug_bounds("scroll-to-latest").unwrap();
            let composer = cx.debug_bounds("composer").unwrap();
            let stats = cx.debug_bounds("turn-stats").unwrap();
            let status_row = cx.debug_bounds("turn-status-row").unwrap();
            assert!((arrow.center().y - stats.center().y).abs() < px(1.));
            assert_eq!(arrow.size.height, stats.size.height);
            assert_eq!(arrow.size.width, arrow.size.height);
            assert!(arrow.left() >= stats.right() + px(6.));
            assert!(arrow.right() <= status_row.right());
            assert!(arrow.bottom() < composer.top());
            if queued {
                assert!(arrow.bottom() < cx.debug_bounds("queued-message").unwrap().top());
            }
            click(cx, "scroll-to-latest");
            assert!(cx.debug_bounds("scroll-to-latest").is_none());
            let tail = cx
                .debug_bounds("message-row-44")
                .expect("latest message is rendered");
            assert!(tail.bottom() <= cx.debug_bounds("turn-status-row").unwrap().top());
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(app.at_bottom);
                assert!(app.pending_message_jump.is_none());
                assert!(app.message_focus.is_none());
            });
        }
    }
}
