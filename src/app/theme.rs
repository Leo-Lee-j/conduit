//! Application palette and appearance synchronization.

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use std::rc::Rc;

/// Application roles projected from the selected component theme. The original
/// Conduit themes retain their palettes for existing installations.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Palette {
    pub(super) canvas: u32,
    pub(super) sidebar_bg: u32,
    pub(super) document_tab_bar_bg: u32,
    pub(super) surface: u32,
    pub(super) surface_hover: u32,
    pub(super) surface_strong: u32,
    pub(super) border: u32,
    pub(super) text: u32,
    pub(super) muted: u32,
    pub(super) faint: u32,
    pub(super) loading: u32,
    pub(super) activity_running: u32,
    pub(super) activity_completed: u32,
    pub(super) focus: u32,
    pub(super) on_text: u32,
    pub(super) highlight_text: u32,
    pub(super) highlight_bg: u32,
    pub(super) diff_add_text: u32,
    pub(super) diff_remove_text: u32,
    pub(super) inline_code_bg: u32,
    pub(super) git_added_text: u32,
    pub(super) git_modified_text: u32,
    pub(super) git_deleted_text: u32,
    pub(super) danger_border: u32,
    pub(super) danger_bg: u32,
    pub(super) danger_text: u32,
    pub(super) warning_border: u32,
    pub(super) warning_bg: u32,
    pub(super) warning_icon_bg: u32,
    pub(super) warning_text: u32,
    pub(super) warning_chip_bg: u32,
    pub(super) warning_chip_text: u32,
    pub(super) warning_muted: u32,
    pub(super) warning_inner_border: u32,
    pub(super) warning_code_bg: u32,
    pub(super) warning_code_text: u32,
    pub(super) warning_accent: u32,
    pub(super) error_text: u32,
}

const DARK_PALETTE: Palette = Palette {
    canvas: 0x0e0e10,
    sidebar_bg: 0x131315,
    document_tab_bar_bg: 0x1b1b1e,
    surface: 0x19191c,
    surface_hover: 0x232327,
    surface_strong: 0x202024,
    border: 0x29292e,
    text: 0xf3f3f4,
    muted: 0xa1a1a8,
    faint: 0x92929b,
    loading: 0x8fb8e6,
    activity_running: 0x60a5fa,
    activity_completed: 0x4ade80,
    focus: 0x8297b5,
    on_text: 0x111113,
    highlight_text: 0xf0d49b,
    highlight_bg: 0x3a3220,
    diff_add_text: 0x9fdcab,
    diff_remove_text: 0xe6a1a1,
    inline_code_bg: 0x33333b,
    git_added_text: 0x8fb8e6,
    git_modified_text: 0x9fdcab,
    git_deleted_text: 0xe6a1a1,
    danger_border: 0x5c332c,
    danger_bg: 0x2a1d1a,
    danger_text: 0xe6b4a8,
    warning_border: 0x4a3b22,
    warning_bg: 0x1d1a14,
    warning_icon_bg: 0x2a2317,
    warning_text: 0xe3b96b,
    warning_chip_bg: 0x332b1b,
    warning_chip_text: 0xd6b26b,
    warning_muted: 0x9c9284,
    warning_inner_border: 0x352c1b,
    warning_code_bg: 0x15130e,
    warning_code_text: 0xd9d3c7,
    warning_accent: 0xd6a75a,
    error_text: 0xe6a1a1,
};

