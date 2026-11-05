//! Browser-free Mermaid: Rust layout -> SVG -> GPUI's existing resvg renderer.
use super::*;
use gpui_kit::base::text::{MarkdownNode, MarkdownParseContext, markdown_ast};
use std::{
    collections::VecDeque,
    sync::{LazyLock, Mutex},
};

const NODE: &str = "conduit-mermaid";
const MAX_SOURCE: usize = 16 * 1024;
const PREVIEW_HEIGHT: f32 = 320.;
const CACHE_BYTES: usize = 32 * 1024 * 1024;
static RASTERIZER: LazyLock<SvgRenderer> =
    LazyLock::new(|| SvgRenderer::new(Arc::new(gpui_kit::assets::AllAssets)));
static CACHE: Mutex<PreviewCache> = Mutex::new(PreviewCache {
    entries: VecDeque::new(),
    bytes: 0,
});
// Keep layout/font work bounded when many diagrams enter the viewport together.
static RENDER_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, PartialEq, Eq)]
struct Key {
    source: String,
    dark: bool,
}

#[derive(Clone)]
struct DiagramImage {
    image: Arc<RenderImage>,
    // Logical size, independent of our bounded 2x raster resolution.
    width: f32,
    height: f32,
}
impl DiagramImage {
    fn bytes(&self) -> usize {
        let size = self.image.size(0);
        i32::from(size.width) as usize * i32::from(size.height) as usize * 4
    }
}
type PreviewResult = Result<DiagramImage, String>;

struct PreviewCache {
    entries: VecDeque<(Key, PreviewResult)>,
    bytes: usize,
}
impl PreviewCache {
    fn get(&mut self, key: &Key) -> Option<PreviewResult> {
        let index = self.entries.iter().position(|(entry, _)| entry == key)?;
        let entry = self.entries.remove(index)?;
        let result = entry.1.clone();
        self.entries.push_back(entry);
        Some(result)
    }
    fn insert(&mut self, key: Key, result: PreviewResult) {
        let cost = result.as_ref().map_or(0, DiagramImage::bytes);
        while !self.entries.is_empty()
            && (self.entries.len() >= 12 || self.bytes + cost > CACHE_BYTES)
        {
            if let Some((_, old)) = self.entries.pop_front() {
                self.bytes -= old.as_ref().map_or(0, DiagramImage::bytes);
            }
        }
        self.bytes += cost;
        self.entries.push_back((key, result));
    }
}

pub(super) fn is_mermaid(lang: &str) -> bool {
    lang.trim().eq_ignore_ascii_case("mermaid")
}

fn closed_fence(source: &str, quote_depth: usize) -> bool {
    let mut lines = source.lines();
    let Some(first) = lines.next() else {
        return false;
    };
    let first = first.trim_start();
    let Some(marker @ (b'`' | b'~')) = first.as_bytes().first().copied() else {
        return false;
    };
    let count = first.bytes().take_while(|byte| *byte == marker).count();
    count >= 3
        && lines.any(|line| {
            // The AST slice starts at the opening fence, but subsequent quoted
            // lines still include the containing block quote's prefix.
            let mut line = line.trim();
            for _ in 0..quote_depth {
                let Some(rest) = line.strip_prefix('>') else {
                    return false;
                };
                line = rest.trim_start();
            }
            line.len() >= count && line.bytes().all(|byte| byte == marker)
        })
}

fn parse(node: &markdown_ast::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
    let markdown_ast::Node::Code(code) = node else {
        return None;
    };
    if !code.lang.as_deref().is_some_and(is_mermaid) {
        return None;
    }
    let source = cx.node_source(node).unwrap_or_default();
    let quote_depth = node.position().map_or(0, |position| {
        cx.source()[..position.start.offset]
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .chars()
            .filter(|ch| *ch == '>')
            .count()
    });
    Some(
        MarkdownNode::new(NODE, closed_fence(source, quote_depth))
            .text(code.value.clone())
            .markdown(source),
    )
}

pub(super) fn mermaid_blocks(
    view: TextView,
    scope: &str,
    selectable: bool,
    search: Option<WeakEntity<super::file_preview::FilePreview>>,
) -> TextView {
    let scope = scope.to_string();
    view.markdown_block_parser(parse)
        .markdown_block_renderer(NODE, move |node, window, cx| {
            let key = Key {
                source: node.as_text().to_string(),
                dark: is_dark_mode(),
            };
            let start = node.source_range().map_or(0, |span| span.start);
            let id: SharedString = format!("{scope}-mermaid-{start}").into();
            let preview = window.use_keyed_state(id, cx, |_, _| MermaidPreview {
                key: None,
                closed: false,
                result: None,
                reserved_height: preview_height(selectable),
                show_source: false,
                selectable,
                viewer: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                task: Task::ready(()),
            });
            preview.update(cx, |preview, cx| {
                preview.selectable = selectable;
                preview.sync(key, node.data::<bool>().copied().unwrap_or(false), cx);
            });
            let reveal = search
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .and_then(|view| view.read(cx).block_search(node.source_range()?.clone()));
            if let Some(reveal) = reveal {
                div()
                    .w_full()
                    .min_w_0()
                    .child(preview)
                    .on_children_prepainted(move |children, window, cx| {
                        if let Some(bounds) = children.first() {
                            reveal.reveal_block(*bounds, window, cx);
                        }
                    })
                    .into_any_element()
            } else {
                preview.into_any_element()
            }
        })
}

