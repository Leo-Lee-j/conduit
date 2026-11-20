//! Independent interface/chat typography, Markdown, and code block controls.

use super::*;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

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
            paragraph_gap: rems(12. / DEFAULT_UI_FONT_SIZE),
            heading_base_font_size: px(self.body_size),
            heading_font_size: Some(Arc::new(|level, base| {
                base * match level {
                    1 => 20. / 15.,
                    2 => 17. / 15.,
                    _ => 1.,
                }
            })),
            table: StyleRefinement::default()
                .rounded(px(10.))
                .border_color(rgb(palette().border)),
            table_head: StyleRefinement::default()
                .bg(rgb(palette().surface_strong))
                .text_color(rgb(palette().muted))
                .text_size(px(self.body_size * 0.8)),
            table_cell: StyleRefinement::default().px(px(14.)).py(px(10.)),
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
        .blockquote_style(
            StyleRefinement::default()
                .border_l(px(3.))
                .border_color(rgb(palette().border))
                .pl(px(typography.body_size * 0.95))
                .text_color(rgb(palette().muted)),
        )
        .list_indent(px(typography.body_size * 1.6))
        .list_item_style(StyleRefinement::default().pb(px(typography.body_size * 0.2)))
        .table_body_column_style(
            0,
            StyleRefinement::default()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette().text))
                .text,
        )
        .link_style(file_link_style())
        .link_icon(file_link_icon())
        .link_underline_on_hover(true)
        // Vertical padding switches the chip from its tight cap-height box to
        // the full ascender..descender extent, so the background stops hugging
        // the glyphs. The builder keeps this apart from the themed style.
        .inline_code_style(
            StyleRefinement::default()
                .text_size(px(typography.body_size * 0.88))
                .px(px(5.))
                .py(px(1.))
                .border_1()
                .border_color(rgb(palette().border))
                .rounded(px(5.)),
        )
        .selectable(selectable);
    if let Some(family) = typography.body_font.clone() {
        view = view.font_family(family);
    }
    view = super::mermaid::mermaid_blocks(
        view.style(typography.text_view_style()),
        copy_scope,
        selectable,
        search.clone(),
    );
    super::numbered_code::numbered_code_blocks(
        view,
        super::numbered_code::NumberedCodeConfig {
            copy_scope: copy_scope.into(),
            copy: typography.code_copy,
            line_numbers: typography.code_line_numbers,
            syntax_highlight: typography.syntax_highlight,
            selectable,
            search,
        },
    )
}

fn file_link_icon() -> gpui_kit::base::text::LinkIconResolver {
    static ICON: std::sync::OnceLock<gpui_kit::base::text::LinkIconResolver> =
        std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        Arc::new(|url| super::file_links::is_file_link(url).then(|| FullIconName::FileText.path()))
    })
    .clone()
}

fn file_link_style() -> gpui_kit::base::text::LinkStyleResolver {
    static STYLE: std::sync::OnceLock<gpui_kit::base::text::LinkStyleResolver> =
        std::sync::OnceLock::new();
    STYLE
        .get_or_init(|| {
            Arc::new(|url| {
                super::file_links::is_file_link(url).then_some(HighlightStyle {
                    font_weight: Some(FontWeight::MEDIUM),
                    ..Default::default()
                })
            })
        })
        .clone()
}

/// Scales an interface text size with the base font, preserving its original
/// size at the default setting. Content text keeps using explicit pixel sizes.
pub(super) fn ui_text_size(design_size: f32) -> Rems {
    rems(design_size / DEFAULT_UI_FONT_SIZE)
}

/// Applies the interface font to every component and window through the theme.
/// Chat text explicitly resolves its own font before inheriting this theme.
pub(super) fn install_ui_font(
    cx: &mut App,
    family: Option<&str>,
    default_family: &SharedString,
    size: f32,
) {
    let family = family.map_or_else(|| default_family.clone(), SharedString::from);
    gpui_kit::component::Theme::update(cx, move |theme| {
        theme.font_family = family;
        theme.font_size = px(size.clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE));
    });
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
