use super::{ChatTypography, Theme, chat_markdown_view};
use gpui_kit::base::TextSelection;
use gpui_kit::{
    AppContext, ClipboardItem, Context, IntoElement, Modifiers, MouseButton, ParentElement, Pixels,
    Point, Render, SharedString, Styled, TestAppContext, VisualTestContext, Window, div, point, px,
};

struct Fixture {
    markdown: SharedString,
    numbered: bool,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let typography = ChatTypography {
            body_font: None,
            body_size: 15.,
            line_height: 1.5,
            code_font: "Consolas".into(),
            code_size: 13.,
            code_copy: true,
            syntax_highlight: true,
            code_line_numbers: self.numbered,
        };
        div().w(px(360.)).child(chat_markdown_view(
            "code-fixture".into(),
            self.markdown.clone(),
            &typography,
            "test-code",
            true,
        ))
    }
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        });
    }
}

fn click(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
}

fn copied(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    })
}

#[gpui_kit::test]
fn fenced_blocks_copy_and_select_only_code_with_or_without_line_numbers(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for numbered in [false, true] {
        let first = "let first = 1;\n// 中文";
        let second = "second block";
        let markdown = format!("```rust\n{first}\n```\n\n```\n{second}\n```");
        let second_offset = markdown.find("```\nsecond").unwrap();
        // The GPUI debug selector helper requires a static string. This fixed
        // fixture allocates one selector per test window, not in production.
        let second_selector =
            Box::leak(format!("test-code-lines-{second_offset}-block").into_boxed_str());
        let (_, cx) = cx.add_window_view(|window, cx| {
            let fixture = cx.new(|_| Fixture {
                markdown: markdown.into(),
                numbered,
            });
            gpui_kit::component::Root::new(fixture, window, cx)
        });
        settle(cx);
        let first_bounds = cx.debug_bounds("test-code-lines-0-block").unwrap();
        let second_bounds = cx.debug_bounds(second_selector).unwrap();

        // Clicking one block must not suppress the other block's copy action.
        for (bounds, expected) in [(first_bounds, first), (second_bounds, second)] {
            click(cx, point(bounds.right() - px(24.), bounds.top() + px(17.)));
            assert_eq!(copied(cx), expected);
        }

        let line_height = cx.update(|_, cx| Theme::global(cx).mono_font_size * 1.6);
        let start = point(
            first_bounds.left() + px(if numbered { 39. } else { 17. }),
            first_bounds.top() + px(52.),
        );
        let end = point(first_bounds.right() - px(3.), start.y + line_height);
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        settle(cx);
        assert_eq!(cx.update(TextSelection::selected_text), first);
        cx.update(|window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("sentinel".into()));
            window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx);
        });
        assert_eq!(copied(cx), first);
        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn streaming_code_updates_copy_content_and_preserves_mermaid_routing(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let fixture = cx.new(|_| Fixture {
            markdown: "```rust\nbefore".into(),
            numbered: false,
        });
        *output.borrow_mut() = Some(fixture.clone());
        gpui_kit::component::Root::new(fixture, window, cx)
    });
    let fixture = slot.borrow().as_ref().unwrap().clone();
    settle(cx);
    let bounds = cx.debug_bounds("test-code-lines-0-block").unwrap();
    click(cx, point(bounds.right() - px(24.), bounds.top() + px(17.)));
    assert_eq!(copied(cx), "before");
    cx.update(|_, cx| {
        fixture.update(cx, |fixture, cx| {
            fixture.markdown = "```rust\nafter 中文\n```\n\n```mermaid\ngraph TD\n A-->B".into();
            fixture.numbered = true;
            cx.notify();
        })
    });
    cx.background_executor
        .advance_clock(std::time::Duration::from_secs(3));
    settle(cx);
    assert!(cx.debug_bounds("mermaid-block").is_some());
    let bounds = cx.debug_bounds("test-code-lines-0-block").unwrap();
    click(cx, point(bounds.right() - px(24.), bounds.top() + px(17.)));
    assert_eq!(copied(cx), "after 中文");
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}