struct MermaidPreview {
    key: Option<Key>,
    closed: bool,
    result: Option<PreviewResult>,
    reserved_height: f32,
    show_source: bool,
    selectable: bool,
    viewer: Option<Entity<MermaidViewer>>,
    cancelled: Arc<AtomicBool>,
    task: Task<()>,
}
impl Drop for MermaidPreview {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl MermaidPreview {
    fn sync(&mut self, key: Key, closed: bool, cx: &mut Context<Self>) {
        if self.key.as_ref() == Some(&key) && self.closed == closed {
            return;
        }
        self.cancelled.store(true, Ordering::Release);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.key = Some(key.clone());
        self.closed = closed;
        self.result = None;
        self.viewer = None;
        self.task = Task::ready(());
        cx.notify();
        if !closed {
            return;
        }
        if let Err(error) = validate_source(&key.source) {
            self.result = Some(Err(error));
            return;
        }
        if let Some(result) = CACHE.lock().unwrap().get(&key) {
            self.result = Some(result);
            return;
        }
        let ticket = self.cancelled.clone();
        self.task = cx.spawn(async move |this, cx| {
            let cancelled = ticket.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _guard = RENDER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                    if cancelled.load(Ordering::Acquire) {
                        return Err("已取消".into());
                    }
                    if let Some(result) = CACHE.lock().unwrap().get(&key) {
                        return result;
                    }
                    let result = render_native(&key.source, key.dark);
                    if !cancelled.load(Ordering::Acquire) {
                        CACHE.lock().unwrap().insert(key, result.clone());
                    }
                    result
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                // Source/theme may have changed while the old background job completed.
                if !Arc::ptr_eq(&ticket, &this.cancelled) || ticket.load(Ordering::Acquire) {
                    return;
                }
                this.result = Some(result);
                cx.notify();
            });
        });
    }
}

impl Render for MermaidPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self
            .key
            .as_ref()
            .map_or("", |key| key.source.as_str())
            .to_string();
        let mut body = v_flex()
            .debug_selector(|| "mermaid-block".into())
            .w_full()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child("Mermaid"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("mermaid-toggle")
                            .debug_selector(|| "mermaid-toggle".into())
                            .ghost()
                            .small()
                            .label(if self.show_source { "预览" } else { "源码" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_source = !this.show_source;
                                cx.notify();
                            })),
                    )
                    .child(
                        Clipboard::new("mermaid-copy")
                            .value(source.clone())
                            .tooltip("复制源码"),
                    ),
            );
        if !self.show_source
            && let Some(Ok(diagram)) = &self.result
        {
            let diagram = diagram.clone();
            let viewer = self
                .viewer
                .get_or_insert_with(|| cx.new(|_| MermaidViewer::inline(diagram.clone())))
                .clone();
            body = body.child(
                div()
                    .debug_selector(|| "mermaid-preview-viewport".into())
                    .w_full()
                    .h(px(self.reserved_height))
                    .child(viewer),
            );
        } else {
            let message = match &self.result {
                Some(Err(error)) => Some(format!("无法预览，已保留源码：{error}")),
                None if self.closed => Some("正在生成图表…".to_string()),
                None => Some("等待代码块结束".to_string()),
                _ => None,
            };
            let mut preview = v_flex()
                .debug_selector(|| "mermaid-preview-viewport".into())
                .w_full()
                .h(px(self.reserved_height))
                .min_h_0()
                .gap_1()
                .overflow_hidden();
            if let Some(message) = message {
                preview = preview.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
                        .child(message),
                );
            }
            preview = preview.child(
                div()
                    .debug_selector(|| "mermaid-source".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .font_family(Theme::global(cx).mono_font_family.clone())
                    .text_size(px(12.))
                    .when(self.selectable, |view| view.cursor_text())
                    .child(if self.selectable {
                        gpui_kit::base::SelectableText::new("mermaid-source-text", source)
                            .into_any_element()
                    } else {
                        div().child(source).into_any_element()
                    }),
            );
            body = body.child(preview);
        }
        body
    }
}

