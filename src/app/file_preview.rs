//! Per-document find state and viewport. Search never invalidates the chat tree.
use super::*;
use gpui_kit::component::{
    scroll::ScrollbarAxis,
    text::{RangeHighlight, RenderedText, TextViewState},
};
use std::{cell::Cell, ops::Range, rc::Rc};

mod search;

const SEARCH_DELAY: Duration = Duration::from_millis(70);

actions!(
    file_preview,
    [OpenFind, NextMatch, PreviousMatch, CloseFind]
);

pub(super) fn init(cx: &mut App) {
    for context in [
        "ConduitFilePreview",
        "ConduitFilePreview > Input",
        "ConduitFilePreview > TextView",
    ] {
        cx.bind_keys([
            KeyBinding::new("ctrl-f", OpenFind, Some(context)),
            KeyBinding::new("f3", NextMatch, Some(context)),
            KeyBinding::new("shift-f3", PreviousMatch, Some(context)),
            KeyBinding::new("escape", CloseFind, Some(context)),
        ]);
    }
    for context in ["ConduitFileFind", "ConduitFileFind > Input"] {
        cx.bind_keys([
            KeyBinding::new("enter", NextMatch, Some(context)),
            KeyBinding::new("shift-enter", PreviousMatch, Some(context)),
        ]);
    }
}

pub(super) fn match_color(active: bool) -> Hsla {
    let mut color = Hsla::from(rgb(palette().warning_accent));
    color.a = if active { 0.48 } else { 0.23 };
    color
}

/// Move only when the target is outside the viewport; horizontal movement is
/// limited to unwrapped source. Called after layout, never while measuring.
fn reveal_bounds(
    scroll: &ScrollHandle,
    bounds: Bounds<Pixels>,
    horizontal: bool,
    window: &mut Window,
    _cx: &mut App,
) {
    let viewport = scroll.bounds();
    let mut offset = scroll.offset();
    let margin = px(8.);
    if bounds.top() < viewport.top() + margin || bounds.bottom() > viewport.bottom() - margin {
        offset.y += viewport.center().y - bounds.center().y;
    }
    if horizontal {
        if bounds.left() < viewport.left() + margin
            || bounds.size.width > viewport.size.width - margin * 2.
        {
            offset.x += viewport.left() + margin - bounds.left();
        } else if bounds.right() > viewport.right() - margin {
            offset.x += viewport.right() - margin - bounds.right();
        }
    } else {
        offset.x = px(0.);
    }
    let max = scroll.max_offset();
    offset.x = offset.x.clamp(-max.x, px(0.));
    offset.y = offset.y.clamp(-max.y, px(0.));
    if offset != scroll.offset() {
        scroll.set_offset(offset);
        window.refresh();
    }
}

#[derive(Clone)]
pub(super) struct CodeSearch {
    pub(super) ranges: Vec<(Range<usize>, bool)>,
    target: Option<Range<usize>>,
    pending: Rc<Cell<bool>>,
    scroll: ScrollHandle,
    horizontal: bool,
}
impl CodeSearch {
    pub(super) fn reveal(&self, layout: &TextLayout, window: &mut Window, cx: &mut App) {
        let Some(target) = &self.target else { return };
        if !self.pending.get() {
            return;
        }
        let Some(start) = layout.position_for_index(target.start) else {
            return;
        };
        let end = layout.position_for_index(target.end).unwrap_or(start);
        let width = if end.y == start.y {
            (end.x - start.x).max(px(1.))
        } else {
            px(1.)
        };
        let bounds = Bounds::new(start, size(width, layout.line_height()));
        self.pending.set(false);
        let scroll = self.scroll.clone();
        let horizontal = self.horizontal;
        window.defer(cx, move |window, cx| {
            reveal_bounds(&scroll, bounds, horizontal, window, cx)
        });
    }
}

