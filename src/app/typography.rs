//! Chat Markdown, code block controls, and font configuration.

use super::*;

/// Chat typography resolved from settings, clamped so a hand-edited
/// `state.json` cannot push the layout to unusable values.
#[derive(Clone)]
pub(super) struct ChatTypography {
    pub(super) body_font: Option<SharedString>,
    pub(super) body_size: f32,
    pub(super) line_height: f32,
    pub(super) code_font: SharedString,
    pub(super) code_size: f32,
    pub(super) code_copy: bool,
    pub(super) syntax_highlight: bool,
}

impl ChatTypography {
    /// `None` keeps the theme's code block style. A fixed, process-wide empty
    /// highlight theme disables coloring without re-parsing every frame.
    pub(super) fn text_view_style(&self) -> Option<TextViewStyle> {
        if self.syntax_highlight && !self.code_copy {
            return None;
        }
        let mut style = TextViewStyle::default();
        if !self.syntax_highlight {
            style.highlight_theme = plain_highlight_theme();
        }
        if self.code_copy {
            // The action row floats over the block's top-right corner.
            style.code_block = StyleRefinement::default().pt(px(34.));
        }
        Some(style)
    }
}

pub(super) fn plain_highlight_theme() -> std::sync::Arc<HighlightTheme> {
    static PLAIN: std::sync::OnceLock<std::sync::Arc<HighlightTheme>> = std::sync::OnceLock::new();
    PLAIN
        .get_or_init(|| {
            std::sync::Arc::new(HighlightTheme {
                name: "Conduit Plain".into(),
                appearance: gpui_kit::component::ThemeMode::Dark,
                style: Default::default(),
            })
        })
        .clone()
}

/// The corner row every chat code block gets: the language tag and a copy
/// button. Each block needs its own element id, otherwise all blocks share one
/// "已复制" state.
pub(super) fn code_block_actions(
    scope: SharedString,
) -> impl Fn(&gpui_kit::base::text::CodeBlock, &mut Window, &mut App) -> Div + Send + Sync + 'static
{
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    move |block, _, _| {
        let index = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        h_flex()
            .gap_1()
            .items_center()
            .when_some(block.lang(), |row, lang| {
                row.child(
                    div()
                        .text_size(px(10.))
                        .line_height(px(16.))
                        .text_color(rgb(palette().faint))
                        .child(lang),
                )
            })
            .child(
                Clipboard::new(SharedString::from(format!("{scope}-{index}")))
                    .tooltip("复制代码")
                    .accessibility_label("复制代码")
                    .value(block.code()),
            )
    }
}

/// A chat markdown view carrying the current typography: prose follows the body
/// settings, code blocks read the mono settings from the global theme.
pub(super) fn chat_markdown_view(
    id: SharedString,
    text: impl Into<SharedString>,
    typography: &ChatTypography,
    copy_scope: &str,
    selectable: bool,
) -> TextView {
    let mut view = TextView::markdown(id, text)
        .text_size(px(typography.body_size))
        .line_height(relative(typography.line_height))
        .selectable(selectable);
    if let Some(family) = typography.body_font.clone() {
        view = view.font_family(family);
    }
    if let Some(style) = typography.text_view_style() {
        view = view.style(style);
    }
    if typography.code_copy {
        view = view.code_block_actions(code_block_actions(copy_scope.into()));
    }
    view
}

/// Writes the code font into the global theme, which code blocks, inline code
/// and the Git diff view all read.
pub(super) fn install_code_font(
    cx: &mut App,
    family: Option<&str>,
    default_family: &SharedString,
    size: f32,
) {
    let family = family.map_or_else(|| default_family.clone(), SharedString::from);
    let size = px(size.clamp(MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE));
    gpui_kit::component::Theme::update(cx, move |theme| {
        theme.mono_font_family = family;
        theme.mono_font_size = size;
    });
}
