use super::{FilePreview, init, match_color};
use gpui_kit::base::TextSelection;
use gpui_kit::component::{Root, v_flex};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Background, Context, Entity, Focusable as _, IntoElement, Modifiers,
    MouseButton, ParentElement, Render, ScrollHandle, Styled, TestAppContext, VisualTestContext,
    Window, point, px, size,
};
use std::{rc::Rc, sync::Arc, time::Duration};

struct Harness {
    views: Vec<Entity<FilePreview>>,
    active: usize,
}
impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().child(self.views[self.active].clone())
    }
}

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

fn type_query(cx: &mut VisualTestContext, view: &Entity<FilePreview>, text: &str) {
    cx.update(|window, cx| {
        let input = view.read(cx).input.clone();
        input.read(cx).focus_handle(cx).focus(window, cx);
    });
    cx.simulate_keystrokes("ctrl-a");
    if text.is_empty() {
        cx.simulate_keystrokes("backspace");
    } else {
        cx.simulate_input(text);
    }
}

fn query(cx: &mut VisualTestContext, view: &Entity<FilePreview>, text: &str) {
    type_query(cx, view, text);
    settle(cx);
}

fn highlighted(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| {
        let color: Background = match_color(true).into();
        window
            .painted_quads()
            .iter()
            .any(|quad| quad.background == color)
    })
}

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    documents: &[(bool, &str)],
) -> (
    &'a mut VisualTestContext,
    Entity<Harness>,
    Vec<Entity<FilePreview>>,
) {
    cx.update(gpui_kit::init);
    cx.update(init);
    let slot = Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let views = documents
            .iter()
            .map(|&(markdown, text)| {
                cx.new(|cx| {
                    let mut view = FilePreview::new(markdown, ScrollHandle::new(), window, cx);
                    view.sync(
                        text.to_owned().into(),
                        Arc::default(),
                        true,
                        view.typography.clone(),
                        cx,
                    );
                    view
                })
            })
            .collect::<Vec<_>>();
        views[0].read(cx).focus.clone().focus(window, cx);
        let harness = cx.new(|_| Harness {
            views: views.clone(),
            active: 0,
        });
        *output.borrow_mut() = Some((harness.clone(), views));
        Root::new(harness, window, cx)
    });
    let (harness, views) = slot.borrow_mut().take().unwrap();
    cx.simulate_resize(size(px(650.), px(400.)));
    settle(cx);
    (cx, harness, views)
}

fn activate(cx: &mut VisualTestContext, harness: &Entity<Harness>, index: usize) {
    cx.update(|window, cx| {
        harness.update(cx, |root, cx| {
            root.active = index;
            root.views[index]
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            cx.notify();
        });
    });
    settle(cx);
}

fn reload(cx: &mut VisualTestContext, view: &Entity<FilePreview>, text: &str) {
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.sync(
                text.to_owned().into(),
                Arc::default(),
                view.wrap,
                view.typography.clone(),
                cx,
            );
        });
    });
}

#[gpui_kit::test]
fn file_find_ctrl_f_toggles_and_reopen_keeps_the_active_match(cx: &mut TestAppContext) {
    let source = format!(
        "opfile\n\n{}opfile\n\n{}open_workspace_file\n",
        "ordinary paragraph\n\n".repeat(30),
        "more paragraphs\n\n".repeat(30)
    );
    let (cx, harness, views) = fixture(cx, &[(false, &source), (true, &source)]);
    for (index, view) in views.iter().enumerate() {
        activate(cx, &harness, index);
        cx.simulate_keystrokes("ctrl-f");
        query(cx, view, "opfile");
        cx.simulate_keystrokes("enter");
        settle(cx);
        let before = cx.update(|_, cx| {
            let view = view.read(cx);
            assert_eq!(view.hits.len(), 3);
            assert_eq!(view.current, 1);
            view.scroll.offset()
        });
        cx.simulate_keystrokes("ctrl-f");
        settle(cx);
        cx.update(|window, cx| {
            let view = view.read(cx);
            assert!(!view.open, "Ctrl+F closes an open find bar");
            assert_eq!(view.current, 1, "closing must retain the active match");
            assert_eq!(view.scroll.offset(), before);
            assert!(view.focus.is_focused(window));
        });
        assert!(cx.debug_bounds("file-find-bar").is_none());
        assert!(!highlighted(cx));
        cx.simulate_keystrokes("ctrl-f");
        settle(cx);
        assert!(cx.debug_bounds("file-find-bar").is_some());
        cx.update(|window, cx| {
            assert!(
                view.read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            );
        });
        assert_eq!(
            cx.update(|_, cx| view.read(cx).current),
            1,
            "reopening must retain the active match"
        );
        assert!(highlighted(cx));
        cx.simulate_input("ordinary");
        settle(cx);
        assert_eq!(
            cx.update(|_, cx| view.read(cx).input.read(cx).value()),
            "ordinary",
            "Ctrl+F selects the complete query for replacement"
        );
        cx.update(|window, cx| window.click("file-find-close", cx));
        settle(cx);
        assert!(!view.read_with(cx, |view, _| view.open));
        assert!(!highlighted(cx));
    }
}

