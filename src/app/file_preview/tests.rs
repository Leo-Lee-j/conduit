use super::{FilePreview, init, match_color};
use gpui_kit::component::{Root, v_flex};
use gpui_kit::{
    AppContext as _, Background, Context, Entity, Focusable as _, IntoElement, ParentElement,
    Render, ScrollHandle, Styled, TestAppContext, VisualTestContext, Window, px, size,
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

fn query(cx: &mut VisualTestContext, view: &Entity<FilePreview>, text: &str) {
    cx.update(|window, cx| {
        let input = view.read(cx).input.clone();
        input.update(cx, |input, cx| input.set_value(text, window, cx));
    });
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
    assert!(cx.update(|_, cx| view.read(cx).scroll.offset().y < before));
    assert_eq!(
        cx.update(|_, cx| view.read(cx).scroll.max_offset().x),
        px(0.)
    );
    query(cx, &view, "no_result");
    assert!(cx.update(|_, cx| view.read(cx).hits.is_empty()));
    assert!(!highlighted(cx));
}