pub(super) struct FilePreview {
    pub(super) focus: FocusHandle,
    pub(super) scroll: ScrollHandle,
    input: Entity<InputState>,
    source: SharedString,
    revision: u64,
    highlights: Arc<Vec<(Range<usize>, &'static str)>>,
    wrap: bool,
    typography: ChatTypography,
    appearance: (u32, u32),
    viewport_size: Rc<Cell<gpui_kit::Size<Pixels>>>,
    markdown: Option<Entity<TextViewState>>,
    rendered: Option<RenderedText>,
    open: bool,
    fuzzy: bool,
    hits: Vec<search::Match>,
    current: usize,
    truncated: bool,
    searching: bool,
    generation: u64,
    task: Option<Task<()>>,
    pending: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
}

impl FilePreview {
    pub(super) fn new(
        markdown: bool,
        scroll: ScrollHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("查找当前文件…"));
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search(cx);
            }
        });
        let markdown = markdown.then(|| cx.new(|cx| TextViewState::markdown("", cx)));
        let mut subscriptions = vec![subscription];
        if let Some(state) = &markdown {
            subscriptions.push(cx.observe(state, |this, state, cx| {
                let snapshot = state.read(cx).rendered_text();
                if this.rendered.as_ref() != Some(&snapshot) {
                    this.rendered = Some(snapshot);
                    this.search(cx);
                }
            }));
        }
        Self {
            focus: cx.focus_handle(),
            scroll,
            input,
            source: "".into(),
            revision: 0,
            highlights: Arc::default(),
            wrap: false,
            typography: ChatTypography {
                body_font: None,
                body_size: 15.,
                line_height: 1.5,
                code_font: Theme::global(cx).mono_font_family.clone(),
                code_size: 13.,
                code_copy: true,
                syntax_highlight: true,
                code_line_numbers: false,
            },
            appearance: (palette().canvas, palette().warning_accent),
            viewport_size: Rc::new(Cell::new(gpui_kit::Size::default())),
            markdown,
            rendered: None,
            open: false,
            fuzzy: true,
            hits: Vec::new(),
            current: 0,
            truncated: false,
            searching: false,
            generation: 0,
            task: None,
            pending: Rc::new(Cell::new(false)),
            _subscriptions: subscriptions,
        }
    }

    pub(super) fn sync(
        &mut self,
        source: SharedString,
        highlights: Arc<Vec<(Range<usize>, &'static str)>>,
        wrap: bool,
        typography: ChatTypography,
        cx: &mut Context<Self>,
    ) {
        let changed = self.source != source;
        let appearance = (palette().canvas, palette().warning_accent);
        let restyled =
            self.wrap != wrap || self.typography != typography || self.appearance != appearance;
        self.wrap = wrap;
        self.typography = typography;
        self.appearance = appearance;
        self.highlights = highlights;
        if changed {
            self.source = source;
            self.revision += 1;
            if let Some(markdown) = &self.markdown {
                self.rendered = None;
                markdown.update(cx, |state, cx| state.set_text(&self.source, cx));
            }
            self.search(cx);
        }
        if restyled {
            self.apply_highlights(cx);
            self.reveal(cx);
            cx.notify();
        }
    }

    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = (!self.open).then(|| TextSelection::selected_text(window, cx));
        self.open = true;
        if let Some(selection) = selection.filter(|s| {
            !s.trim().is_empty()
                && !s.contains(['\n', '\r'])
                && s.chars().count() <= search::MAX_QUERY_CHARS
        }) {
            self.input
                .update(cx, |input, cx| input.set_value(selection, window, cx));
        }
        self.input.update(cx, |input, cx| {
            input.focus_handle(cx).focus(window, cx);
            input.select_all(window, cx);
        });
        self.search(cx);
        cx.notify();
    }

    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        })
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || self.composing(window, cx) {
            cx.propagate();
            return;
        }
        self.open = false;
        self.generation += 1;
        self.task = None;
        self.searching = false;
        self.pending.set(false);
        if let Some(state) = &self.markdown {
            state.update(cx, |state, cx| state.clear_range_highlights(cx));
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn navigate(&mut self, previous: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        if !self.open {
            self.open_find(window, cx);
            return;
        }
        if self.hits.is_empty() {
            return;
        }
        self.current = if previous {
            (self.current + self.hits.len() - 1) % self.hits.len()
        } else {
            (self.current + 1) % self.hits.len()
        };
        self.apply_highlights(cx);
        self.reveal(cx);
        cx.notify();
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.task = None;
        self.pending.set(false);
        self.hits.clear();
        self.current = 0;
        self.truncated = false;
        self.searching = false;
        self.apply_highlights(cx);
        let query = self.input.read(cx).value().to_string();
        if !self.open || query.trim().is_empty() || query.chars().count() > search::MAX_QUERY_CHARS
        {
            cx.notify();
            return;
        }
        let text = if self.markdown.is_some() {
            let Some(rendered) = self
                .rendered
                .as_ref()
                .filter(|r| r.source() == self.source.as_ref())
            else {
                cx.notify();
                return;
            };
            SharedString::from(rendered.as_str().to_owned())
        } else {
            self.source.clone()
        };
        let generation = self.generation;
        let fuzzy = self.fuzzy;
        self.searching = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DELAY).await;
            let results = cx
                .background_executor()
                .spawn(async move { search::search(&text, &query, fuzzy) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.hits = results.hits;
                this.truncated = results.truncated;
                this.searching = false;
                this.apply_highlights(cx);
                this.reveal(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_highlights(&self, cx: &mut Context<Self>) {
        let Some(state) = &self.markdown else { return };
        let ranges =
            self.hits
                .iter()
                .enumerate()
                .flat_map(|(i, hit)| {
                    hit.spans.iter().cloned().map(move |range| {
                        RangeHighlight::new(range, match_color(i == self.current))
                    })
                })
                .collect::<Vec<_>>();
        state.update(cx, |state, cx| {
            if self.open {
                let _ = state.set_range_highlights(ranges, cx);
            } else {
                state.clear_range_highlights(cx);
            }
        });
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        self.pending.set(self.open && !self.hits.is_empty());
        if let Some(state) = &self.markdown
            && let Some(hit) = self.hits.get(self.current)
        {
            state.update(cx, |state, cx| {
                let _ = state.reveal_range(hit.range.clone(), cx);
            });
        }
    }

    fn code_search(&self, block: Range<usize>) -> Option<CodeSearch> {
        if !self.open {
            return None;
        }
        let ranges = self
            .hits
            .iter()
            .enumerate()
            .flat_map(|(i, hit)| {
                hit.spans
                    .iter()
                    .filter_map(|span| {
                        let start = span.start.max(block.start);
                        let end = span.end.min(block.end);
                        (start < end)
                            .then(|| (start - block.start..end - block.start, i == self.current))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let target = self
            .hits
            .get(self.current)
            .filter(|hit| block.contains(&hit.range.start))
            .map(|hit| hit.range.start - block.start..hit.range.end.min(block.end) - block.start);
        Some(CodeSearch {
            ranges,
            target,
            pending: self.pending.clone(),
            scroll: self.scroll.clone(),
            horizontal: self.markdown.is_none() && !self.wrap,
        })
    }

    pub(super) fn block_search(&self, source: Range<usize>) -> Option<CodeSearch> {
        let block = self.rendered.as_ref()?.range_for_source(source)?;
        self.code_search(block)
    }

    fn find_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.input.read(cx).value();
        let label = if query.chars().count() > search::MAX_QUERY_CHARS {
            "最多 256 字符".into()
        } else if self.searching {
            "搜索中…".into()
        } else if query.trim().is_empty() {
            "输入关键词".into()
        } else {
            format!(
                "{} / {}{}",
                if self.hits.is_empty() {
                    0
                } else {
                    self.current + 1
                },
                self.hits.len(),
                if self.truncated { "+" } else { "" }
            )
        };
        h_flex()
            .id("file-find-bar")
            .debug_selector(|| "file-find-bar".into())
            .key_context("ConduitFileFind")
            .w_full()
            .h(px(36.))
            .flex_shrink_0()
            .min_w_0()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(|| "file-find-input".into())
                    .child(Input::new(&self.input).h(px(28.))),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child(label),
            )
            .child(
                Button::new("file-find-fuzzy")
                    .ghost()
                    .small()
                    .compact()
                    .label("模糊")
                    .selected(self.fuzzy)
                    .toggled(self.fuzzy)
                    .tooltip("按顺序匹配字符；忽略大小写")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.fuzzy = !this.fuzzy;
                        this.search(cx);
                    })),
            )
            .child(
                Button::new("file-find-previous")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::ChevronUp)
                    .tooltip("上一个 · Shift+Enter / Shift+F3")
                    .disabled(self.hits.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(true, window, cx))),
            )
            .child(
                Button::new("file-find-next")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::ChevronDown)
                    .tooltip("下一个 · Enter / F3")
                    .disabled(self.hits.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(false, window, cx))),
            )
            .child(
                Button::new("file-find-close")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::X)
                    .tooltip("关闭查找 · Esc")
                    .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
            )
            .into_any_element()
    }
}

