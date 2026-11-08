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
                        return Err(conduit::i18n::text_static("events.cancelled").into());
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
                            .label(if self.show_source {
                                conduit::i18n::text_static("mermaid.preview")
                            } else {
                                conduit::i18n::text_static("mermaid.source")
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_source = !this.show_source;
                                cx.notify();
                            })),
                    )
                    .child(
                        Clipboard::new("mermaid-copy")
                            .value(source.clone())
                            .tooltip(conduit::i18n::text_static("mermaid.copy_source")),
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
                Some(Err(error)) => Some(conduit::i18n::format(
                    "mermaid.preview_unavailable_source_preserved",
                    &[("error", error.to_string())],
                )),
                None if self.closed => {
                    Some(conduit::i18n::text_static("mermaid.generating_diagram").to_string())
                }
                None => Some(
                    conduit::i18n::text_static("mermaid.waiting_for_the_code_block_to_finish")
                        .to_string(),
                ),
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
                    .tooltip(conduit::i18n::text_static("mermaid.zoom_out"))
                    .accessibility_label(conduit::i18n::text_static("mermaid.zoom_out_diagram"))
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
                    .tooltip(conduit::i18n::text_static("mermaid.reset_zoom_and_center"))
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
                    .tooltip(conduit::i18n::text_static("mermaid.zoom_in"))
                    .accessibility_label(conduit::i18n::text_static("mermaid.zoom_in_diagram"))
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
            .title(conduit::i18n::text_static("mermaid.mermaid_diagram"))
            .w(px(width))
            .content(move |content, _, _| {
                content.child(div().w_full().h(px(height)).child(viewer.clone()))
            })
    });
}

fn validate_source(source: &str) -> Result<(), String> {
    if source.len() > MAX_SOURCE || source.lines().count() > 256 {
        return Err(conduit::i18n::text_static(
            "mermaid.diagram_exceeds_preview_limits_16_kib_256",
        )
        .into());
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
                        return Err(conduit::i18n::text_static(
                            "mermaid.unmatched_brackets_in_flowchart_nodes",
                        )
                        .into());
                    }
                }
                _ => (),
            }
        }
    }
    if !stack.is_empty() || quoted || edge_label {
        return Err(
            conduit::i18n::text_static("mermaid.flowchart_node_or_label_is_not_closed").into(),
        );
    }
    Ok(())
}

fn render_native(source: &str, dark: bool) -> PreviewResult {
    // Guard against a malformed diagram panicking inside the third-party renderer.
    std::panic::catch_unwind(|| render_native_inner(source, dark)).unwrap_or_else(|_| {
        Err(
            conduit::i18n::text_static("mermaid.the_native_renderer_cannot_process_this_diagram")
                .into(),
        )
    })
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
        return Err(
            conduit::i18n::text_static("mermaid.native_preview_is_not_available_for_this").into(),
        );
    }
    if parsed.graph.nodes.len() > 128
        || parsed.graph.edges.len() > 256
        || parsed.graph.subgraphs.len() > 32
    {
        return Err(conduit::i18n::text_static("mermaid.diagram_is_too_complex_up_to_128").into());
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
        return Err(conduit::i18n::text_static(
            "mermaid.diagram_dimensions_exceed_the_native_preview_limit",
        )
        .into());
    }
    let svg = render_svg(&layout, &theme, &config);
    if svg.len() > 2 * 1024 * 1024 {
        return Err(conduit::i18n::text_static("mermaid.diagram_output_is_too_large").into());
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
