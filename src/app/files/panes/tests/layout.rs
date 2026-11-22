use super::*;
use crate::app::Store;
use gpui_kit::{Modifiers, MouseButton, point};

fn ratio(cx: &mut VisualTestContext) -> f32 {
    let main = cx.debug_bounds("content-main").unwrap();
    let reference = cx.debug_bounds("content-reference").unwrap();
    let total = f32::from(main.size.width + reference.size.width);
    assert!(f32::from(main.size.width) >= 319.);
    assert!(f32::from(reference.size.width) >= 319.);
    f32::from(main.size.width) / total
}

#[gpui_kit::test]
fn content_panes_ratio_persists_drag_and_survives_layout_transitions(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (settings_path, saved_ratio) = {
        let (app, cx, root) = fixture(cx, "content-panes-ratio");
        prepare(&app, cx, &root);
        let file = FileKey {
            workspace: root.clone(),
            relative: "main.txt".into(),
        };
        let id = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_workspace_file(file.clone(), true, window, cx);
                app.files.panes.active.unwrap()
            })
        });
        settle(cx);
        assert!((ratio(cx) - 0.5).abs() < 0.01);
        let main = cx.debug_bounds("content-main").unwrap();
        let reference = cx.debug_bounds("content-reference").unwrap();
        let start = point(reference.left(), reference.center().y);
        let end = point(
            main.left() + (main.size.width + reference.size.width) * 0.7,
            start.y,
        );
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            point(start.x + px(12.), start.y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        settle(cx);
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        settle(cx);
        assert_eq!(
            app.read_with(cx, |app, _| app.store.ui.content_split_ratio),
            None
        );
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        settle(cx);
        let saved_ratio = app.read_with(cx, |app, _| app.store.ui.content_split_ratio.unwrap());
        assert!((saved_ratio - 0.7).abs() < 0.02);
        assert!((ratio(cx) - saved_ratio).abs() < 0.01);
        let settings_path = root.join("ratio-state.json");
        app.read_with(cx, |app, _| app.store.save_to(&settings_path).unwrap());

        // A narrower two-column area clamps to minimum widths temporarily.
        let chrome = cx.update(|window, _| f32::from(window.viewport_size().width))
            - f32::from(main.size.width + reference.size.width);
        let stale_state = app.read_with(cx, |app, _| {
            app.files.panes.resize.as_ref().unwrap().1.clone()
        });
        cx.simulate_resize(size(px(chrome + 700.), px(1000.)));
        settle(cx);
        assert!((ratio(cx) - saved_ratio).abs() > 0.05);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.dirty = false;
                app.remember_content_split_ratio(&stale_state, cx);
                assert!(
                    !app.dirty,
                    "an old divider must not overwrite the preference"
                );
                assert_eq!(app.store.ui.content_split_ratio, Some(saved_ratio));
            });
        });

        cx.simulate_resize(size(px(850.), px(1000.)));
        settle(cx);
        assert!(
            cx.debug_bounds("content-main").is_none()
                || cx.debug_bounds("content-reference").is_none()
        );
        assert_eq!(
            app.read_with(cx, |app, _| app.store.ui.content_split_ratio),
            Some(saved_ratio)
        );
        cx.simulate_resize(size(px(1800.), px(1000.)));
        settle(cx);
        assert!((ratio(cx) - saved_ratio).abs() < 0.01);

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.move_file_to_pane(id, Pane::Main, window, cx)
            });
        });
        settle(cx);
        assert!(cx.debug_bounds("content-reference").is_none());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.move_file_to_pane(id, Pane::Reference, window, cx)
            });
        });
        settle(cx);
        assert!((ratio(cx) - saved_ratio).abs() < 0.01);
        cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
        settle(cx);
        cx.update(|window, cx| app.update(cx, |app, cx| app.close_settings(window, cx)));
        settle(cx);
        assert!((ratio(cx) - saved_ratio).abs() < 0.01);

        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.close_documents(&[DocumentTab::File(id)], window, cx);
            });
        });
        settle(cx);
        assert!(cx.debug_bounds("content-reference").is_none());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_workspace_file(file, true, window, cx)
            });
        });
        settle(cx);
        assert!((ratio(cx) - saved_ratio).abs() < 0.01);
        assert_eq!(
            std::fs::read_to_string(root.join("main.txt")).unwrap(),
            "main file\nneedle main\n"
        );
        (settings_path, saved_ratio)
    };

    // Restore the saved preferences into a fresh app before opening its first split.
    let restored = Store::load_from(&settings_path).unwrap();
    assert_eq!(restored.ui.content_split_ratio, Some(saved_ratio));
    let (app, cx, root) = fixture(cx, "content-panes-ratio-restored");
    prepare(&app, cx, &root);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui = restored.ui;
            assert!(app.files.panes.resize.is_none());
            app.open_workspace_file(reference(&root), true, window, cx);
        });
    });
    settle(cx);
    assert!((ratio(cx) - saved_ratio).abs() < 0.01);
    std::fs::remove_file(settings_path).unwrap();
}