/// Use the measured viewport for both axes, independent of image intrinsic pixels.
fn diagram_bounds(
    viewport: Bounds<Pixels>,
    diagram: &DiagramImage,
    fit: bool,
    zoom: f32,
    pan: Point<Pixels>,
) -> Bounds<Pixels> {
    let scale = if fit {
        1_f32
            .min(f32::from(viewport.size.width) / diagram.width)
            .min(f32::from(viewport.size.height) / diagram.height)
    } else {
        1.
    };
    let image_size = size(
        px(diagram.width * scale * zoom),
        px(diagram.height * scale * zoom),
    );
    Bounds::new(
        point(
            viewport.center().x - image_size.width / 2. + pan.x,
            viewport.center().y - image_size.height / 2. + pan.y,
        ),
        image_size,
    )
}

struct MermaidViewer {
    diagram: DiagramImage,
    viewport: Bounds<Pixels>,
    inline: bool,
    zoom: f32,
    pan: Point<Pixels>,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    dragged: bool,
}
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 8.;
const ZOOM_STEP: f32 = 1.25;

impl MermaidViewer {
    fn new(diagram: DiagramImage) -> Self {
        Self {
            diagram,
            viewport: Default::default(),
            inline: false,
            zoom: 1.,
            pan: Point::default(),
            drag: None,
            dragged: false,
        }
    }

    fn inline(diagram: DiagramImage) -> Self {
        Self {
            inline: true,
            ..Self::new(diagram)
        }
    }

    fn can_pan(&self) -> bool {
        !self.inline || self.zoom > 1.
    }

    fn pan_to(&mut self, pan: Point<Pixels>) {
        let bounds = diagram_bounds(
            self.viewport,
            &self.diagram,
            self.inline,
            self.zoom,
            Point::default(),
        );
        let limit = |image: Pixels, viewport: Pixels| {
            // Inline panning only reveals cropped content. Popup panning also
            // permits moving a small image, while keeping 48px recoverable.
            if self.inline {
                ((f32::from(image) - f32::from(viewport)) / 2.).max(0.)
            } else {
                ((f32::from(viewport) + f32::from(image)) / 2. - 48.).max(0.)
            }
        };
        let limit_x = limit(bounds.size.width, self.viewport.size.width);
        let limit_y = limit(bounds.size.height, self.viewport.size.height);
        self.pan = point(
            px(f32::from(pan.x).clamp(-limit_x, limit_x)),
            px(f32::from(pan.y).clamp(-limit_y, limit_y)),
        );
    }

    fn zoom_at(&mut self, zoom: f32, anchor: Point<Pixels>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = zoom / self.zoom;
        let offset = anchor - self.viewport.center();
        self.pan = offset - (offset - self.pan) * ratio;
        self.zoom = zoom;
        self.drag = None;
        self.pan_to(self.pan);
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }
}
impl Render for MermaidViewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let image = self.diagram.image.clone();
        let prefix = if self.inline {
            "mermaid-inline"
        } else {
            "mermaid-popup"
        };
        let zoom_out = format!("{prefix}-zoom-out");
        let zoom_in = format!("{prefix}-zoom-in");
        let zoom_reset = format!("{prefix}-zoom-reset");
        let toolbar = h_flex()
            .w_full()
            .h(px(32.))
            .flex_shrink_0()
            .justify_end()
            .items_center()
            .gap_1()
            .child(
                Button::new(SharedString::from(zoom_out.clone()))
                    .debug_selector(move || zoom_out.clone())
                    .ghost()
                    .small()
                    .icon(FullIconName::Minus)
                    .tooltip("缩小")
                    .accessibility_label("缩小图表")
                    .disabled(self.zoom <= MIN_ZOOM)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom_at(this.zoom / ZOOM_STEP, this.viewport.center());
                        cx.notify();
                    })),
            )
            .child(
                Button::new(SharedString::from(zoom_reset.clone()))
                    .debug_selector(move || zoom_reset.clone())
                    .ghost()
                    .small()
                    .label(format!("{:.0}%", self.zoom * 100.))
                    .tooltip("恢复默认比例并居中")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom = 1.;
                        this.pan = Point::default();
                        this.drag = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new(SharedString::from(zoom_in.clone()))
                    .debug_selector(move || zoom_in.clone())
                    .ghost()
                    .small()
                    .icon(FullIconName::Plus)
                    .tooltip("放大")
                    .accessibility_label("放大图表")
                    .disabled(self.zoom >= MAX_ZOOM)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.zoom_at(this.zoom * ZOOM_STEP, this.viewport.center());
                        cx.notify();
                    })),
            );
        let selector = if self.inline {
            "mermaid-image"
        } else {
            "mermaid-large"
        };
        let viewport = div()
            .id(selector)
            .debug_selector(move || selector.into())
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .cursor(if self.drag.is_some() {
                CursorStyle::ClosedHand
            } else if self.can_pan() {
                CursorStyle::OpenHand
            } else {
                CursorStyle::PointingHand
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.dragged = false;
                    if this.can_pan() {
                        this.drag = Some((event.position, this.pan));
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    this.end_drag(cx);
                    return;
                }
                if let Some((start, pan)) = this.drag {
                    let delta = event.position - start;
                    if f32::from(delta.x).abs().max(f32::from(delta.y).abs()) > 4. {
                        this.dragged = true;
                    }
                    if this.dragged {
                        this.pan_to(pan + delta);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_drag(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_drag(cx)),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(24.));
                if event.modifiers.control {
                    let factor = ZOOM_STEP.powf((f32::from(delta.y) / 80.).clamp(-4., 4.));
                    this.zoom_at(this.zoom * factor, event.position);
                } else if this.inline {
                    // Leave ordinary wheel input to the chat transcript.
                    return;
                } else {
                    this.pan_to(this.pan + delta);
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                if this.dragged {
                    return;
                }
                if this.inline {
                    open_mermaid_popup(this.diagram.clone(), window, cx);
                } else if event.click_count() == 2 {
                    this.pan = Point::default();
                    cx.notify();
                }
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        view.update(cx, |this, _| {
                            this.viewport = bounds;
                            this.pan_to(this.pan);
                            diagram_bounds(bounds, &this.diagram, this.inline, this.zoom, this.pan)
                        })
                    },
                    move |bounds, image_bounds, window, _| {
                        let _ = window.paint_image(
                            bounds,
                            image_bounds,
                            Corners::default(),
                            image.clone(),
                            0,
                            false,
                        );
                    },
                )
                .size_full(),
            );
        v_flex().size_full().gap_0().child(toolbar).child(viewport)
    }
}