#[gpui_kit::test]
fn file_find_toggle_buttons_and_exact_whitespace(cx: &mut TestAppContext) {
    let (cx, _, views) = fixture(
        cx,
        &[(
            false,
            "OPFILE\nopen_workspace_file\nopfile\nxopfilex\n opfile \n",
        )],
    );
    let view = &views[0];
    cx.simulate_keystrokes("ctrl-f");
    settle(cx);
    assert!(cx.debug_bounds("file-find-count").is_none());
    assert!(cx.debug_bounds("focus-ring").is_none());
    let input_bounds = cx.debug_bounds("file-find-input").unwrap();
    cx.update(|window, _| {
        let toggle = window.find("file-find-fuzzy");
        assert!(
            input_bounds.contains(&toggle.bounds().center()),
            "fuzzy toggle belongs inside the input"
        );
    });
    query(cx, view, "opfile");
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 5);
    cx.update(|window, cx| window.click("file-find-fuzzy", cx));
    settle(cx);
    cx.update(|window, cx| {
        assert!(!view.read(cx).fuzzy);
        assert_eq!(view.read(cx).hits.len(), 4);
        assert!(
            view.read(cx)
                .input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
    });
    cx.update(|window, cx| window.click("file-find-previous", cx));
    settle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).current), 3);
    cx.update(|window, cx| window.click("file-find-next", cx));
    settle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).current), 0);
    query(cx, view, " opfile ");
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(
            view.hits.len(),
            1,
            "exact search preserves query whitespace"
        );
        assert_eq!(&view.source[view.hits[0].range.clone()], " opfile ");
    });
    for text in [
        "absent".to_owned(),
        " ".to_owned(),
        "文".repeat(super::search::MAX_QUERY_CHARS + 1),
        String::new(),
    ] {
        query(cx, view, &text);
        cx.update(|window, cx| {
            assert!(view.read(cx).hits.is_empty());
            assert!(!view.read(cx).searching);
            assert_eq!(window.find("file-find-previous").disabled(), Some(true));
            assert_eq!(window.find("file-find-next").disabled(), Some(true));
        });
        assert!(!highlighted(cx));
    }
    query(cx, view, "opfile");
    cx.update(|window, cx| window.click("file-find-fuzzy", cx));
    settle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 5);
}

#[gpui_kit::test]
fn file_find_rapid_queries_reload_and_close_cancel_old_results(cx: &mut TestAppContext) {
    let source = "alpha alpha\n\nbeta\n\ngamma gamma gamma\n";
    let (cx, harness, views) = fixture(cx, &[(false, source), (true, source)]);
    for (index, view) in views.iter().enumerate() {
        activate(cx, &harness, index);
        cx.simulate_keystrokes("ctrl-f");
        for text in ["alpha", "beta", "gamma"] {
            type_query(cx, view, text);
            cx.run_until_parked();
            assert!(cx.update(|_, cx| view.read(cx).searching));
        }
        settle(cx);
        assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 3);
        type_query(cx, view, "alpha");
        reload(cx, view, "reloaded alpha\n");
        settle(cx);
        assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
        type_query(cx, view, "reloaded");
        cx.run_until_parked();
        cx.simulate_keystrokes("ctrl-f");
        settle(cx);
        cx.update(|_, cx| {
            let view = view.read(cx);
            assert!(!view.open);
            assert!(!view.searching);
            assert!(view.hits.is_empty());
        });
        assert!(!highlighted(cx));
        cx.simulate_keystrokes("ctrl-f");
        settle(cx);
        assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
        assert!(highlighted(cx));
    }
}

