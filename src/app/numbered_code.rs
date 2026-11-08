//! Fenced code blocks with a line-number gutter.
//!
//! The base text view draws every fenced block with its own chrome and no
//! gutter. When the setting is on, each fenced block is parsed into a custom
//! node drawn here instead: the same chrome, language tag and copy button, the
//! same syntax highlighting, and one number per logical line. The numbers live
//! in the block's left padding, outside the text run, so selection and copying
//! still return only the code.

use std::ops::Range;

use gpui_kit::{
    base::{
        TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun,
        text::{MarkdownNode, MarkdownParseContext, markdown_ast},
    },
    component::{Theme, text::TextView},
};

use super::*;

// `Edges` reaches this module through two globs (gpui's and gpui-base's
// serde-compatible twin), so pin the gpui one `Style::padding` expects.
use gpui_kit::Edges;

/// Node name shared by the parser and the renderer registration.
pub(super) const NUMBERED_CODE: &str = "conduit-numbered-code";

/// What one numbered block needs from the settings behind the markup.
pub(super) struct NumberedCodeConfig {
    pub(super) copy_scope: SharedString,
    pub(super) copy: bool,
    pub(super) syntax_highlight: bool,
    pub(super) selectable: bool,
    pub(super) search: Option<WeakEntity<super::file_preview::FilePreview>>,
}

/// Registers the parser and renderer that replace fenced code blocks.
pub(super) fn numbered_code_blocks(view: TextView, config: NumberedCodeConfig) -> TextView {
    let config = Arc::new(config);
    view.markdown_block_parser(parse_numbered_code)
        .markdown_block_renderer(NUMBERED_CODE, move |node, _, cx| {
            render_numbered_code(node, &config, cx)
        })
}