const LIGHT_PALETTE: Palette = Palette {
    canvas: 0xf7f7f8,
    sidebar_bg: 0xf1f1f3,
    document_tab_bar_bg: 0xeaeaed,
    surface: 0xffffff,
    surface_hover: 0xe9e9ee,
    surface_strong: 0xf7f7f9,
    border: 0xe4e4e9,
    text: 0x1c1c20,
    muted: 0x5f5f68,
    faint: 0x666670,
    loading: 0x60a5df,
    activity_running: 0x2563eb,
    activity_completed: 0x15803d,
    focus: 0x5e789c,
    on_text: 0xffffff,
    highlight_text: 0x6b4a00,
    highlight_bg: 0xfff0bf,
    diff_add_text: 0x1e6631,
    diff_remove_text: 0xa43636,
    inline_code_bg: 0xe8e8ee,
    git_added_text: 0x2c5f9e,
    git_modified_text: 0x1e6631,
    git_deleted_text: 0xa43636,
    danger_border: 0xe7b8b1,
    danger_bg: 0xfdf1ef,
    danger_text: 0x8f2f29,
    warning_border: 0xe5cf9f,
    warning_bg: 0xfff8e9,
    warning_icon_bg: 0xf9eed2,
    warning_text: 0x8a5b00,
    warning_chip_bg: 0xf5e8c8,
    warning_chip_text: 0x785200,
    warning_muted: 0x8c7c5d,
    warning_inner_border: 0xe8d8b2,
    warning_code_bg: 0xf8f2e6,
    warning_code_text: 0x4b4336,
    warning_accent: 0x9a6a00,
    error_text: 0xb03b3b,
};

static DARK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
static BACKGROUND_VISIBLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub(super) fn background_visible() -> bool {
    BACKGROUND_VISIBLE.load(std::sync::atomic::Ordering::Relaxed)
}

pub(super) fn set_background_visible(visible: bool) -> bool {
    BACKGROUND_VISIBLE.swap(visible, std::sync::atomic::Ordering::Relaxed) != visible
}

/// Only large chrome surfaces opt in. Text, menus, code and terminal keep their colors.
pub(super) fn chrome_background(color: u32) -> gpui_kit::Rgba {
    let mut color = gpui_kit::rgb(color);
    if background_visible() {
        color.a = 0.35;
    }
    color
}

/// The palette every render path reads. Kept as runtime state so a theme pack
/// can later replace it without touching the call sites.
static ACTIVE_PALETTE: std::sync::RwLock<Palette> = std::sync::RwLock::new(DARK_PALETTE);