#[gpui_kit::test]
fn file_find_inactive_reload_keeps_other_tabs_and_refreshes_on_return(cx: &mut TestAppContext) {
    let (cx, harness, views) = fixture(
        cx,
        &[
            (false, "needle\n\nneedle\n"),
            (true, "needle\n\nneedle\n"),
            (false, "other document\n"),
        ],
    );
    activate(cx, &harness, 2);
    cx.simulate_keystrokes("ctrl-f");
    query(cx, &views[2], "other");
    for (index, view) in views[..2].iter().enumerate() {
        activate(cx, &harness, index);
        cx.simulate_keystrokes("ctrl-f");
        query(cx, view, "needle");
        cx.simulate_keystrokes("enter");
        settle(cx);
        activate(cx, &harness, 2);
        reload(cx, view, "new needle\n\nneedle\n\nneedle\n");
        settle(cx);
        cx.update(|window, cx| {
            let other = views[2].read(cx);
            assert_eq!(other.input.read(cx).value(), "other");
            assert_eq!(other.hits.len(), 1);
            assert_eq!(other.current, 0);
            assert!(other.input.read(cx).focus_handle(cx).is_focused(window));
        });
        activate(cx, &harness, index);
        assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 3);
        assert!(highlighted(cx));
        cx.simulate_keystrokes("escape");
        activate(cx, &harness, 2);
        reload(cx, view, "needle after closing\n");
        settle(cx);
        activate(cx, &harness, index);
        assert!(!highlighted(cx));
        cx.simulate_keystrokes("ctrl-f");
        settle(cx);
        assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
        assert!(highlighted(cx));
    }
}

#[gpui_kit::test]
fn file_find_selection_prefill_and_input_copy_keep_focus(cx: &mut TestAppContext) {
    let (cx, _, views) = fixture(cx, &[(false, "chosen\nsecond\n")]);
    let view = &views[0];
    let source = cx.debug_bounds("file-source-view").unwrap();
    let position = source.origin + point(px(50.), px(12.));
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(gpui_kit::MouseDownEvent {
        position,
        button: MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    settle(cx);
    assert_eq!(cx.update(TextSelection::selected_text), "chosen");
    cx.simulate_keystrokes("ctrl-f");
    settle(cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).input.read(cx).value()),
        "chosen"
    );
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
    cx.simulate_input("second");
    settle(cx);
    cx.simulate_keystrokes("ctrl-a");
    settle(cx);
    cx.simulate_keystrokes("ctrl-c");
    cx.update(|window, cx| {
        let view = view.read(cx);
        assert_eq!(view.input.read(cx).value(), "second");
        assert!(view.input.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("second")
        );
    });
    cx.simulate_keystrokes("escape");
    settle(cx);
    let source = cx.debug_bounds("file-source-view").unwrap();
    let start = source.origin + point(px(39.), px(12.));
    let end = source.origin + point(px(190.), px(35.));
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    settle(cx);
    assert!(cx.update(TextSelection::selected_text).contains('\n'));
    cx.simulate_keystrokes("ctrl-f");
    settle(cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).input.read(cx).value()),
        "second",
        "multiline selection does not replace the query"
    );
}

