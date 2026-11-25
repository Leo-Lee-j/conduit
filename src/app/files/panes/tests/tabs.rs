use super::*;
use gpui_kit::{Focusable as _, Modifiers, MouseButton};

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let point = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_mouse_move(point, None, Modifiers::default());
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_chat_tabs_keep_height_and_close_buttons_with_multiline_titles(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-chat-tab-bounds");
    prepare(&app, cx, &root);
    let title = "SELECT very_long_column_name, another_long_column_name\r\n\tFROM database.table\u{2028}WHERE id > 0\u{2029}ORDER BY id";
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.workspaces[0].sessions[0].title = title.into();
            app.store.workspaces[1].sessions[0].title = title.into();
            app.show_chat_beside(2, window, cx);
        });
    });
    // The same title belongs to two distinct tabs, even when workspace groups
    // consume label space. A short window stresses the vertical flex layout.
    for group_tabs in [false, true] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.group_tabs = group_tabs;
                cx.notify();
            });
        });
        cx.simulate_resize(size(px(1800.), px(500.)));
        settle(cx);
        for (strip, tab, close) in [
            ("document-tab-strip", "session-tab-1", "session-tab-close-1"),
            (
                "reference-tab-strip",
                "session-tab-2",
                "session-tab-close-2",
            ),
        ] {
            let strip = cx.debug_bounds(strip).expect(strip);
            let tab = cx.debug_bounds(tab).expect(tab);
            let close = cx.debug_bounds(close).expect(close);
            assert_eq!(strip.size.height, px(crate::app::TAB_BAR_HEIGHT));
            assert!(tab.top() >= strip.top() && tab.bottom() <= strip.bottom());
            assert!(tab.size.width <= px(240.));
            assert!(close.top() >= strip.top() && close.bottom() <= strip.bottom());
            assert!(close.left() > tab.center().x && close.right() <= tab.right());
        }
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.document_tabs_for(Pane::Main),
                vec![DocumentTab::Chat(1)]
            );
            assert_eq!(
                app.document_tabs_for(Pane::Reference),
                vec![DocumentTab::Chat(2)]
            );
            assert_eq!(app.pane_document(Pane::Main), Some(DocumentTab::Chat(1)));
            assert_eq!(
                app.pane_document(Pane::Reference),
                Some(DocumentTab::Chat(2))
            );
            assert_eq!(
                app.session(2).unwrap().title,
                title,
                "keep the stored title intact"
            );
        });
    }
    click(cx, "session-tab-close-2");
    app.read_with(cx, |app, _| {
        assert_eq!(app.open_tabs, vec![DocumentTab::Chat(1)]);
        assert_eq!(app.selected_session, Some(1));
        assert!(app.pane_document(Pane::Reference).is_none());
    });
}

