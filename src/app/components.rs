//! Shared icons, message actions, and menu controls.

use super::*;

/// A small, steady layout slot; only the running dot's opacity breathes.
pub(super) fn session_activity_dot(id: u64, running: bool) -> AnyElement {
    let label = if running {
        "正在执行"
    } else {
        "回复完成，未读"
    };
    let dot = div().size(px(7.)).rounded_full().bg(rgb(if running {
        palette().activity_running
    } else {
        palette().activity_completed
    }));
    let dot = if running {
        dot.with_animation(
            ("session-pulse", id),
            Animation::new(Duration::from_millis(1600)).repeat(),
            |dot, delta| {
                dot.opacity(0.4 + 0.6 * (0.5 + 0.5 * (delta * std::f32::consts::TAU).cos()))
            },
        )
        .into_any_element()
    } else {
        dot.into_any_element()
    };
    h_flex()
        .id(("session-activity", id))
        .debug_selector(move || {
            format!(
                "session-activity-{id}-{}",
                if running { "running" } else { "unread" }
            )
        })
        .size(px(14.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
        })
        .child(dot)
        .into_any_element()
}

/// Engine and model marks compiled into the binary; provenance and licensing
/// notes live in `assets/icons/README.md`.
pub(super) const CLAUDE_ACCENT: u32 = 0xd97757;
pub(super) const MARK_CLAUDE: &[u8] = include_bytes!("../../assets/icons/claude.svg");
pub(super) const MARK_OPENAI: &[u8] = include_bytes!("../../assets/icons/openai.svg");
pub(super) const MARK_GROK: &[u8] = include_bytes!("../../assets/icons/grok.svg");
pub(super) const MARK_DEEPSEEK: &[u8] = include_bytes!("../../assets/icons/deepseek.svg");
pub(super) const MARK_QWEN: &[u8] = include_bytes!("../../assets/icons/qwen.svg");
pub(super) const MARK_KIMI: &[u8] = include_bytes!("../../assets/icons/kimi.svg");
pub(super) const MARK_ZHIPU: &[u8] = include_bytes!("../../assets/icons/zhipu.svg");
pub(super) const MARK_MINIMAX: &[u8] = include_bytes!("../../assets/icons/minimax.svg");
pub(super) const MARK_DOUBAO: &[u8] = include_bytes!("../../assets/icons/doubao.svg");
pub(super) const MARK_HUNYUAN: &[u8] = include_bytes!("../../assets/icons/hunyuan.svg");
pub(super) const MARK_GEMINI: &[u8] = include_bytes!("../../assets/icons/gemini.svg");

pub(super) fn provider_icon(provider: Provider) -> Icon {
    match provider {
        Provider::Claude => svg_mark(MARK_CLAUDE, CLAUDE_ACCENT),
        Provider::Codex => svg_mark(MARK_OPENAI, palette().text),
    }
}

/// Vendor mark for a model ID. Marks stay monochrome so they read in both
/// themes; IDs whose vendor has no local mark keep the engine mark rather
/// than borrowing an unrelated brand.
pub(super) fn model_icon(provider: Provider, id: &str) -> Icon {
    let neutral = palette().text;
    match models::brand_of(id) {
        models::ModelBrand::Anthropic => svg_mark(MARK_CLAUDE, CLAUDE_ACCENT),
        models::ModelBrand::OpenAI => svg_mark(MARK_OPENAI, neutral),
        models::ModelBrand::Xai => svg_mark(MARK_GROK, neutral),
        models::ModelBrand::DeepSeek => svg_mark(MARK_DEEPSEEK, neutral),
        models::ModelBrand::Qwen => svg_mark(MARK_QWEN, neutral),
        models::ModelBrand::Kimi => svg_mark(MARK_KIMI, neutral),
        models::ModelBrand::Zhipu => svg_mark(MARK_ZHIPU, neutral),
        models::ModelBrand::MiniMax => svg_mark(MARK_MINIMAX, neutral),
        models::ModelBrand::Doubao => svg_mark(MARK_DOUBAO, neutral),
        models::ModelBrand::Hunyuan => svg_mark(MARK_HUNYUAN, neutral),
        models::ModelBrand::Gemini => svg_mark(MARK_GEMINI, neutral),
        models::ModelBrand::Other => provider_icon(provider),
    }
}

/// Mark for a picker or chip that may still follow the CLI default: without a
/// model override there is no vendor to show, only the engine.
pub(super) fn model_mark(provider: Provider, id: Option<&str>) -> Icon {
    match id {
        Some(id) => model_icon(provider, id),
        None => provider_icon(provider),
    }
}

pub(super) fn svg_mark(data: &'static [u8], color: u32) -> Icon {
    Icon::default()
        .data(data)
        .size(px(14.))
        .text_color(rgb(color))
}

pub(super) fn message_action_button(id: SharedString, icon: Icon, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .compact()
        .icon(icon)
        .tooltip(tooltip)
        .accessibility_label(tooltip)
        .cursor_pointer()
}

/// Menu popovers render only compact buttons, so the default `p_3()` popover
/// padding (12px on every side) makes the menu look sparse. This refinement is
/// applied through `Popover`'s own `Styled` impl, which `refine_style`s the
/// content container after the default padding, overriding it.
pub(super) fn menu_popover_style() -> StyleRefinement {
    StyleRefinement::default().p_1().style().clone()
}

pub(super) fn session_menu_button(
    id: (&'static str, u64),
    label: &'static str,
    icon: IconName,
) -> Button {
    Button::new(id)
        .ghost()
        .h(px(30.))
        .w_full()
        .px_2()
        .rounded(px(6.))
        .cursor_pointer()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).small().text_color(rgb(palette().faint)))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().muted))
                        .child(label),
                ),
        )
}