impl Focusable for FilePreview {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.open {
            self.input.read(cx).focus_handle(cx)
        } else {
            self.focus.clone()
        }
    }
}

impl Render for FilePreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let horizontal = self.markdown.is_none() && !self.wrap;
        let viewport_size = self.viewport_size.clone();
        let owner = cx.entity().downgrade();
        let content = if let Some(state) = &self.markdown {
            let scroll = self.scroll.clone();
            let pending = self.pending.clone();
            div()
                .debug_selector(|| "file-markdown-content".into())
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .p(px(10.))
                .child(
                    super::typography::styled_markdown_view(
                        TextView::new(state),
                        &self.typography,
                        &format!("file-preview-{}", cx.entity_id()),
                        true,
                        Some(cx.entity().downgrade()),
                    )
                    .w_full()
                    .min_w_0()
                    .on_reveal(move |bounds, window, cx| {
                        if pending.get() {
                            reveal_bounds(&scroll, bounds, false, window, cx);
                        }
                    }),
                )
                .into_any_element()
        } else {
            super::numbered_code::file_code(
                cx.entity_id().as_u64(),
                self.revision,
                self.source.clone(),
                &self.highlights,
                self.wrap,
                self.code_search(0..self.source.len()),
                cx,
            )
        };
        v_flex()
            .id("file-preview")
            .key_context("ConduitFilePreview")
            .track_focus(&self.focus)
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &OpenFind, window, cx| this.open_find(window, cx)))
            .on_action(
                cx.listener(|this, _: &NextMatch, window, cx| this.navigate(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PreviousMatch, window, cx| this.navigate(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close_find(window, cx)))
            .on_action(
                cx.listener(|_, _: &gpui_kit::component::input::Copy, window, cx| {
                    let text = TextSelection::selected_text(window, cx);
                    if text.is_empty() {
                        cx.propagate();
                    } else {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }),
            )
            .when(self.open, |view| view.child(self.find_bar(cx)))
            .child(
                div()
                    .on_children_prepainted(move |children, window, cx| {
                        if let Some(bounds) = children.first()
                            && viewport_size.replace(bounds.size) != bounds.size
                        {
                            let owner = owner.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = owner.update(cx, |view, cx| {
                                    if view.open {
                                        view.reveal(cx);
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    })
                    .id("file-content-viewport")
                    .debug_selector(|| "file-content-viewport".into())
                    .relative()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("file-content-scroll")
                            .debug_selector(|| "file-content-scroll".into())
                            .v_flex()
                            .items_start()
                            .track_scroll(&self.scroll)
                            .size_full()
                            .min_h_0()
                            .min_w_0()
                            .overflow_y_scroll()
                            .when(horizontal, |area| area.overflow_x_scroll())
                            .when(!horizontal, |area| {
                                area.overflow_x_hidden().lock_scroll_axis()
                            })
                            .on_mouse_up(MouseButton::Left, {
                                let focus = self.focus.clone();
                                move |_, window, cx| focus.focus(window, cx)
                            })
                            .when(self.markdown.is_none(), |area| {
                                area.font_family(self.typography.code_font.clone())
                                    .text_size(px(self.typography.code_size))
                                    .line_height(px(20.))
                            })
                            .child(content),
                    )
                    .scrollbar(
                        &self.scroll,
                        if horizontal {
                            ScrollbarAxis::Both
                        } else {
                            ScrollbarAxis::Vertical
                        },
                    ),
            )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