#[gpui_kit::test]
fn content_panes_chat_moves_and_batch_close_preserve_owners_and_drafts(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-chat-tab-owners");
    prepare(&app, cx, &root);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut third = app.session(1).unwrap().clone();
            third.id = 3;
            third.native_id = None;
            app.store.workspaces[0].sessions.push(third);
            app.input
                .update(cx, |input, cx| input.set_value("left draft", window, cx));
            app.show_chat_beside(2, window, cx);
            app.files.panes.chat_views[&2].update(cx, |view, cx| {
                view.input
                    .update(cx, |input, cx| input.set_value("right draft", window, cx));
            });
            app.show_chat_beside(3, window, cx);
            let registered = app.reference_chat_views();
            assert_eq!(registered.len(), 2);
            for (id, view) in registered {
                assert!(app.reference_chat_view(id) == Some(view));
            }
            assert!(app.reference_chat_view(1).is_none());
            app.select_document(DocumentTab::Chat(2), window, cx);
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));
            assert_eq!(app.input.read(cx).value(), "left draft");
            app.move_chat_to_main(2, window, cx);
            assert!(app.reference_chat_view(2).is_none());
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(app.selected_workspace, Some(1));
            assert_eq!(app.input.read(cx).value(), "right draft");
            assert_eq!(
                app.pane_document(Pane::Reference),
                Some(DocumentTab::Chat(3))
            );
            assert_eq!(app.document_pane(DocumentTab::Chat(2)), Pane::Main);
            app.show_chat_beside(2, window, cx);
            assert_eq!(app.document_pane(DocumentTab::Chat(2)), Pane::Reference);
            assert_eq!(
                app.pane_document(Pane::Reference),
                Some(DocumentTab::Chat(2))
            );
            assert_eq!(
                app.open_tabs
                    .iter()
                    .filter(|tab| **tab == DocumentTab::Chat(2))
                    .count(),
                1
            );
            app.select_document(DocumentTab::Chat(1), window, cx);
            app.show_content_pane(Pane::Reference, window, cx);
            let reference = app.document_tabs_for(Pane::Reference);
            assert_eq!(reference.len(), 2);
            app.close_documents(&reference, window, cx);
            assert_eq!(app.open_tabs, vec![DocumentTab::Chat(1)]);
            assert!(app.files.panes.chat_views.is_empty());
            assert!(app.reference_chat_views().is_empty());
            assert!(app.pane_document(Pane::Reference).is_none());
            assert!(!app.files.panes.split);
            assert_eq!(app.files.panes.last, Pane::Main);
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.input.read(cx).value(), "left draft");
            assert_eq!(app.drafts.get(&2).map(String::as_str), Some("right draft"));
            assert!(
                app.input
                    .read(cx)
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            );
        });
    });
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_reference_composer_lifecycle_rejects_closed_and_moved_bindings(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-composer-lifecycle");
    prepare(&app, cx, &root);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.show_chat_beside(2, window, cx);
            let input = app.reference_chat_view(2).unwrap().read(cx).input.clone();
            let original = app.composer_binding_for_session(2, input.clone()).unwrap();
            // Registration must be live before the first reference render.
            assert!(app.composer_binding_live(&original));
            input.update(cx, |input, cx| {
                input.set_value("preserved reference draft", window, cx)
            });
            app.show_chat_beside(2, window, cx);
            assert!(app.reference_chat_view(2).unwrap().read(cx).input == input);
            assert!(app.composer_binding_live(&original));
            let main = app.composer_binding();

            // Close and reopen inside a bound callback. Its scoped editor
            // survives until unwind, but must become unusable immediately.
            let reopened = app
                .with_composer(original.clone(), cx, |app, cx| {
                    app.close_documents(&[DocumentTab::Chat(2)], window, cx);
                    assert!(!app.composer_binding_live(&original));
                    assert!(app.with_composer(original.clone(), cx, |_, _| ()).is_none());
                    app.show_chat_beside(2, window, cx);
                    let next = app.reference_chat_view(2).unwrap().read(cx).input.clone();
                    assert!(next != input);
                    assert_eq!(next.read(cx).value(), "preserved reference draft");
                    let binding = app.composer_binding_for_session(2, next).unwrap();
                    assert!(app.composer_binding_live(&binding));
                    assert!(!app.composer_binding_live(&original));
                    binding
                })
                .expect("registered reference callback");
            assert!(app.composer_binding_live(&main));
            assert!(app.composer_input() == main.input);
            assert!(app.composer_binding_live(&reopened));
            assert!(app.with_composer(original.clone(), cx, |_, _| ()).is_none());
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));

            app.move_chat_to_main(2, window, cx);
            assert!(!app.composer_binding_live(&reopened));
            assert!(app.with_composer(reopened, cx, |_, _| ()).is_none());
            assert!(app.reference_chat_view(2).is_none());
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(app.input.read(cx).value(), "preserved reference draft");
        });
    });
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_closing_main_file_never_selects_a_reference_chat(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-chat-tab-main-close");
    prepare(&app, cx, &root);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.files_open_beside = false;
            app.open_workspace_file(
                FileKey {
                    workspace: root.clone(),
                    relative: "main.txt".into(),
                },
                true,
                window,
                cx,
            );
            let file = app.active_file.unwrap();
            // The visible main document survives moving its background chat.
            app.show_chat_beside(1, window, cx);
            assert_eq!(app.pane_document(Pane::Main), Some(DocumentTab::File(file)));
            assert_eq!(app.selected_session, None);
            assert_eq!(app.document_pane(DocumentTab::Chat(1)), Pane::Reference);
            app.close_documents(&[DocumentTab::File(file)], window, cx);
            assert!(app.pane_document(Pane::Main).is_none());
            assert_eq!(app.selected_session, None);
            assert_eq!(
                app.pane_document(Pane::Reference),
                Some(DocumentTab::Chat(1))
            );
            assert!(app.document_tabs_for(Pane::Main).is_empty());
            assert_eq!(
                app.document_tabs_for(Pane::Reference),
                vec![DocumentTab::Chat(1)]
            );
        });
    });
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_reference_chat_navigation_exits_overlays_in_narrow_windows(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-chat-tab-overlays");
    prepare(&app, cx, &root);
    let main_file = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.files_open_beside = false;
            app.open_workspace_file(
                FileKey {
                    workspace: root.clone(),
                    relative: "main.txt".into(),
                },
                true,
                window,
                cx,
            );
            let file = app.active_file.unwrap();
            app.show_chat_beside(2, window, cx);
            file
        })
    });
    cx.simulate_resize(size(px(850.), px(800.)));
    settle(cx);
    assert!(cx.debug_bounds("content-reference").is_some());
    assert!(cx.debug_bounds("content-main").is_none());
    click(cx, "show-main-pane");
    assert!(cx.debug_bounds("content-main").is_some());
    assert!(cx.debug_bounds("content-reference").is_none());
    click(cx, "show-reference-pane");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let focus = app
                .reference_chat_view(2)
                .unwrap()
                .read(cx)
                .chat_focus
                .clone();
            focus.focus(window, cx);
            // Model a stale last-pane value while selection focus is in the
            // right transcript; layout must follow that actual content focus.
            app.files.panes.last = Pane::Main;
            app.update_content_pane_layout(500., window, cx);
            assert_eq!(app.files.panes.last, Pane::Reference);
            assert!(app.reference_pane_visible());
        });
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_settings(window, cx);
            assert!(app.main_pane_visible());
            assert!(!app.reference_pane_visible());
            // Match the direct entry used by sidebar session navigation.
            app.select_reference_chat(2, window, cx);
            assert!(!app.content_overlay_open());
            assert!(app.files.panes.overlay.is_none());
            assert_eq!(app.active_file, Some(main_file));
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.files.panes.last, Pane::Reference);
            app.open_settings(window, cx);
            app.close_documents(&[DocumentTab::Settings], window, cx);
            assert!(app.reference_pane_visible());
            assert_eq!(
                app.pane_document(Pane::Main),
                Some(DocumentTab::File(main_file))
            );
            // An empty Main has no document to return to. The overlay must
            // still restore the surviving right pane and its focus.
            app.close_documents(
                &[DocumentTab::File(main_file), DocumentTab::Chat(1)],
                window,
                cx,
            );
            app.show_content_pane(Pane::Reference, window, cx);
            app.open_settings(window, cx);
            app.close_documents(&[DocumentTab::Settings], window, cx);
            assert!(!app.content_overlay_open());
            assert!(app.files.panes.overlay.is_none());
            assert!(app.reference_pane_visible());
            assert_eq!(
                app.pane_document(Pane::Reference),
                Some(DocumentTab::Chat(2))
            );
        });
    });
    settle(cx);
    assert!(cx.debug_bounds("content-reference").is_some());
    assert!(cx.debug_bounds("content-main").is_none());
}