pub(super) fn palette() -> Palette {
    *ACTIVE_PALETTE.read().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn sidebar_session_text_color() -> u32 {
    let palette = palette();
    // Lift titles above metadata even in themes that share muted/faint colors.
    // Keep metadata readable instead of lowering its contrast against the sidebar.
    blend(palette.text, palette.muted, 0.25)
}

pub(super) fn is_dark_mode() -> bool {
    DARK_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn set_palette(mode: gpui_kit::component::ThemeMode) {
    DARK_MODE.store(mode.is_dark(), std::sync::atomic::Ordering::Relaxed);
    *ACTIVE_PALETTE.write().unwrap_or_else(|e| e.into_inner()) = palette_for_mode(mode);
}

pub(super) fn palette_for_mode(mode: gpui_kit::component::ThemeMode) -> Palette {
    if mode.is_dark() {
        DARK_PALETTE
    } else {
        LIGHT_PALETTE
    }
}

const THEME_PACKS: &[&str] = &[
    include_str!("../../assets/themes/conduit.json"),
    include_str!("../../assets/themes/adventure.json"),
    include_str!("../../assets/themes/alduin.json"),
    include_str!("../../assets/themes/asciinema.json"),
    include_str!("../../assets/themes/aurora.json"),
    include_str!("../../assets/themes/ayu.json"),
    include_str!("../../assets/themes/catppuccin.json"),
    include_str!("../../assets/themes/everforest.json"),
    include_str!("../../assets/themes/fahrenheit.json"),
    include_str!("../../assets/themes/flexoki.json"),
    include_str!("../../assets/themes/gruvbox.json"),
    include_str!("../../assets/themes/harper.json"),
    include_str!("../../assets/themes/hybrid.json"),
    include_str!("../../assets/themes/jellybeans.json"),
    include_str!("../../assets/themes/kibble.json"),
    include_str!("../../assets/themes/macos-classic.json"),
    include_str!("../../assets/themes/mellifluous.json"),
    include_str!("../../assets/themes/molokai.json"),
    include_str!("../../assets/themes/solarized.json"),
    include_str!("../../assets/themes/spaceduck.json"),
    include_str!("../../assets/themes/tokyonight.json"),
    include_str!("../../assets/themes/twilight.json"),
];

/// Embed the catalog so installed builds work offline and from any directory.
pub(crate) fn install_theme_packs(cx: &mut gpui_kit::App) {
    if ThemeRegistry::global(cx)
        .themes()
        .contains_key("Conduit Dark")
    {
        return;
    }
    for pack in THEME_PACKS {
        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(pack) {
            conduit::diagnostics::record_crash("bundled themes", &format!("{error}"));
            eprintln!(
                "{}",
                conduit::i18n::format(
                    "theme.failed_to_register_bundled_themes",
                    &[("error", format!("{}", error))]
                )
            );
        }
    }
    Theme::update(cx, |theme| {
        // Keep the 1px focus border without the library's extra 3px outer band.
        // Borderless controls retain an inset focus line. This app-level setting
        // survives Theme::change; the light/dark packs supply its color.
        theme.focus_ring = false;
    });
}

fn resolve_theme(name: &str, mode: ThemeMode, cx: &gpui_kit::App) -> Rc<ThemeConfig> {
    let registry = ThemeRegistry::global(cx);
    registry
        .themes()
        .get(name)
        .filter(|theme| theme.mode == mode)
        .or_else(|| {
            registry.themes().get(if mode.is_dark() {
                "Conduit Dark"
            } else {
                "Conduit Light"
            })
        })
        .unwrap_or_else(|| {
            if mode.is_dark() {
                registry.default_dark_theme()
            } else {
                registry.default_light_theme()
            }
        })
        .clone()
}

pub(crate) fn apply_theme(mode: ThemeMode, light: &str, dark: &str, cx: &mut gpui_kit::App) {
    let light = resolve_theme(light, ThemeMode::Light, cx);
    let dark = resolve_theme(dark, ThemeMode::Dark, cx);
    let current = Theme::global(cx);
    let changed = current.mode != mode
        || current.light_theme.name != light.name
        || current.dark_theme.name != dark.name;
    if changed {
        Theme::update(cx, |theme| {
            theme.light_theme = light;
            theme.dark_theme = dark;
        });
        Theme::change(mode, None, cx);
    }
    set_palette(mode);
    let current = Theme::global(cx);
    if !matches!(
        current.theme_name().as_ref(),
        "Conduit Light" | "Conduit Dark"
    ) {
        *ACTIVE_PALETTE.write().unwrap_or_else(|e| e.into_inner()) = palette_from_theme(current);
    }
}

pub(crate) fn apply_icon_size(size: crate::store::UiIconSize, cx: &mut gpui_kit::App) {
    Theme::update(cx, |theme| theme.icon_scale = size.scale());
}

pub(super) fn theme_preview(name: &str, mode: ThemeMode, cx: &gpui_kit::App) -> Palette {
    let config = resolve_theme(name, mode, cx);
    if matches!(config.name.as_ref(), "Conduit Light" | "Conduit Dark") {
        return palette_for_mode(mode);
    }
    let mut theme = Theme::default();
    theme.apply_config(&config);
    palette_from_theme(&theme)
}

fn blend(foreground: u32, background: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        let fg = ((foreground >> shift) & 255) as f32;
        let bg = ((background >> shift) & 255) as f32;
        (fg * alpha + bg * (1. - alpha)).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}

fn color_on(color: gpui_kit::Hsla, background: u32) -> u32 {
    let color: gpui_kit::Rgba = color.into();
    let rgb = ((color.r * 255.).round() as u32) << 16
        | ((color.g * 255.).round() as u32) << 8
        | (color.b * 255.).round() as u32;
    blend(rgb, background, color.a)
}

fn contrast(a: u32, b: u32) -> f32 {
    let luminance = |color: u32| {
        let channel = |shift: u32| {
            let value = ((color >> shift) & 255) as f32 / 255.;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    };
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn readable(color: u32, backgrounds: &[u32]) -> u32 {
    let minimum = |color| {
        backgrounds
            .iter()
            .map(|background| contrast(color, *background))
            .fold(f32::INFINITY, f32::min)
    };
    let target = if minimum(0xffffff) > minimum(0) {
        0xffffff
    } else {
        0
    };
    for step in 0..=20 {
        let candidate = blend(target, color, step as f32 / 20.);
        if minimum(candidate) >= 4.5 {
            return candidate;
        }
    }
    target
}

fn palette_from_theme(theme: &Theme) -> Palette {
    let canvas = color_on(theme.background, if theme.is_dark() { 0 } else { 0xffffff });
    let surface = color_on(theme.popover, canvas);
    let sidebar_bg = color_on(theme.tab_bar, canvas);
    // Some theme packs use the canvas color for the strip too. Give the
    // document tabs a subtle tonal step while preserving the theme's tint.
    let document_tab_bar_bg = if contrast(sidebar_bg, canvas) < 1.09 {
        let tone = if theme.is_dark() { 0xffffff } else { 0 };
        blend(tone, canvas, 0.06)
    } else {
        sidebar_bg
    };
    let surface_hover = color_on(theme.secondary_hover, canvas);
    let surface_strong = color_on(theme.group_box, canvas);
    let backgrounds = [canvas, sidebar_bg, surface, surface_hover, surface_strong];
    let text = readable(color_on(theme.foreground, canvas), &backgrounds);
    let muted = readable(color_on(theme.muted_foreground, canvas), &backgrounds);
    let primary = readable(color_on(theme.primary, canvas), &backgrounds);
    let success = readable(color_on(theme.success, canvas), &backgrounds);
    let danger = readable(color_on(theme.danger, canvas), &backgrounds);
    let warning = readable(color_on(theme.warning, canvas), &backgrounds);
    Palette {
        canvas,
        sidebar_bg,
        document_tab_bar_bg,
        surface,
        surface_hover,
        surface_strong,
        border: color_on(theme.border, canvas),
        text,
        muted,
        faint: muted,
        loading: primary,
        activity_running: primary,
        activity_completed: success,
        focus: color_on(theme.ring, canvas),
        on_text: canvas,
        highlight_text: warning,
        highlight_bg: blend(warning, canvas, 0.12),
        diff_add_text: success,
        diff_remove_text: danger,
        inline_code_bg: color_on(theme.secondary, canvas),
        git_added_text: success,
        git_modified_text: warning,
        git_deleted_text: danger,
        danger_border: blend(danger, canvas, 0.4),
        danger_bg: blend(danger, canvas, 0.08),
        danger_text: danger,
        warning_border: blend(warning, canvas, 0.4),
        warning_bg: blend(warning, canvas, 0.06),
        warning_icon_bg: blend(warning, canvas, 0.12),
        warning_text: warning,
        warning_chip_bg: blend(warning, canvas, 0.12),
        warning_chip_text: warning,
        warning_muted: muted,
        warning_inner_border: blend(warning, canvas, 0.25),
        warning_code_bg: canvas,
        warning_code_text: text,
        warning_accent: warning,
        error_text: danger,
    }
}

#[derive(Clone)]
pub(super) struct ThemeChoice {
    pub(super) name: gpui_kit::SharedString,
    colors: Palette,
}

impl gpui_kit::component::searchable_list::SearchableListItem for ThemeChoice {
    type Value = gpui_kit::SharedString;

    fn title(&self) -> gpui_kit::SharedString {
        self.name.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.name
    }

    fn render(
        &self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::App,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _, component::h_flex, div, px, rgb};
        h_flex()
            .gap_2()
            .child(
                h_flex()
                    .border_1()
                    .border_color(rgb(self.colors.border))
                    .rounded(px(3.))
                    .overflow_hidden()
                    .children(
                        [
                            self.colors.canvas,
                            self.colors.sidebar_bg,
                            self.colors.focus,
                        ]
                        .map(|color| div().w(px(10.)).h(px(16.)).bg(rgb(color))),
                    ),
            )
            .child(self.name.clone())
    }
}

pub(super) fn theme_choices(mode: ThemeMode, cx: &gpui_kit::App) -> Vec<ThemeChoice> {
    let default = if mode.is_dark() {
        "Conduit Dark"
    } else {
        "Conduit Light"
    };
    let mut choices: Vec<_> = ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|theme| theme.mode == mode && !theme.is_default)
        .map(|theme| ThemeChoice {
            name: theme.name.clone(),
            colors: theme_preview(&theme.name, mode, cx),
        })
        .collect();
    choices.sort_by_key(|theme| (theme.name.as_ref() != default, theme.name.to_lowercase()));
    choices
}
