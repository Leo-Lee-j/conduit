//! Shared icons, message actions, and menu controls.

use super::*;

pub(super) fn app_icon(size: f32) -> Img {
    static ICON: std::sync::LazyLock<Arc<Image>> = std::sync::LazyLock::new(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/app-icons/128x128@2x.png").to_vec(),
        ))
    });
    img(ICON.clone())
        .size(px(size))
        .flex_shrink_0()
        .object_fit(ObjectFit::Contain)
}

/// Share the same status appearance between sidebar and tabs, with separate
/// element/animation keys so both can be mounted for the same session.
pub(super) fn session_activity_dot(scope: &'static str, id: u64, running: bool) -> AnyElement {
    let label = if running {
        "正在执行"
    } else {
        "回复完成，未读"
    };
    let dot = div()
        .size(px(7.))
        .rounded_full()
        .bg(rgb(if running {
            palette().activity_running
        } else {
            palette().activity_completed
        }))
        .with_animation(
            SharedString::from(format!("{scope}-pulse-{id}")),
            Animation::new(Duration::from_millis(if running { 1600 } else { 2400 })).repeat(),
            |dot, delta| {
                dot.opacity(0.4 + 0.6 * (0.5 + 0.5 * (delta * std::f32::consts::TAU).cos()))
            },
        );
    h_flex()
        .id((scope, id))
        .debug_selector(move || {
            format!(
                "{scope}-{id}-{}",
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
const BRAND_SVGS: &[(&str, &[u8])] = &[
    ("claude", include_bytes!("../../assets/icons/claude.svg")),
    (
        "claudecode",
        include_bytes!("../../assets/icons/claudecode.svg"),
    ),
    ("codex", include_bytes!("../../assets/icons/codex.svg")),
    (
        "opencode",
        include_bytes!("../../assets/icons/opencode.svg"),
    ),
    ("openai", include_bytes!("../../assets/icons/openai.svg")),
    ("grok", include_bytes!("../../assets/icons/grok.svg")),
    (
        "deepseek",
        include_bytes!("../../assets/icons/deepseek.svg"),
    ),
    ("qwen", include_bytes!("../../assets/icons/qwen.svg")),
    ("kimi", include_bytes!("../../assets/icons/kimi.svg")),
    ("zhipu", include_bytes!("../../assets/icons/zhipu.svg")),
    ("minimax", include_bytes!("../../assets/icons/minimax.svg")),
    ("doubao", include_bytes!("../../assets/icons/doubao.svg")),
    ("hunyuan", include_bytes!("../../assets/icons/hunyuan.svg")),
    ("gemini", include_bytes!("../../assets/icons/gemini.svg")),
    ("stepfun", include_bytes!("../../assets/icons/stepfun.svg")),
    ("mistral", include_bytes!("../../assets/icons/mistral.svg")),
    ("meta", include_bytes!("../../assets/icons/meta.svg")),
    ("cohere", include_bytes!("../../assets/icons/cohere.svg")),
    ("gemma", include_bytes!("../../assets/icons/gemma.svg")),
    ("nvidia", include_bytes!("../../assets/icons/nvidia.svg")),
    (
        "perplexity",
        include_bytes!("../../assets/icons/perplexity.svg"),
    ),
    ("ai21", include_bytes!("../../assets/icons/ai21.svg")),
    (
        "baichuan",
        include_bytes!("../../assets/icons/baichuan.svg"),
    ),
    ("yi", include_bytes!("../../assets/icons/yi.svg")),
    ("spark", include_bytes!("../../assets/icons/spark.svg")),
    ("nova", include_bytes!("../../assets/icons/nova.svg")),
    ("wenxin", include_bytes!("../../assets/icons/wenxin.svg")),
];

// GPUI's Icon/Svg paints an alpha mask in text_color and loses the original
// colors. ImageFormat::Svg preserves fills/gradients. Cache both currentColor
// variants once; fixed brand colors are never tinted by surrounding controls.
static BRAND_IMAGES: std::sync::LazyLock<HashMap<(&str, bool), Arc<Image>>> =
    std::sync::LazyLock::new(|| {
        BRAND_SVGS
            .iter()
            .flat_map(|(name, bytes)| {
                [false, true].map(|dark| {
                    let ink = if dark { "#F3F3F4" } else { "#17171B" };
                    let svg = std::str::from_utf8(bytes).expect("embedded brand SVG");
                    (
                        (*name, dark),
                        Arc::new(Image::from_bytes(
                            ImageFormat::Svg,
                            svg.replace("currentColor", ink)
                                .replace("\"1em\"", "\"64\"")
                                .into_bytes(),
                        )),
                    )
                })
            })
            .collect()
    });

fn brand_image(name: &'static str) -> Img {
    let dark = is_dark_mode();
    img(BRAND_IMAGES[&(name, dark)].clone())
        .size(px(14.))
        .flex_shrink_0()
        .object_fit(ObjectFit::Contain)
}

pub(super) fn provider_icon(provider: Provider) -> Img {
    brand_image(match provider {
        Provider::Claude => "claudecode",
        Provider::Codex => "codex",
        Provider::OpenCode => "opencode",
    })
}

/// Model family wins over the hosting provider, including routed model IDs.
pub(super) fn model_icon(provider: Provider, id: &str) -> Img {
    use models::ModelBrand;
    let name = match models::brand_of(id) {
        ModelBrand::Anthropic => "claude",
        ModelBrand::OpenAI => "openai",
        ModelBrand::Xai => "grok",
        ModelBrand::DeepSeek => "deepseek",
        ModelBrand::Qwen => "qwen",
        ModelBrand::Kimi => "kimi",
        ModelBrand::Zhipu => "zhipu",
        ModelBrand::MiniMax => "minimax",
        ModelBrand::Doubao => "doubao",
        ModelBrand::Hunyuan => "hunyuan",
        ModelBrand::Gemini => "gemini",
        ModelBrand::Step => "stepfun",
        ModelBrand::Mistral => "mistral",
        ModelBrand::Meta => "meta",
        ModelBrand::Cohere => "cohere",
        ModelBrand::Gemma => "gemma",
        ModelBrand::Nvidia => "nvidia",
        ModelBrand::Perplexity => "perplexity",
        ModelBrand::Ai21 => "ai21",
        ModelBrand::Baichuan => "baichuan",
        ModelBrand::Yi => "yi",
        ModelBrand::Spark => "spark",
        ModelBrand::Nova => "nova",
        ModelBrand::Wenxin => "wenxin",
        ModelBrand::Other => return provider_icon(provider),
    };
    brand_image(name)
}

/// Mark for a picker or chip that may still follow the CLI default: without a
/// model override there is no vendor to show, only the engine.
pub(super) fn model_mark(provider: Provider, id: Option<&str>) -> Img {
    match id {
        Some(id) => model_icon(provider, id),
        None => provider_icon(provider),
    }
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

#[cfg(test)]
mod brand_tests {
    use super::BRAND_IMAGES;
    use gpui_kit::SvgRenderer;
    use std::sync::Arc;

    #[test]
    fn embedded_brand_images_render_in_both_themes_and_keep_colors() {
        for (&(name, dark), image) in BRAND_IMAGES.iter() {
            let rendered = image
                .to_image_data(SvgRenderer::new(Arc::new(gpui_kit::assets::AllAssets)))
                .unwrap_or_else(|error| panic!("{name} dark={dark}: {error}"));
            let pixels = rendered.as_bytes(0).unwrap();
            assert!(
                pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 200),
                "empty {name}"
            );
            if !matches!(name, "openai" | "opencode" | "grok") {
                assert!(
                    pixels.as_chunks::<4>().0.iter().any(|p| {
                        p[3] > 200 && p[0].abs_diff(p[1]).max(p[1].abs_diff(p[2])) > 30
                    }),
                    "brand color missing: {name}"
                );
            }
            if name == "gemini" {
                // BGRA: the Gemini gradient has both red- and blue-dominant
                // regions. A text-colored SVG mask cannot satisfy both.
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[3] > 200 && p[0] > p[2].saturating_add(50))
                );
                assert!(
                    pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .any(|p| p[3] > 200 && p[2] > p[0].saturating_add(50))
                );
            }
        }
    }
}
