//! Chat Markdown, code block controls, and font configuration.

use super::*;

/// Chat typography resolved from settings, clamped so a hand-edited
/// `state.json` cannot push the layout to unusable values.
#[derive(Clone, PartialEq)]
pub(super) struct ChatTypography {
    pub(super) body_font: Option<SharedString>,
    pub(super) body_size: f32,
    pub(super) line_height: f32,
    pub(super) code_font: SharedString,
    pub(super) code_size: f32,
    pub(super) code_copy: bool,
    pub(super) syntax_highlight: bool,
    pub(super) code_line_numbers: bool,
}

impl ChatTypography {
    /// Inline code always gets a distinct chip background and weight: the
    /// theme's accent sits too close to the chat canvas to read as code.
    pub(super) fn text_view_style(&self) -> TextViewStyle {
        let mut style = TextViewStyle {
            inline_code: HighlightStyle {
                background_color: Some(Hsla::from(rgb(palette().inline_code_bg))),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
            ..TextViewStyle::default()
        };
        if !self.syntax_highlight {
            style.highlight_theme = plain_highlight_theme();
        }
        if self.code_copy {
            // The action row floats over the block's top-right corner.
            style.code_block = StyleRefinement::default().pt(px(34.));
        }
        style
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
        super::numbered_code::code_actions(
            SharedString::from(format!("{scope}-{index}")),
            block.lang(),
            block.code(),
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
    styled_markdown_view(
        TextView::markdown(id, text),
        typography,
        copy_scope,
        selectable,
        None,
    )
}

pub(super) fn styled_markdown_view(
    view: TextView,
    typography: &ChatTypography,
    copy_scope: &str,
    selectable: bool,
    search: Option<WeakEntity<super::file_preview::FilePreview>>,
) -> TextView {
    let mut view = view
        .text_size(px(typography.body_size))
        .line_height(relative(typography.line_height))
        // Vertical padding switches the chip from its tight cap-height box to
        // the full ascender..descender extent, so the background stops hugging
        // the glyphs. The builder keeps this apart from the themed style.
        .inline_code_style(StyleRefinement::default().py(px(1.)))
        .selectable(selectable);
    if let Some(family) = typography.body_font.clone() {
        view = view.font_family(family);
    }
    view = super::mermaid::mermaid_blocks(
        view.style(typography.text_view_style()),
        copy_scope,
        selectable,
    );
    if typography.code_line_numbers {
        // The numbered renderer replaces fenced blocks wholesale, so it draws
        // its own action row and padding; the stock refinement above would
        // double-pad.
        view = super::numbered_code::numbered_code_blocks(
            view,
            super::numbered_code::NumberedCodeConfig {
                copy_scope: copy_scope.into(),
                copy: typography.code_copy,
                syntax_highlight: typography.syntax_highlight,
                selectable,
                search,
            },
        );
    } else if typography.code_copy {
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