#[gpui_kit::test]
fn file_find_source_shortcuts_wrap_reload_and_independent_tabs(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(init);
    let slot = Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let content = format!(
        "{}{}open_workspace_file\n{}opfile\n",
        "plain line\n".repeat(80),
        "prefix ".repeat(80),
        "more lines\n".repeat(80)
    );
    let (_, cx) = cx.add_window_view(|window, cx| {
        let mut views = Vec::new();
        for text in [content.as_str(), "other document second\n"] {
            let view = cx.new(|cx| {
                let mut view = FilePreview::new(false, ScrollHandle::new(), window, cx);
                let typography = view.typography.clone();
                view.sync(
                    text.to_owned().into(),
                    Arc::default(),
                    false,
                    typography,
                    cx,
                );
                view
            });
            views.push(view);
        }
        views[0].read(cx).focus.clone().focus(window, cx);
        let harness = cx.new(|_| Harness {
            views: views.clone(),
            active: 0,
        });
        *output.borrow_mut() = Some((harness.clone(), views));
        Root::new(harness, window, cx)
    });
    let (harness, views) = slot.borrow().as_ref().unwrap().clone();
    let view = &views[0];
    cx.simulate_resize(size(px(650.), px(400.)));
    settle(cx);
    cx.simulate_keystrokes("ctrl-f");
    settle(cx);
    assert!(cx.debug_bounds("file-find-bar").is_some());
    cx.simulate_input("opfile");
    settle(cx);
    cx.update(|window, cx| {
        let v = view.read(cx);
        assert_eq!(v.hits.len(), 2);
        assert_eq!(v.current, 0);
        assert!(v.scroll.offset().y < px(-500.));
        assert!(v.scroll.offset().x < px(0.));
        assert!(v.input.read(cx).focus_handle(cx).is_focused(window));
    });
    assert!(highlighted(cx), "active source match is painted");
    cx.simulate_keystrokes("enter");
    settle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).current), 1);
    cx.simulate_keystrokes("shift-f3");
    settle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).current), 0);
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.sync(
                v.source.clone(),
                v.highlights.clone(),
                true,
                v.typography.clone(),
                cx,
            );
        })
    });
    settle(cx);
    cx.simulate_resize(size(px(420.), px(400.)));
    settle(cx);
    cx.simulate_keystrokes("f3");
    cx.simulate_keystrokes("shift-enter");
    settle(cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).scroll.max_offset().x),
        px(0.)
    );
    assert!(highlighted(cx), "wrapped source match is still visible");
    // Close preserves scroll and copy selection is independent of find marks.
    let before = cx.update(|_, cx| view.read(cx).scroll.offset());
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(cx.debug_bounds("file-find-bar").is_none());
    assert_eq!(cx.update(|_, cx| view.read(cx).scroll.offset()), before);
    assert!(!highlighted(cx));
    cx.simulate_keystrokes("ctrl-f");
    settle(cx);
    cx.simulate_keystrokes("f3");
    settle(cx);
    cx.update(|window, cx| {
        harness.update(cx, |root, cx| {
            root.active = 1;
            cx.notify();
        });
        views[1].read(cx).focus_handle(cx).focus(window, cx);
    });
    settle(cx);
    cx.simulate_keystrokes("ctrl-f");
    query(cx, &views[1], "second");
    cx.update(|window, cx| {
        harness.update(cx, |root, cx| {
            root.active = 0;
            cx.notify();
        });
        view.read(cx).focus_handle(cx).focus(window, cx);
    });
    settle(cx);
    cx.update(|_, cx| {
        let v = view.read(cx);
        assert_eq!(v.input.read(cx).value(), "opfile");
        assert_eq!(v.current, 1);
        assert_eq!(views[1].read(cx).input.read(cx).value(), "second");
    });
    // Older asynchronous results cannot overwrite a newer query or reloaded file.
    query(cx, view, "missing");
    assert!(cx.update(|_, cx| view.read(cx).hits.is_empty()));
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.sync(
                "missing now exists".into(),
                Arc::default(),
                true,
                v.typography.clone(),
                cx,
            )
        })
    });
    settle(cx);
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
    query(cx, view, "");
    assert!(!highlighted(cx));
}

#[gpui_kit::test]
fn file_find_markdown_prose_numbered_code_and_diagram_reveal(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(init);
    let slot = Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let source = format!(
        "# Heading\n\n{}\n\n**Bold** &amp; label\n\n```rust\n{}\nlet numbered_target = 1;\n```\n\n{}\n\n```mermaid\nflowchart LR\nA[diagram_target] --> B[End]\n```\n",
        "ordinary paragraph\n\n".repeat(30),
        "// ordinary code\n".repeat(50),
        "more paragraphs\n\n".repeat(20)
    );
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            let mut view = FilePreview::new(true, ScrollHandle::new(), window, cx);
            let mut typography = view.typography.clone();
            typography.code_line_numbers = true;
            view.sync(source.into(), Arc::default(), true, typography, cx);
            view
        });
        view.read(cx).focus.clone().focus(window, cx);
        *output.borrow_mut() = Some(view.clone());
        let harness = cx.new(|_| Harness {
            views: vec![view],
            active: 0,
        });
        Root::new(harness, window, cx)
    });
    let view = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(size(px(650.), px(400.)));
    settle(cx);
    cx.simulate_keystrokes("ctrl-f");
    query(cx, &view, "bold & label");
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
    assert!(cx.update(|_, cx| view.read(cx).scroll.offset().y < px(-100.)));
    assert!(
        highlighted(cx),
        "Markdown search uses rendered text across emphasis and entities"
    );
    query(cx, &view, "numtarget");
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
    assert!(
        highlighted(cx),
        "custom numbered code highlights and reveals the exact line"
    );
    let before = cx.update(|_, cx| view.read(cx).scroll.offset().y);
    query(cx, &view, "diagram_target");
    assert_eq!(cx.update(|_, cx| view.read(cx).hits.len()), 1);
    let after = cx.update(|_, cx| view.read(cx).scroll.offset().y);
    assert!(
        after < before,
        "diagram reveal: before {before:?}, after {after:?}"
    );
    let diagram = cx.debug_bounds("mermaid-block").unwrap();
    let viewport = cx.debug_bounds("file-content-viewport").unwrap();
    assert!(diagram.top() < viewport.bottom() && diagram.bottom() > viewport.top());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).scroll.max_offset().x),
        px(0.)
    );
    query(cx, &view, "no_result");
    assert!(cx.update(|_, cx| view.read(cx).hits.is_empty()));
    assert!(!highlighted(cx));
}