/// Parsed data for one fenced code block. Highlight names are captured during
/// parsing, which runs off the main thread, and resolved against the active
/// theme when the block renders.
struct NumberedCodeData {
    code: SharedString,
    lang: Option<SharedString>,
    highlights: Vec<(Range<usize>, &'static str)>,
}

fn parse_numbered_code(
    node: &markdown_ast::Node,
    cx: &MarkdownParseContext<'_>,
) -> Option<MarkdownNode> {
    let markdown_ast::Node::Code(code) = node else {
        return None;
    };
    if code.lang.as_deref().is_some_and(super::mermaid::is_mermaid) {
        return None;
    }
    let lang = code
        .lang
        .as_deref()
        .map(str::trim)
        .filter(|lang| !lang.is_empty())
        .map(SharedString::from);
    let highlights = match &lang {
        Some(lang) => super::code_highlight::highlight_names(lang, &code.value),
        None => Vec::new(),
    };
    let text = SharedString::from(code.value.as_str());
    Some(
        MarkdownNode::new(
            NUMBERED_CODE,
            NumberedCodeData {
                code: text.clone(),
                lang,
                highlights,
            },
        )
        .text(text)
        .markdown(cx.node_source(node).unwrap_or_default()),
    )
}

fn render_numbered_code(
    node: &MarkdownNode,
    config: &NumberedCodeConfig,
    cx: &mut App,
) -> AnyElement {
    let Some(data) = node.data::<NumberedCodeData>() else {
        return div().into_any_element();
    };
    let theme = Theme::global(cx);
    let radius = theme.semantic_tokens().radius.md;
    let background = theme.muted;
    let border = theme.border;
    let mono_family = theme.mono_font_family.clone();
    let mono_size = theme.mono_font_size;
    let sans_family = theme.font_family.clone();
    let xs_size = theme.semantic_tokens().typography.xs.size;
    let muted_foreground = theme.muted_foreground;
    let highlight_theme = theme.highlight_theme.clone();
    let highlights: Vec<(Range<usize>, HighlightStyle)> = if config.syntax_highlight {
        data.highlights
            .iter()
            .filter_map(|(range, name)| {
                highlight_theme
                    .style
                    .syntax
                    .style(name)
                    .map(|style| (range.clone(), style))
            })
            .collect()
    } else {
        Vec::new()
    };
    // The action row floats over the block's top-right corner, so the first
    // line starts below it. Without actions the language tag gets the same
    // treatment at a smaller height.
    let top = if config.copy {
        px(34.)
    } else if data.lang.is_some() {
        px(28.)
    } else {
        px(14.)
    };
    let start = node.source_range().map(|span| span.start).unwrap_or(0);
    let id = SharedString::from(format!("{}-lines-{start}", config.copy_scope));
    let code = NumberedCode {
        id: ElementId::Name(id.clone()),
        text: StyledText::new(data.code.clone()).with_highlights(highlights),
        code: data.code.clone(),
        top,
        selectable: config.selectable,
        search: config
            .search
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .and_then(|view| view.read(cx).block_search(node.source_range()?.clone())),
    };

    let actions = config.copy.then(|| {
        div()
            .absolute()
            .top_2()
            .right_2()
            .bg(background)
            .rounded(radius)
            .child(code_actions(
                id.clone(),
                data.lang.clone(),
                data.code.clone(),
            ))
    });
    let lang_label = data.lang.clone().filter(|_| !config.copy).map(|lang| {
        div()
            .absolute()
            .top(px(7.))
            .right_3()
            .font_family(sans_family)
            .text_size(xs_size)
            .line_height(px(16.))
            .text_color(muted_foreground)
            .child(lang)
    });

    div()
        .w_full()
        .min_w_0()
        .bg(background)
        .border_1()
        .border_color(border)
        .rounded(radius)
        .font_family(mono_family)
        .text_size(mono_size)
        .line_height(relative(1.6))
        .relative()
        .child(code)
        .children(actions)
        .children(lang_label)
        .into_any_element()
}

/// The language tag and copy button shared by numbered and stock blocks.
pub(super) fn code_actions(
    id: SharedString,
    lang: Option<SharedString>,
    code: SharedString,
) -> Div {
    h_flex()
        .gap_1()
        .items_center()
        .when_some(lang, |row, lang| {
            row.child(
                div()
                    .text_size(px(10.))
                    .line_height(px(16.))
                    .text_color(rgb(palette().faint))
                    .child(lang),
            )
        })
        .child(
            Clipboard::new(id)
                .tooltip(conduit::i18n::text_static("numbered_code.copy_code"))
                .accessibility_label(conduit::i18n::text_static("numbered_code.copy_code"))
                .value(code),
        )
}

/// Standalone read-only source viewer, sharing theme colors and real multi-line selection.
pub(super) fn file_code(
    id: u64,
    revision: u64,
    code: SharedString,
    names: &[(Range<usize>, &'static str)],
    wrap: bool,
    search: Option<super::file_preview::CodeSearch>,
    cx: &mut App,
) -> AnyElement {
    let theme = Theme::global(cx);
    let highlights: Vec<_> = names
        .iter()
        .filter_map(|(range, name)| {
            theme
                .highlight_theme
                .style
                .syntax
                .style(name)
                .map(|style| (range.clone(), style))
        })
        .collect();
    div()
        .id("file-source-view")
        .debug_selector(|| "file-source-view".into())
        .flex_shrink_0()
        .h_auto()
        .min_w_full()
        .when(wrap, |d| d.w_full())
        .when(!wrap, |d| d.whitespace_nowrap())
        .child(NumberedCode {
            id: SharedString::from(format!("file-source-{id}-{revision}")).into(),
            text: StyledText::new(code.clone()).with_highlights(highlights),
            code,
            top: px(8.),
            selectable: true,
            search,
        })
        .into_any_element()
}

/// The code text plus its gutter. One element owns the whole block so the
/// gutter and the text share the same measured line positions, and so the
/// block's selection participant covers exactly the code it paints.
struct NumberedCode {
    id: ElementId,
    text: StyledText,
    code: SharedString,
    top: Pixels,
    selectable: bool,
    search: Option<super::file_preview::CodeSearch>,
}

struct NumberedCodeState {
    selection: Option<TextSelectionHandle>,
    text_layout: LayoutId,
}

impl IntoElement for NumberedCode {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for NumberedCode {
    type RequestLayoutState = NumberedCodeState;
    type PrepaintState = Bounds<Pixels>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let global_id = global_id.expect("NumberedCode must have a stable element id");
        let selection = self.selectable.then(|| {
            window.with_element_state(
                global_id,
                |retained: Option<TextSelectionHandle>, window| {
                    let handle = retained.unwrap_or_else(|| {
                        let handle = TextSelectionHandle::new(self.code.clone(), cx);
                        // This custom node paints its own selection, outside TextView's
                        // selection adapter. Repaint when its snapshot changes, even
                        // when no other chat state or hover target changes.
                        handle.refresh_window_on_change(window, cx).detach();
                        handle
                    });
                    (handle.clone(), handle)
                },
            )
        });
        let text_layout = self
            .text
            .request_layout(Some(global_id), inspector_id, window, cx)
            .0;

        // Reserve room for the widest number the block will show, measured in
        // the same font the numbers are painted with.
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let digits = line_count(&self.code).to_string().len().max(1);
        let run = text_style.to_run(1);
        let digit_width = window
            .text_system()
            .shape_line("8".into(), font_size, &[run], None)
            .width;
        let gutter = digit_width * digits as f32 + NUMBER_GAP;
        let style = Style {
            padding: Edges {
                top: self.top.into(),
                right: px(16.).into(),
                bottom: px(14.).into(),
                left: (gutter + px(16.)).into(),
            },
            ..Default::default()
        };
        let layout = window.request_layout(style, [text_layout], cx);
        (
            layout,
            NumberedCodeState {
                selection,
                text_layout,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let text_bounds = window.layout_bounds(state.text_layout);
        self.text
            .prepaint(None, None, text_bounds, &mut (), window, cx);
        if let Some(search) = &self.search {
            search.reveal(self.text.layout(), window, cx);
        }
        if let Some(handle) = &state.selection {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            let registration = TextSelectionRegistration::new(hitbox, bounds)
                .with_text_bounds(vec![text_bounds])
                .with_rendered_element(handle, window, cx);
            handle.register(registration, window, cx);
        }
        text_bounds
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        text_bounds: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text_bounds = *text_bounds;
        let layout = self.text.layout().clone();

        if let Some(search) = &self.search {
            for (range, active) in &search.ranges {
                paint_selection(
                    &layout,
                    range.clone(),
                    super::file_preview::match_color(*active),
                    window,
                );
            }
        }
        if let Some(handle) = &state.selection {
            let selected_before = TextSelection::selected_text(window, cx);
            let projection = handle.update_runs(
                &[TextSelectionRun::new(
                    self.code.clone(),
                    layout.clone(),
                    text_bounds,
                )],
                cx,
            );
            if selected_before != TextSelection::selected_text(window, cx) {
                window.refresh();
            }
            let color = Theme::global(cx).selection;
            for range in projection.ranges().iter().flatten().cloned() {
                paint_selection(&layout, range, color, window);
            }
        }

        self.paint_numbers(&layout, text_bounds, window, cx);
        self.text
            .paint(None, None, text_bounds, &mut (), &mut (), window, cx);
    }
}

impl NumberedCode {
    fn paint_numbers(
        &self,
        layout: &TextLayout,
        text_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut text_style = window.text_style();
        text_style.color = Hsla::from(rgb(palette().faint));
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = layout.line_height();
        for (index, start) in line_starts(&self.code).enumerate() {
            let Some(position) = layout.position_for_index(start) else {
                continue;
            };
            let label = SharedString::from((index + 1).to_string());
            let run = text_style.to_run(label.len());
            let line = window
                .text_system()
                .shape_line(label, font_size, &[run], None);
            let origin = point(text_bounds.left() - NUMBER_GAP - line.width, position.y);
            _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
        }
    }
}

/// Space between the gutter numbers and the code text.
const NUMBER_GAP: Pixels = px(12.);

fn line_count(code: &str) -> usize {
    if code.is_empty() {
        0
    } else {
        code.bytes().filter(|byte| *byte == b'\n').count() + 1
    }
}

/// Byte offset where each logical line starts, including the empty line a
/// trailing newline leaves behind.
fn line_starts(code: &str) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(
        code.bytes()
            .enumerate()
            .filter(|(index, byte)| *byte == b'\n' && index + 1 < code.len())
            .map(|(index, _)| index + 1),
    )
}

fn paint_selection(layout: &TextLayout, range: Range<usize>, color: Hsla, window: &mut Window) {
    let (Some(start), Some(end)) = (
        layout.position_for_index(range.start),
        layout.position_for_index(range.end),
    ) else {
        return;
    };
    for bounds in selection_quad_bounds(start, end, layout.bounds(), layout.line_height()) {
        window.paint_quad(PaintQuad {
            bounds,
            background: color.into(),
            corner_radii: Corners::default(),
            border_widths: Edges::default(),
            border_color: transparent_black(),
            border_style: BorderStyle::default(),
        });
    }
}

/// Wrapped lines deserve a quad from the line start to the line end, so the
/// middle rows of a multi-line selection read as one continuous band.
fn selection_quad_bounds(
    start: Point<Pixels>,
    end: Point<Pixels>,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
) -> Vec<Bounds<Pixels>> {
    if start.y == end.y {
        return vec![Bounds::from_corners(
            start,
            Point::new(end.x, end.y + line_height),
        )];
    }

    let mut quads = vec![Bounds::from_corners(
        start,
        Point::new(bounds.right(), start.y + line_height),
    )];
    if end.y > start.y + line_height {
        quads.push(Bounds::from_corners(
            Point::new(bounds.left(), start.y + line_height),
            Point::new(bounds.right(), end.y),
        ));
    }
    quads.push(Bounds::from_corners(
        Point::new(bounds.left(), end.y),
        Point::new(end.x, end.y + line_height),
    ));
    quads
}
