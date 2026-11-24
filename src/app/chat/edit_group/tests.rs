use super::{ChatItem, status};
use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};

fn edit(index: usize) -> ChatItem {
    serde_json::from_value(serde_json::json!({
        "key": format!("edit-{index}"), "kind": "Tool", "text": "Edit",
        "input": serde_json::json!({
            "file_path": format!("src/file-{}.rs", index % 3),
            "old_string": format!("before-{index}"), "new_string": format!("after-{index}")
        }).to_string(), "status": "completed"
    }))
    .unwrap()
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

fn toggle(cx: &mut VisualTestContext) {
    let center = cx.debug_bounds("edit-group-toggle-1-0").unwrap().center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn edit_group_disclosure_shows_every_patch_and_keeps_its_choice_when_appended(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "edit-group");
    cx.simulate_resize(size(px(1100.), px(900.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().messages = (0..16).map(edit).collect();
            app.sync_chat_rows();
            cx.notify();
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("edit-group-toggle-1-0").is_some());
    assert!(cx.debug_bounds("edit-group-body-1-0").is_none());
    assert!(cx.debug_bounds("message-row-1").is_none());
    toggle(cx);
    assert!(cx.debug_bounds("edit-group-body-1-0").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let items = app.session(1).unwrap().messages.clone();
            let analyses: Vec<_> = items
                .iter()
                .map(|item| app.tool_analysis.get(item, cx).unwrap())
                .collect();
            let patch: String = analyses
                .iter()
                .flat_map(|value| value.files.iter())
                .map(|file| file.patch.as_str())
                .collect();
            let state = app
                .tool_diffs
                .state(&(1, "edit-0".into()), &patch, cx)
                .unwrap();
            assert_eq!(state.read(cx).files().len(), 16);
            for index in 0..16 {
                assert_eq!(
                    state.read(cx).files()[index].path().as_ref(),
                    format!("src/file-{}.rs", index % 3)
                );
                assert!(patch.contains(&format!("-before-{index}\n+after-{index}\n")));
            }
            app.session_mut(1).unwrap().messages.push(edit(16));
            app.sync_chat_rows();
            cx.notify();
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("edit-group-body-1-0").is_some());
    toggle(cx);
    assert!(cx.debug_bounds("edit-group-body-1-0").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages.push(edit(17));
            app.sync_chat_rows();
            cx.notify();
        });
    });
    draw(cx);
    assert!(cx.debug_bounds("edit-group-body-1-0").is_none());
}

#[test]
fn group_status_keeps_failures_and_running_edits_visible() {
    let mut items = vec![edit(0), edit(1), edit(2)];
    assert_eq!(status(&items), items[2].status);
    items[0].status = "failed".into();
    assert_eq!(status(&items), items[0].status);
    items[0].status = Default::default();
    items[2].status = "failed".into();
    assert_eq!(status(&items), items[2].status);
    items[1].status = "running".into();
    assert_eq!(status(&items), items[1].status);
}