fn open_mermaid_popup(diagram: DiagramImage, window: &mut Window, cx: &mut App) {
    let viewer = cx.new(|_| MermaidViewer::new(diagram));
    window.open_dialog(cx, move |dialog, window, _| {
        let width = (f32::from(window.viewport_size().width) - 48.).max(200.);
        let height = (f32::from(window.viewport_size().height) - 150.).max(120.);
        let viewer = viewer.clone();
        dialog
            .title("Mermaid 图表")
            .w(px(width))
            .content(move |content, _, _| {
                content.child(div().w_full().h(px(height)).child(viewer.clone()))
            })
    });
}

fn validate_source(source: &str) -> Result<(), String> {
    if source.len() > MAX_SOURCE || source.lines().count() > 256 {
        return Err("图表超过预览上限（16 KiB / 256 行）".into());
    }
    Ok(())
}

// Reserve a stable block height before parsing finishes. When a recycled
// markdown row re-enters the viewport, it must never measure as source first
// and then resize to the diagram after the background render completes.
fn preview_height(selectable: bool) -> f32 {
    PREVIEW_HEIGHT + if selectable { 44. } else { 40. }
}

// The backend's strict validator does not reject unfinished flowchart node shapes.
// Check delimiters while honoring quoted labels, escapes, comments and edge labels.
fn validate_flowchart(source: &str) -> Result<(), String> {
    let mut stack = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut edge_label = false;
    for line in source.lines() {
        if line.trim_start().starts_with("%%") {
            continue;
        }
        for ch in line.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quoted {
                escaped = true;
                continue;
            }
            if ch == '"' {
                quoted = !quoted;
                continue;
            }
            if quoted {
                continue;
            }
            if ch == '|' {
                edge_label = !edge_label;
                continue;
            }
            if edge_label {
                continue;
            }
            match ch {
                '[' | '(' | '{' => stack.push(ch),
                ']' | ')' | '}' => {
                    let expected = match ch {
                        ']' => '[',
                        ')' => '(',
                        _ => '{',
                    };
                    if stack.pop() != Some(expected) {
                        return Err("流程图节点括号不匹配".into());
                    }
                }
                _ => (),
            }
        }
    }
    if !stack.is_empty() || quoted || edge_label {
        return Err("流程图节点或标签尚未闭合".into());
    }
    Ok(())
}

fn render_native(source: &str, dark: bool) -> PreviewResult {
    // Guard against a malformed diagram panicking inside the third-party renderer.
    std::panic::catch_unwind(|| render_native_inner(source, dark))
        .unwrap_or_else(|_| Err("原生渲染器无法处理此图表".into()))
}

