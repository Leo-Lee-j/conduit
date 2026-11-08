//! Shared icons, message actions, and menu controls.

use super::*;

mod app_logo;
pub(super) use app_logo::app_icon;

/// Share the same status appearance between sidebar and tabs, with separate
/// element/animation keys so both can be mounted for the same session.
pub(super) fn session_activity_dot(scope: &'static str, id: u64, running: bool) -> AnyElement {
    let label = if running {
        conduit::i18n::text_static("components.running")
    } else {
        conduit::i18n::text_static("components.response_complete_unread")
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
    ("qoder", include_bytes!("../../assets/icons/qoder.svg")),
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

// Keep full-color SVGs, but rasterize at the actual device-pixel size. The
// generic Image SVG decoder supersamples a fixed intrinsic size, leaving tiny
// marks to be downscaled by the GPU and blurring fine strokes.
static BRAND_RENDERER: std::sync::LazyLock<SvgRenderer> =
    std::sync::LazyLock::new(|| SvgRenderer::new(Arc::new(gpui_kit::assets::AllAssets)));
static BRAND_PARSED: std::sync::LazyLock<HashMap<(&str, bool), ParsedSvg>> =
    std::sync::LazyLock::new(|| {
        BRAND_SVGS
            .iter()
            .flat_map(|(name, bytes)| {
                [false, true].map(|dark| {
                    let ink = if dark { "#F3F3F4" } else { "#17171B" };
                    let svg = std::str::from_utf8(bytes)
                        .expect("embedded brand SVG")
                        .replace("currentColor", ink)
                        .replace("\"1em\"", "\"24\"");
                    (
                        (*name, dark),
                        BRAND_RENDERER
                            .parse_svg(svg.as_bytes())
                            .expect("valid brand SVG"),
                    )
                })
            })
            .collect()
    });
type BrandImageCache = HashMap<(&'static str, bool, i32, i32), Arc<RenderImage>>;
static BRAND_IMAGES: std::sync::LazyLock<std::sync::Mutex<BrandImageCache>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

fn brand_render(name: &'static str, dark: bool, width: i32, height: i32) -> Arc<RenderImage> {
    let mut cache = BRAND_IMAGES.lock().expect("brand image cache");
    cache
        .entry((name, dark, width, height))
        .or_insert_with(|| {
            BRAND_RENDERER
                .render_parsed(
                    &BRAND_PARSED[&(name, dark)],
                    SvgSize::ExactSize(size(DevicePixels(width), DevicePixels(height))),
                )
                .expect("brand SVG rasterization")
        })
        .clone()
}

fn brand_image(name: &'static str) -> Div {
    let dark = is_dark_mode();
    div().size(px(14.)).flex_shrink_0().child(
        canvas(
            move |bounds, window, _| {
                let scale = window.scale_factor();
                let width = (f32::from(bounds.size.width) * scale).round().max(1.) as i32;
                let height = (f32::from(bounds.size.height) * scale).round().max(1.) as i32;
                let origin = point(
                    px((f32::from(bounds.origin.x) * scale).round() / scale),
                    px((f32::from(bounds.origin.y) * scale).round() / scale),
                );
                (
                    brand_render(name, dark, width, height),
                    Bounds::new(
                        origin,
                        size(px(width as f32 / scale), px(height as f32 / scale)),
                    ),
                )
            },
            |_, (image, bounds), window, _| {
                let _ = window.paint_image(bounds, bounds, Corners::default(), image, 0, false);
            },
        )
        .size_full(),
    )
}

pub(super) fn provider_icon(provider: Provider) -> Div {
    brand_image(match provider {
        Provider::Claude => "claudecode",
        Provider::Codex => "codex",
        Provider::OpenCode => "opencode",
        Provider::Qoder => "qoder",
    })
}

/// Model family wins over the hosting provider, including routed model IDs.
pub(super) fn model_icon(provider: Provider, id: &str) -> Div {
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
pub(super) fn model_mark(provider: Provider, id: Option<&str>) -> Div {
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
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(label),
                ),
        )
}
