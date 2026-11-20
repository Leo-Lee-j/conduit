use super::{ChatTypography, styled_markdown_view};
use gpui_kit::base::TextSelection;
use gpui_kit::component::text::{RangeHighlight, TextView, TextViewState};
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Render, Styled, TestAppContext, VisualTestContext, Window, div, point, px,
};
use std::sync::{Arc, Mutex};

struct Fixture {
    state: Entity<TextViewState>,
    width: f32,
    clicks: Arc<Mutex<Vec<String>>>,
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
            code_line_numbers: false,
        };
        let clicks = self.clicks.clone();
        div()
            .id("markdown-fixture")
            .debug_selector(|| "markdown-fixture".into())
            .w(px(self.width))
            .child(
                styled_markdown_view(
                    TextView::new(&self.state),
                    &typography,
                    "markdown-test",
                    true,
                    None,
                )
                .on_link_click(move |url, _, _, _| clicks.lock().unwrap().push(url.to_string())),
            )
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

#[gpui_kit::test]
fn native_file_links_keep_partial_selection_source_and_clicks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    // Plain Inline, inline-code flow, and reference links take different rendering paths.
    for markdown in [
        "[src/中文/long_file_name.rs](src/main.rs#L12)",
        "[src/中文/long_file_name.rs](src/main.rs#L12) and `code`",
        "[src/中文/long_file_name.rs][file]\n\n[file]: src/main.rs#L12",
        "[`src/中文/long_file_name.rs`](src/main.rs#L12)",
        "**[src/中文/long_file_name.rs](src/main.rs#L12)**",
    ] {
        let state = cx.new(|cx| TextViewState::markdown(markdown, cx));
        let clicks = Arc::new(Mutex::new(Vec::new()));
        let (_, cx) = cx.add_window_view(|window, cx| {
            let fixture = cx.new(|_| Fixture {
                state: state.clone(),
                width: 130.,
                clicks: clicks.clone(),
            });
            gpui_kit::component::Root::new(fixture, window, cx)
        });
        settle(cx);
        let bounds = cx.debug_bounds("markdown-fixture").unwrap();
        let icon = cx
            .debug_bounds("markdown-link-icon")
            .expect("file link decoration");
        assert!(icon.size.width > px(0.));
        let start = bounds.origin + point(px(25.), px(11.));
        let end = start + point(px(65.), px(0.));
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        settle(cx);
        let selection = state.read_with(cx, |state, _| {
            // Plain selections include the paragraph separator; the Copy handler
            // trims it before placing the selected characters on the clipboard.
            let selected = state.selected_text().trim_end_matches('\n').to_owned();
            assert!(
                selected.chars().count() >= 2,
                "partial selection: {selected:?}"
            );
            let source = state.selected_source_range().unwrap();
            assert_eq!(&markdown[source], selected);
            assert!(!selected.contains("#L12"));
            selected
        });
        assert!(
            clicks.lock().unwrap().is_empty(),
            "dragging must not open a link"
        );
        cx.update(|window, cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into()));
            window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx);
        });
        assert_eq!(
            cx.update(|_, cx| cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .unwrap()),
            selection,
        );
        cx.update(TextSelection::clear);
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
        assert_eq!(*clicks.lock().unwrap(), vec!["src/main.rs#L12"]);
        clicks.lock().unwrap().clear();
        cx.update(TextSelection::clear);
        settle(cx);
        cx.simulate_mouse_move(icon.center(), None, Modifiers::default());
        cx.simulate_mouse_down(icon.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(icon.center(), MouseButton::Left, Modifiers::default());
        assert_eq!(
            *clicks.lock().unwrap(),
            vec!["src/main.rs#L12"],
            "icon click"
        );
        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn styled_nested_blocks_keep_copy_search_and_streaming_offsets(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let markdown = "> 引用 **中文**\n>\n> 3. 第三项\n>    - [x] 子任务\n>    - [ ] 待办\n\n| 文件 | 描述 |\n| --- | --- |\n| [main.rs](src/main.rs) | 中文长内容 |\n";
    let state = cx.new(|cx| TextViewState::markdown(markdown, cx));
    let (root, cx) = cx.add_window_view(|window, cx| {
        let fixture = cx.new(|_| Fixture {
            state: state.clone(),
            width: 180.,
            clicks: Arc::default(),
        });
        gpui_kit::component::Root::new(fixture, window, cx)
    });
    settle(cx);
    state.update(cx, |state, cx| {
        let rendered = state.rendered_text();
        let source_start = markdown.find("中文长内容").unwrap();
        let range = rendered
            .range_for_source(source_start..source_start + "中文长内容".len())
            .unwrap();
        assert_eq!(&rendered.as_str()[range.clone()], "中文长内容");
        state
            .set_range_highlights([RangeHighlight::new(range, gpui_kit::yellow())], cx)
            .unwrap();
        state.select_all(cx);
        assert_eq!(state.selected_source_range(), Some(0..markdown.len()));
        let copied = state.selected_text();
        for text in [
            "引用 中文",
            "第三项",
            "子任务",
            "待办",
            "main.rs",
            "中文长内容",
        ] {
            assert!(copied.contains(text), "missing {text}: {copied}");
        }
    });
    settle(cx);
    // Appending a row must keep the old source positions and allow new search ranges.
    state.update(cx, |state, cx| state.push_str("| next.rs | 新内容 |\n", cx));
    settle(cx);
    state.read_with(cx, |state, _| {
        let rendered = state.rendered_text();
        assert!(rendered.as_str().contains("新内容"));
        let start = markdown.find("main.rs").unwrap();
        let range = rendered.range_for_source(start..start + 7).unwrap();
        assert_eq!(&rendered.as_str()[range], "main.rs");
    });
    drop(root);
}

#[test]
fn file_decoration_uses_navigation_classification() {
    let style = super::file_link_style();
    let icon = super::file_link_icon();
    for url in [
        "src/main.rs#L12",
        "../src/main.rs#L12",
        "file:///C:/work/main.rs",
        "./中文.md:3",
        "C:/work/main.rs:12:4",
    ] {
        assert!(style(url).is_some(), "{url}");
        assert!(icon(url).is_some(), "{url}");
    }
    for url in [
        "https://example.com/main.rs",
        "mailto:dev@example.com",
        "#section",
        "ssh://server/file",
        "",
        "%ZZ.md",
    ] {
        assert!(style(url).is_none(), "{url}");
        assert!(icon(url).is_none(), "{url}");
    }
    assert!(Arc::ptr_eq(&style, &super::file_link_style()));
    assert!(Arc::ptr_eq(&icon, &super::file_link_icon()));
}

#[gpui_kit::test]
fn file_link_icons_follow_streamed_references_and_preserve_copy(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let initial = "[中文文件名与很长的路径标签][file]";
    let definition = "\n\n[file]: src/main.rs#L12";
    let state = cx.new(|cx| TextViewState::markdown(initial, cx));
    let clicks = Arc::new(Mutex::new(Vec::new()));
    let (_, cx) = cx.add_window_view(|window, cx| {
        let fixture = cx.new(|_| Fixture {
            state: state.clone(),
            width: 95.,
            clicks: clicks.clone(),
        });
        gpui_kit::component::Root::new(fixture, window, cx)
    });
    settle(cx);
    assert!(cx.debug_bounds("markdown-link-icon").is_none());
    state.update(cx, |state, cx| state.push_str(definition, cx));
    settle(cx);
    let icon = cx.debug_bounds("markdown-link-icon").unwrap();
    let bounds = cx.debug_bounds("markdown-fixture").unwrap();
    assert!(bounds.size.height > px(30.), "long label must wrap");
    assert!(icon.right() <= bounds.right());
    let rendered_before = state.read_with(cx, |state, _| state.rendered_text().as_str().to_owned());
    cx.update(|window, cx| {
        let focus = state.read(cx).focus_handle().clone();
        window.focus(&focus, cx);
    });
    state.update(cx, |state, cx| {
        state.select_all(cx);
        assert_eq!(state.selected_text().trim(), "中文文件名与很长的路径标签");
        assert_eq!(
            state.selected_source_range(),
            Some(0..initial.len() + definition.len())
        );
        let source_start = initial.find("很长").unwrap();
        let rendered = state.rendered_text();
        let range = rendered
            .range_for_source(source_start..source_start + "很长".len())
            .unwrap();
        assert_eq!(&rendered.as_str()[range], "很长");
    });
    settle(cx);
    cx.update(|window, cx| window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx));
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
            .trim(),
        "中文文件名与很长的路径标签"
    );
    cx.update(TextSelection::clear);
    settle(cx);
    // Drag from the icon across wrapped label rows: decoration must neither
    // navigate nor inject an object marker into the selected/source text.
    let end = bounds.bottom_right() - point(px(2.), px(5.));
    cx.simulate_mouse_move(icon.center(), None, Modifiers::default());
    cx.simulate_mouse_down(icon.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    settle(cx);
    assert!(clicks.lock().unwrap().is_empty());
    state.read_with(cx, |state, _| {
        let selected = state.selected_text();
        assert!(selected.contains("中文"), "{selected:?}");
        assert!(!selected.contains('\u{fffc}'));
        assert_eq!(state.rendered_text().as_str(), rendered_before);
    });
    state.update(cx, |state, cx| {
        state.set_text("[外部文档](https://example.com/guide)", cx)
    });
    settle(cx);
    assert!(
        cx.debug_bounds("markdown-link-icon").is_none(),
        "replacement removes decoration"
    );
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}