fn render_native_inner(source: &str, dark: bool) -> PreviewResult {
    use mermaid_rs_renderer::{
        DiagramKind, LayoutConfig, compute_layout, parse_mermaid_strict, render_svg,
    };
    validate_source(source)?;
    let parsed = parse_mermaid_strict(source)
        .map_err(|error| error.to_string().chars().take(180).collect::<String>())?;
    if parsed.graph.kind == DiagramKind::Flowchart {
        validate_flowchart(source)?;
    }
    if !matches!(
        parsed.graph.kind,
        DiagramKind::Flowchart
            | DiagramKind::Sequence
            | DiagramKind::Class
            | DiagramKind::State
            | DiagramKind::Er
            | DiagramKind::Pie
            | DiagramKind::Mindmap
    ) {
        return Err("此图表类型尚未开放原生预览".into());
    }
    if parsed.graph.nodes.len() > 128
        || parsed.graph.edges.len() > 256
        || parsed.graph.subgraphs.len() > 32
    {
        return Err("图表过于复杂（最多 128 个节点 / 256 条边 / 32 个子图）".into());
    }
    let mut theme = if dark {
        mermaid_rs_renderer::Theme::dark()
    } else {
        mermaid_rs_renderer::Theme::modern()
    };
    theme.font_family = "Segoe UI, Microsoft YaHei, Noto Sans CJK SC, sans-serif".into();
    let config = LayoutConfig::default();
    let layout = compute_layout(&parsed.graph, &theme, &config);
    let width = layout.width;
    let height = layout.height;
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
        || width > 12000.
        || height > 12000.
    {
        return Err("图表尺寸超过原生预览上限".into());
    }
    let svg = render_svg(&layout, &theme, &config);
    if svg.len() > 2 * 1024 * 1024 {
        return Err("图表输出过大".into());
    }
    let parsed_svg = RASTERIZER
        .parse_svg(svg.as_bytes())
        .map_err(|error| error.to_string())?;
    let scale = 2_f32
        .min(4096. / width.max(height))
        .min((4_000_000. / (width * height)).sqrt());
    let raster_size = size(
        DevicePixels((width * scale).ceil() as i32),
        DevicePixels((height * scale).ceil() as i32),
    );
    let image = RASTERIZER
        .render_parsed(&parsed_svg, SvgSize::ExactSize(raster_size))
        .map_err(|error| error.to_string())?;
    Ok(DiagramImage {
        image,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CACHE_BYTES, Key, PreviewCache, closed_fence, diagram_bounds, is_mermaid, render_native,
    };
    use gpui_kit::{Bounds, Point, point, px, size};
    use std::collections::VecDeque;

    #[test]
    fn mermaid_streaming_fences_require_matching_closer() {
        assert!(is_mermaid(" Mermaid "));
        assert!(!is_mermaid("rust"));
        assert!(!closed_fence("```mermaid\ngraph TD\n A-->B", 0));
        assert!(!closed_fence("````mermaid\ngraph TD\n```", 0));
        assert!(!closed_fence("```mermaid\ngraph TD\n~~~", 0));
        assert!(closed_fence("```mermaid\ngraph TD\n A-->B\n```", 0));
        assert!(closed_fence("~~~~mermaid\r\ngraph TD\r\n~~~~", 0));
        assert!(closed_fence("```mermaid\n> graph TD\n> A-->B\n> ```", 1));
        assert!(!closed_fence("```mermaid\n> graph TD\n> > ```", 1));
    }

    #[test]
    fn mermaid_native_renders_chinese_and_common_diagrams_in_both_themes() {
        for source in [
            "flowchart TD\n A[聊天内容] --> B{需要图表?}\n B -->|是| C[原生绘制]",
            "sequenceDiagram\n participant A as 用户\n participant B as 助手\n A->>B: 绘制图表\n B-->>A: 完成",
            "classDiagram\n Animal <|-- Duck\n Animal : +int age",
            "stateDiagram-v2\n [*] --> Idle\n Idle --> Done\n Done --> [*]",
            "erDiagram\n CUSTOMER ||--o{ ORDER : places",
            "pie\n \"Rust\" : 80\n \"GPUI\" : 20",
            "mindmap\n root((原生))\n  Rust\n  GPUI",
        ] {
            for dark in [false, true] {
                let result = render_native(source, dark);
                assert!(result.is_ok(), "{source}: {:?}", result.err());
                let diagram = result.unwrap();
                assert!(diagram.width > 0. && diagram.height > 0.);
                assert!(diagram.bytes() <= 16_050_000);
                // Optional review artifacts come from the actual native rasterizer.
                if source.starts_with("flowchart")
                    && let Some(directory) = std::env::var_os("CONDUIT_MERMAID_REVIEW_DIR")
                {
                    std::fs::create_dir_all(&directory).unwrap();
                    let size = diagram.image.size(0);
                    let mut pixels = diagram.image.as_bytes(0).unwrap().to_vec();
                    for pixel in pixels.as_chunks_mut::<4>().0 {
                        pixel.swap(0, 2);
                    }
                    image::save_buffer_with_format(
                        std::path::Path::new(&directory).join(if dark {
                            "dark.png"
                        } else {
                            "light.png"
                        }),
                        &pixels,
                        i32::from(size.width) as u32,
                        i32::from(size.height) as u32,
                        image::ColorType::Rgba8,
                        image::ImageFormat::Png,
                    )
                    .unwrap();
                }
            }
        }
    }

    #[test]
    fn mermaid_native_errors_limits_and_cache_eviction() {
        assert!(render_native("not a diagram", false).is_err());
        assert!(render_native("flowchart TD\n A[broken", false).is_err());
        assert!(render_native(&"a".repeat(16 * 1024 + 1), false).is_err());
        assert!(render_native("gantt\n title Unsupported", false).is_err());
        let too_many = format!(
            "flowchart TD\n{}",
            (0..130)
                .map(|i| format!("N{i} --> N{}\n", i + 1))
                .collect::<String>()
        );
        assert!(render_native(&too_many, false).is_err());
        let mut cache = PreviewCache {
            entries: VecDeque::new(),
            bytes: 0,
        };
        for i in 0..14 {
            cache.insert(
                Key {
                    source: i.to_string(),
                    dark: false,
                },
                Err("error".into()),
            );
        }
        assert_eq!(cache.entries.len(), 12);
        assert!(
            cache
                .get(&Key {
                    source: "0".into(),
                    dark: false
                })
                .is_none()
        );
        assert!(
            cache
                .get(&Key {
                    source: "13".into(),
                    dark: true
                })
                .is_none()
        );
        assert!(cache.bytes <= CACHE_BYTES);
    }

    #[test]
    fn mermaid_images_center_in_preview_and_popup() {
        let mut diagram = render_native("graph LR; A-->B", false).unwrap();
        let viewport = Bounds::new(point(px(30.), px(50.)), size(px(600.), px(400.)));
        for (width, height) in [(200., 100.), (1400., 1000.), (100., 2000.)] {
            diagram.width = width;
            diagram.height = height;
            for fit in [false, true] {
                let bounds = diagram_bounds(viewport, &diagram, fit, 1., Point::default());
                assert_eq!(bounds.center(), viewport.center());
                if fit {
                    assert!(bounds.size.width <= viewport.size.width);
                    assert!(bounds.size.height <= viewport.size.height);
                }
            }
        }
    }

    #[test]
    fn mermaid_zoom_keeps_pointer_anchor_and_limits_scale() {
        use super::{MAX_ZOOM, MIN_ZOOM, MermaidViewer};
        let diagram = render_native("graph LR; A-->B", false).unwrap();
        let mut viewer = MermaidViewer::new(diagram);
        viewer.viewport = Bounds::new(point(px(30.), px(50.)), size(px(600.), px(400.)));
        let anchor = viewer.viewport.center() + point(px(40.), px(30.));
        viewer.zoom_at(1.25, anchor);
        assert_eq!(viewer.pan, point(px(-10.), px(-7.5)));
        viewer.zoom_at(100., anchor);
        assert_eq!(viewer.zoom, MAX_ZOOM);
        viewer.zoom_at(0.001, anchor);
        assert_eq!(viewer.zoom, MIN_ZOOM);
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests {
    use super::{ChatTypography, Context, IntoElement, Render, TextView, Window, div, px};
    use gpui_kit::{
        AppContext as _, Modifiers, MouseButton, ParentElement, Styled, TestAppContext,
        VisualTestContext,
    };

    struct TestView {
        source: String,
        numbered: bool,
    }
    impl Render for TestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let typography = ChatTypography {
                body_font: None,
                body_size: 15.,
                line_height: 1.5,
                code_font: "Consolas".into(),
                code_size: 12.,
                code_copy: true,
                syntax_highlight: true,
                code_line_numbers: self.numbered,
            };
            let view: TextView = super::super::chat_markdown_view(
                "mermaid-test".into(),
                self.source.clone(),
                &typography,
                "mermaid-test",
                true,
            );
            div().w(px(600.)).child(view)
        }
    }
    fn settle(cx: &mut VisualTestContext) {
        for _ in 0..4 {
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
    }
    #[gpui_kit::test]
    fn mermaid_chat_streaming_line_numbers_and_source_toggle(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|_| TestView {
                source: "```mermaid\ngraph TD\n A[聊天] --> B[原生]".into(),
                numbered: false,
            });
            *output.borrow_mut() = Some(view.clone());
            gpui_kit::component::Root::new(view, window, cx)
        });
        let view = slot.borrow().as_ref().unwrap().clone();
        settle(cx);
        assert!(cx.debug_bounds("mermaid-source").is_some());
        assert!(cx.debug_bounds("mermaid-image").is_none());
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.source.push_str("\n```");
                cx.notify();
            })
        });
        settle(cx);
        assert!(cx.debug_bounds("mermaid-image").is_some());
        for numbered in [true, false] {
            cx.update(|_, cx| {
                view.update(cx, |view, cx| {
                    view.numbered = numbered;
                    cx.notify();
                })
            });
            settle(cx);
            assert!(
                cx.debug_bounds("mermaid-image").is_some(),
                "line numbers must preserve the diagram"
            );
        }
        // Toolbar controls must change the inline view without opening a popup.
        for selector in ["mermaid-inline-zoom-in", "mermaid-inline-zoom-out"] {
            let position = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
            settle(cx);
            use gpui_kit::component::WindowExt as _;
            cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        }
        let position = cx.debug_bounds("mermaid-toggle").unwrap().center();
        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        settle(cx);
        assert!(cx.debug_bounds("mermaid-source").is_some());
        assert!(cx.debug_bounds("mermaid-image").is_none());
        let position = cx.debug_bounds("mermaid-toggle").unwrap().center();
        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        settle(cx);
        let position = cx.debug_bounds("mermaid-image").unwrap().center();
        cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
        settle(cx);
        use gpui_kit::component::WindowExt as _;
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
    }

    struct ViewerRoot(gpui_kit::Entity<super::MermaidViewer>);
    impl Render for ViewerRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(600.)).h(px(400.)).child(self.0.clone())
        }
    }

    #[gpui_kit::test]
    fn mermaid_popup_drags_and_stops_after_release_outside(cx: &mut TestAppContext) {
        use gpui_kit::{Point, point};
        cx.update(gpui_kit::init);
        let diagram = super::render_native("graph LR; A[开始]-->B[结束]", false).unwrap();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let viewer = cx.new(|_| super::MermaidViewer::new(diagram));
            *output.borrow_mut() = Some(viewer.clone());
            let root = cx.new(|_| ViewerRoot(viewer));
            gpui_kit::component::Root::new(root, window, cx)
        });
        let viewer = slot.borrow().as_ref().unwrap().clone();
        settle(cx);
        let bounds = cx.debug_bounds("mermaid-large").unwrap();
        cx.update(|_, cx| assert_eq!(viewer.read(cx).pan, Point::default()));
        let start = bounds.center();
        let delta = point(px(80.), px(50.));
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(start + delta, Some(MouseButton::Left), Modifiers::default());
        settle(cx);
        cx.update(|_, cx| assert_eq!(viewer.read(cx).pan, delta));
        let outside = point(bounds.right() + px(20.), bounds.bottom() + px(20.));
        cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
        settle(cx);
        cx.update(|_, cx| assert!(viewer.read(cx).drag.is_none()));
        cx.simulate_mouse_move(start, None, Modifiers::default());
        settle(cx);
        cx.update(|_, cx| assert_eq!(viewer.read(cx).pan, delta));
    }

    #[gpui_kit::test]
    fn mermaid_zoom_controls_ctrl_wheel_and_inline_drag(cx: &mut TestAppContext) {
        use gpui_kit::component::WindowExt as _;
        use gpui_kit::{Point, ScrollDelta, ScrollWheelEvent, point};
        cx.update(gpui_kit::init);
        let mut diagram = super::render_native("graph TD; A[开始]-->B[结束]", false).unwrap();
        diagram.width = 800.;
        diagram.height = 800.;
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let viewer = cx.new(|_| super::MermaidViewer::inline(diagram));
            *output.borrow_mut() = Some(viewer.clone());
            let root = cx.new(|_| ViewerRoot(viewer));
            gpui_kit::component::Root::new(root, window, cx)
        });
        let viewer = slot.borrow().as_ref().unwrap().clone();
        settle(cx);
        for inline in [true, false] {
            cx.update(|_, cx| {
                viewer.update(cx, |viewer, cx| {
                    viewer.inline = inline;
                    viewer.zoom = 1.;
                    viewer.pan = Point::default();
                    cx.notify();
                })
            });
            settle(cx);
            let (zoom_in, zoom_out, zoom_reset) = if inline {
                (
                    "mermaid-inline-zoom-in",
                    "mermaid-inline-zoom-out",
                    "mermaid-inline-zoom-reset",
                )
            } else {
                (
                    "mermaid-popup-zoom-in",
                    "mermaid-popup-zoom-out",
                    "mermaid-popup-zoom-reset",
                )
            };
            let selector = if inline {
                "mermaid-image"
            } else {
                "mermaid-large"
            };
            let click = |cx: &mut VisualTestContext, selector: &'static str| {
                let position = cx.debug_bounds(selector).unwrap().center();
                cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
                cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
                settle(cx);
            };
            click(cx, zoom_in);
            cx.update(|_, cx| assert_eq!(viewer.read(cx).zoom, 1.25));
            let start = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_move(
                start + point(px(0.), px(24.)),
                Some(MouseButton::Left),
                Modifiers::default(),
            );
            cx.simulate_mouse_up(
                start + point(px(0.), px(24.)),
                MouseButton::Left,
                Modifiers::default(),
            );
            settle(cx);
            cx.update(|window, cx| {
                assert!(viewer.read(cx).pan.y > px(0.));
                assert!(
                    !window.has_active_dialog(cx),
                    "dragging must not open a popup"
                );
            });
            click(cx, zoom_reset);
            cx.update(|_, cx| {
                assert_eq!(viewer.read(cx).zoom, 1.);
                assert_eq!(viewer.read(cx).pan, Point::default());
            });
            let position = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(80.))),
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            });
            settle(cx);
            cx.update(|_, cx| assert_eq!(viewer.read(cx).zoom, 1.25));
            cx.simulate_event(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(-80.))),
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            });
            settle(cx);
            cx.update(|_, cx| assert_eq!(viewer.read(cx).zoom, 1.));
            click(cx, zoom_out);
            cx.update(|_, cx| assert_eq!(viewer.read(cx).zoom, 0.8));
            if inline {
                cx.simulate_event(ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Pixels(point(px(0.), px(-80.))),
                    ..Default::default()
                });
                settle(cx);
                cx.update(|_, cx| {
                    assert_eq!(viewer.read(cx).zoom, 0.8);
                    assert_eq!(viewer.read(cx).pan, Point::default());
                });
            }
        }
    }

    #[gpui_kit::test]
    fn mermaid_history_scroll_keeps_message_height_and_anchor(cx: &mut TestAppContext) {
        use super::super::ConduitApp;
        use crate::{domain::Workspace, store::Store};
        use gpui_kit::{ScrollDelta, ScrollWheelEvent, point};
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut session = super::super::tests::session();
                session.messages = (0..32)
                    .map(|index| {
                        let kind = if index % 2 == 0 { "Assistant" } else { "User" };
                        let text = if index % 2 == 0 {
                            format!(
                                "```mermaid\nflowchart TD\n A[开始 {index}] --> B[完成]\n```\n\n{}",
                                (0..120).map(|line| format!("段落 {line}：图表下方的聊天内容，用来检查上下滚动时布局是否稳定。\n\n")).collect::<String>()
                            )
                        } else {
                            format!("用户消息 {index}，用于验证气泡跳转后的滚动锚点。{}", "额外内容 ".repeat(300))
                        };
                        serde_json::from_value(serde_json::json!({
                            "kind": kind, "key": format!("diagram-{index}"), "text": text
                        })).unwrap()
                    })
                    .collect();
                let store = Store { workspaces: vec![Workspace { path: std::env::temp_dir(), sessions: vec![session] }], ..Default::default() };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.startup = None;
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(gpui_kit::size(px(1100.), px(850.)));
        settle(cx);
        for index in [1, 5, 13, 21] {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.jump_to_message(1, &format!("diagram-{index}"), cx);
                })
            });
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let initial_preview = cx.debug_bounds("mermaid-preview-viewport");
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert!(
                    !app.at_bottom,
                    "navigation to row {index} was overridden after first layout: {:?}",
                    app.scroll.logical_scroll_top()
                );
            });
            settle(cx);
            let settled = cx.update(|_, cx| app.read(cx).scroll.logical_scroll_top());
            assert_eq!(
                settled.item_ix, index,
                "jump target {index} was lost after deferred Markdown layout: {settled:?}"
            );
            if let Some(initial) = initial_preview {
                assert_eq!(
                    cx.debug_bounds("mermaid-preview-viewport"),
                    Some(initial),
                    "preview layout changed after native rendering"
                );
            }
            let bounds = cx.update(|_, cx| {
                let app = app.read(cx);
                app.scroll.bounds_for_item(index).unwrap_or_else(|| {
                    panic!(
                        "missing row {index}, count {}, anchor {:?}",
                        app.scroll.item_count(),
                        app.scroll.logical_scroll_top()
                    )
                })
            });
            let anchor = cx.update(|_, cx| app.read(cx).scroll.logical_scroll_top());
            for _ in 0..6 {
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                });
                cx.run_until_parked();
                let current =
                    cx.update(|_, cx| app.read(cx).scroll.bounds_for_item(index).unwrap());
                assert_eq!(
                    current, bounds,
                    "message {index} changed size/position without input"
                );
                let current_anchor = cx.update(|_, cx| app.read(cx).scroll.logical_scroll_top());
                assert_eq!(
                    (current_anchor.item_ix, current_anchor.offset_in_item),
                    (anchor.item_ix, anchor.offset_in_item)
                );
            }
            let mut preview_bounds = None;
            for delta in [-70., 70., -70., 70.] {
                let position = cx.update(|_, cx| app.read(cx).scroll.viewport_bounds().center());
                cx.simulate_event(ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
                    ..Default::default()
                });
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                });
                cx.update(|_, cx| {
                    let app = app.read(cx);
                    assert!(
                        app.pending_message_jump.is_none(),
                        "wheel input must take over the jump anchor"
                    );
                    assert!(app.scroll.logical_scroll_top().item_ix < app.scroll.item_count());
                });
                if let Some(bounds) = cx.debug_bounds("mermaid-preview-viewport") {
                    if let Some(previous) = preview_bounds {
                        assert_eq!(
                            bounds, previous,
                            "Mermaid block changed height as it re-entered the viewport"
                        );
                    }
                    preview_bounds = Some(bounds);
                }
                let before = cx.update(|_, cx| app.read(cx).scroll.logical_scroll_top());
                settle(cx);
                let after = cx.update(|_, cx| app.read(cx).scroll.logical_scroll_top());
                assert_eq!(
                    (before.item_ix, before.offset_in_item),
                    (after.item_ix, after.offset_in_item),
                    "scroll anchor moved after the wheel stopped"
                );
            }
        }
    }
}
