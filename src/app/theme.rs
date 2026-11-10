//! Application palette and appearance synchronization.

use gpui_kit::component::{Theme, ThemeConfig, ThemeRegistry, ThemeSet};
use std::rc::Rc;

/// Application colors that are not owned by the component theme. The dark
/// values preserve the original palette; the light variant is derived from the
/// same roles so both modes stay consistent.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Palette {
    pub(super) canvas: u32,
    pub(super) sidebar_bg: u32,
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
    *ACTIVE_PALETTE.read().unwrap()
}

pub(super) fn is_dark_mode() -> bool {
    DARK_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

pub(crate) fn set_palette(mode: gpui_kit::component::ThemeMode) {
    DARK_MODE.store(mode.is_dark(), std::sync::atomic::Ordering::Relaxed);
    *ACTIVE_PALETTE.write().unwrap() = palette_for_mode(mode);
}

pub(super) fn palette_for_mode(mode: gpui_kit::component::ThemeMode) -> Palette {
    if mode.is_dark() {
        DARK_PALETTE
    } else {
        LIGHT_PALETTE
    }
}

const THEME_PACKS: &str = include_str!("../../assets/themes/conduit.json");

/// Installs the built-in theme packs as the light/dark themes, so
/// `Theme::change` reloads the pack instead of the library defaults. A pack
/// owns the shared input and focus colors; other tokens fall back to the library.
pub(crate) fn install_theme_packs(cx: &mut gpui_kit::App) {
    let packs = match serde_json::from_str::<ThemeSet>(THEME_PACKS) {
        Ok(set) => set.themes,
        Err(error) => {
            eprintln!(
                "{}",
                conduit::i18n::format(
                    "theme.failed_to_parse_bundled_themes",
                    &[("error", format!("{}", error))]
                )
            );
            return;
        }
    };
    let mut light: Option<Rc<ThemeConfig>> = None;
    let mut dark: Option<Rc<ThemeConfig>> = None;
    for pack in packs {
        let pack = Rc::new(pack);
        if pack.mode.is_dark() {
            dark = Some(pack);
        } else {
            light = Some(pack);
        }
    }
    // Register the packs so the registry's name-based reload resolves them
    // instead of dropping back to the library themes.
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(THEME_PACKS) {
        eprintln!(
            "{}",
            conduit::i18n::format(
                "theme.failed_to_register_bundled_themes",
                &[("error", format!("{}", error))]
            )
        );
    }
    Theme::update(cx, |theme| {
        // Keep the 1px focus border without the library's extra 3px outer band.
        // Borderless controls retain an inset focus line. This app-level setting
        // survives Theme::change; the light/dark packs supply its color.
        theme.focus_ring = false;
        if let Some(light) = light {
            theme.light_theme = light;
        }
        if let Some(dark) = dark {
            theme.dark_theme = dark;
        }
    });
}
