//! Per-document find state and viewport. Search never invalidates the chat tree.
use super::*;
use gpui_kit::component::{
    scroll::ScrollbarAxis,
    text::{RangeHighlight, RenderedText, TextViewState},
};
use std::{cell::Cell, ops::Range, rc::Rc};

pub(super) mod changes;
pub(super) mod proposal;
mod review;
pub(super) use review::mark_color as change_color;
mod outline;
mod search;

const SEARCH_DELAY: Duration = Duration::from_millis(70);

/// One-based line and Unicode character column from a source citation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourcePosition {
    pub line: usize,
    pub column: usize,
}

impl SourcePosition {
    fn range(self, source: &str) -> Range<usize> {
        let start = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
            .take(self.line.max(1))
            .last()
            .unwrap_or(0);
        let line = source[start..]
            .split('\n')
            .next()
            .unwrap_or("")
            .trim_end_matches('\r');
        let column = line.char_indices().nth(self.column.saturating_sub(1));
        let offset = column.map_or(line.len(), |(offset, _)| offset);
        let end = column.map_or(offset, |(_, ch)| offset + ch.len_utf8());
        start + offset..start + end
    }
}

actions!(
    file_preview,
    [ToggleFind, NextMatch, PreviousMatch, CloseFind]
);

pub(super) fn init(cx: &mut App) {
    for context in [
        "ConduitFilePreview",
        "ConduitFilePreview > Input",
        "ConduitFilePreview > TextView",
    ] {
        cx.bind_keys([
            KeyBinding::new("ctrl-f", ToggleFind, Some(context))
                .with_meta(shortcuts::OWNED_BINDING),
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
    pub(super) changes: Vec<(Range<usize>, bool)>,
    pub(super) caret: Option<usize>,
    target: Option<Range<usize>>,
    pending: Rc<Cell<bool>>,
    scroll: ScrollHandle,
    horizontal: bool,
}
impl CodeSearch {
    pub(super) fn reveal_block(&self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        if self.target.is_none() || !self.pending.replace(false) {
            return;
        }
        // TextView drops custom-block reveals before calling on_reveal when its
        // parent owns scrolling. Use the block's laid-out bounds in that case.
        let scroll = self.scroll.clone();
        window.defer(cx, move |window, cx| {
            reveal_bounds(&scroll, bounds, false, window, cx)
        });
    }

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
    results_valid: bool,
    generation: u64,
    task: Option<Task<()>>,
    pending: Rc<Cell<bool>>,
    _subscriptions: Vec<Subscription>,
    location: Option<SourcePosition>,
    location_pending: Rc<Cell<bool>>,
    outline: outline::Outline,
    review: review::State,
    proposal: proposal::State,
    status_encoding: Option<&'static str>,
}

impl FilePreview {
    pub(super) fn new(
        markdown: bool,
        scroll: ScrollHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(conduit::i18n::text_static(
                "file_preview.find_in_current_file",
            ))
        });
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.outline.cancel_reveal();
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
                    this.reveal_location(cx);
                    this.reveal_outline(cx);
                    this.apply_highlights(cx);
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
            results_valid: false,
            generation: 0,
            task: None,
            pending: Rc::new(Cell::new(false)),
            _subscriptions: subscriptions,
            location: None,
            location_pending: Rc::new(Cell::new(false)),
            outline: outline::Outline::default(),
            review: review::State::default(),
            proposal: proposal::State::default(),
            status_encoding: None,
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
        if self.review.latest != source {
            self.stop_replay(cx);
            self.set_proposal(None, cx);
        }
        self.review.latest = source.clone();
        if self.review.playback.is_some() || self.proposal_active() {
            return;
        }
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
            self.refresh_outline(cx);
            self.search(cx);
            self.reveal_location(cx);
        }
        if restyled {
            self.apply_highlights(cx);
            self.reveal(cx);
            cx.notify();
        }
    }

    pub(super) fn set_status_encoding(
        &mut self,
        encoding: Option<&'static str>,
        cx: &mut Context<Self>,
    ) {
        if self.status_encoding != encoding {
            self.status_encoding = encoding;
            cx.notify();
        }
    }

    pub(super) fn go_to(&mut self, position: SourcePosition, cx: &mut Context<Self>) {
        self.outline.cancel_reveal();
        // Citation navigation takes precedence over a previous find reveal.
        self.open = false;
        self.generation += 1;
        self.task = None;
        self.searching = false;
        self.pending.set(false);
        self.apply_highlights(cx);
        self.location = Some(position);
        self.location_pending.set(true);
        self.reveal_location(cx);
        cx.notify();
    }

    fn location_range(&self) -> Option<Range<usize>> {
        if !self.location_pending.get() || self.source.is_empty() {
            return None;
        }
        let range = self.location?.range(&self.source);
        if self.markdown.is_none() {
            return Some(range);
        }
        let rendered = self
            .rendered
            .as_ref()
            .filter(|r| r.source() == self.source.as_ref())?;
        // Syntax and blank lines have no rendered characters. Prefer the next
        // visible content, then the preceding content at the end of the file.
        rendered
            .range_for_source(range.clone())
            .or_else(|| {
                rendered
                    .range_for_source(range.start..self.source.len())
                    .map(|range| range.start..range.start)
            })
            .or_else(|| {
                rendered
                    .range_for_source(0..range.start)
                    .map(|range| range.end..range.end)
            })
    }

    fn reveal_location(&self, cx: &mut Context<Self>) {
        if let Some(state) = &self.markdown
            && let Some(range) = self.location_range()
        {
            state.update(cx, |state, cx| {
                let _ = state.reveal_range(range, cx);
            });
        }
    }

    fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close_find(window, cx);
        } else {
            self.open_find(window, cx);
        }
    }

    pub(super) fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pause_review(cx);
        self.outline.cancel_reveal();
        self.location_pending.set(false);
        let was_open = self.open;
        let selection = (!was_open).then(|| TextSelection::selected_text(window, cx));
        let mut query_changed = false;
        self.open = true;
        if let Some(selection) = selection.filter(|s| {
            !s.trim().is_empty()
                && !s.contains(['\n', '\r'])
                && s.chars().count() <= search::MAX_QUERY_CHARS
        }) {
            query_changed = self.input.read(cx).value().as_ref() != selection;
            if query_changed {
                self.input
                    .update(cx, |input, cx| input.set_value(selection, window, cx));
            }
        }
        self.input.update(cx, |input, cx| {
            input.focus_handle(cx).focus(window, cx);
            input.select_all(window, cx);
        });
        if query_changed || !was_open {
            // Closing keeps completed results, but any query/source change or
            // cancelled search invalidates them. Refocusing never restarts work.
            if !query_changed && self.results_valid {
                self.apply_highlights(cx);
                self.reveal(cx);
            } else {
                self.search(cx);
            }
        }
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
        self.apply_highlights(cx);
        if self.focus.contains_focused(window, cx) {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn navigate(&mut self, previous: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        self.outline.cancel_reveal();
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
        self.results_valid = false;
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
                this.results_valid = true;
                this.apply_highlights(cx);
                this.reveal(cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_highlights(&self, cx: &mut Context<Self>) {
        let Some(state) = &self.markdown else { return };
        let mut ranges =
            self.hits
                .iter()
                .enumerate()
                .flat_map(|(i, hit)| {
                    hit.spans.iter().cloned().map(move |range| {
                        RangeHighlight::new(range, match_color(i == self.current))
                    })
                })
                .collect::<Vec<_>>();
        if !self.open {
            ranges.clear();
        }
        ranges.extend(
            self.review_marks()
                .into_iter()
                .filter(|(range, _)| !range.is_empty())
                .map(|(range, deleted)| RangeHighlight::new(range, review::mark_color(deleted))),
        );
        state.update(cx, |state, cx| {
            let _ = state.set_range_highlights(ranges, cx);
        });
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        // A search already in flight may finish after the user chose a heading.
        if self.outline.navigation_active() {
            return;
        }
        self.pending.set(self.open && !self.hits.is_empty());
        if self.pending.get()
            && let Some(state) = &self.markdown
            && let Some(hit) = self.hits.get(self.current)
        {
            state.update(cx, |state, cx| {
                let _ = state.reveal_range(hit.range.clone(), cx);
            });
        }
    }

    fn search_decoration(&self, block: Range<usize>) -> Option<CodeSearch> {
        if let Some(range) = self.location_range() {
            let target = (block.contains(&range.start)
                || (range.is_empty() && range.start == block.end))
                .then(|| range.start - block.start..range.end.min(block.end) - block.start);
            return target.map(|target| CodeSearch {
                ranges: Vec::new(),
                changes: Vec::new(),
                caret: None,
                target: Some(target),
                pending: self.location_pending.clone(),
                scroll: self.scroll.clone(),
                horizontal: self.markdown.is_none() && !self.wrap,
            });
        }
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
            changes: Vec::new(),
            caret: None,
            target,
            pending: self.pending.clone(),
            scroll: self.scroll.clone(),
            horizontal: self.markdown.is_none() && !self.wrap,
        })
    }

    fn code_search(&self, block: Range<usize>) -> Option<CodeSearch> {
        let marks = self.review_marks();
        let changes: Vec<_> = marks
            .into_iter()
            .filter_map(|(range, deleted)| {
                let start = range.start.max(block.start);
                let end = range.end.min(block.end);
                (start < end || (range.is_empty() && block.start <= start && start <= block.end))
                    .then(|| (start - block.start..end.max(start) - block.start, deleted))
            })
            .collect();
        let caret = self
            .review_caret()
            .filter(|caret| block.start <= *caret && *caret <= block.end)
            .map(|caret| caret - block.start);
        let mut result = self.search_decoration(block).or_else(|| {
            (!changes.is_empty() || caret.is_some()).then(|| CodeSearch {
                ranges: Vec::new(),
                changes: Vec::new(),
                caret: None,
                target: None,
                pending: Rc::new(Cell::new(false)),
                scroll: self.scroll.clone(),
                horizontal: self.markdown.is_none() && !self.wrap,
            })
        })?;
        result.changes = changes;
        result.caret = caret;
        Some(result)
    }

    pub(super) fn block_search(&self, source: Range<usize>) -> Option<CodeSearch> {
        let block = self.rendered.as_ref()?.range_for_source(source)?;
        self.code_search(block)
    }

    pub(super) fn code_block_search(&self, source: Range<usize>, code: &str) -> Option<CodeSearch> {
        let mut search = self.block_search(source.clone())?;
        if self.review.playback.is_none()
            && let Some(changes) = &self.review.changes
        {
            search.changes = changes
                .hunks
                .iter()
                .filter(|h| h.new.start < source.end && h.new.end >= source.start)
                .map(|h| (0..code.len(), h.added == 0))
                .collect();
        }
        if self.location_pending.get() {
            let position = self.location?;
            let target = position.range(&self.source);
            if !source.contains(&target.start) {
                return Some(search);
            }
            // Custom Markdown nodes map as a whole. Rebase the citation onto
            // the code's lines, excluding the fence and parser-removed indent.
            let block = &self.source[source.clone()];
            let first = block.lines().next().unwrap_or("").trim_start();
            let fenced = first.starts_with("```") || first.starts_with("~~~");
            let line = self.source[source.start..target.start]
                .matches('\n')
                .count()
                .saturating_sub(usize::from(fenced))
                + 1;
            let code_line = code.lines().nth(line - 1).unwrap_or("");
            let source_line = self.source[..target.start]
                .rfind('\n')
                .map_or(self.source.as_ref(), |start| &self.source[start + 1..])
                .lines()
                .next()
                .unwrap_or("");
            let indent = source_line
                .strip_suffix(code_line)
                .map_or(0, |prefix| prefix.chars().count());
            let range = SourcePosition {
                line,
                column: position.column.saturating_sub(indent).max(1),
            }
            .range(code);
            search.target = Some(range);
            search.ranges.clear();
        }
        Some(search)
    }

    fn find_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let query = self.input.read(cx).value();
        let label = if query.chars().count() > search::MAX_QUERY_CHARS {
            Some(conduit::i18n::text_static("file_preview.up_to_256_characters").to_owned())
        } else if self.searching {
            Some(conduit::i18n::text_static("file_preview.searching").to_owned())
        } else if query.trim().is_empty() {
            None
        } else {
            Some(format!(
                "{} / {}{}",
                if self.hits.is_empty() {
                    0
                } else {
                    self.current + 1
                },
                self.hits.len(),
                if self.truncated { "+" } else { "" }
            ))
        };
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let fuzzy = Button::new("file-find-fuzzy")
            .ghost()
            .xsmall()
            .compact()
            .icon(FullIconName::ListFilter)
            .accessibility_label(conduit::i18n::text_static("file_preview.fuzzy_matching"))
            .selected(self.fuzzy)
            .toggled(self.fuzzy)
            .tooltip(if self.fuzzy {
                conduit::i18n::text_static(
                    "file_preview.fuzzy_matching_on_click_for_exact_matching",
                )
            } else {
                conduit::i18n::text_static(
                    "file_preview.fuzzy_matching_off_click_to_match_characters",
                )
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.fuzzy = !this.fuzzy;
                this.search(cx);
                this.input.read(cx).focus_handle(cx).focus(window, cx);
                cx.stop_propagation();
            }));
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
                    .child(
                        Input::new(&self.input)
                            .small()
                            .focus_bordered(false)
                            .when(focused, |input| input.border_color(Theme::global(cx).ring))
                            .suffix(fuzzy),
                    ),
            )
            .when_some(label, |bar, label| {
                bar.child(
                    div()
                        .debug_selector(|| "file-find-count".into())
                        .flex_shrink_0()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(label),
                )
            })
            .child(
                Button::new("file-find-previous")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::ChevronUp)
                    .tooltip(conduit::i18n::text_static(
                        "file_preview.previous_shift_enter_shift_f3",
                    ))
                    .disabled(self.hits.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(true, window, cx))),
            )
            .child(
                Button::new("file-find-next")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::ChevronDown)
                    .tooltip(conduit::i18n::text_static("file_preview.next_enter_f3"))
                    .disabled(self.hits.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.navigate(false, window, cx))),
            )
            .child(
                Button::new("file-find-close")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::X)
                    .tooltip(conduit::i18n::format(
                        "file_preview.close_find_esc",
                        &[("arg0", shortcuts::hint("file_find", cx).to_string())],
                    ))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let review_bar = self.review_bar(window, cx);
        let horizontal = self.markdown.is_none() && !self.wrap;
        let viewport_size = self.viewport_size.clone();
        let owner = cx.entity().downgrade();
        let outline_expanded = self.outline_expanded();
        let reserve_outline = outline_expanded && self.outline_wide();
        let content = if let Some(state) = &self.markdown {
            let scroll = self.scroll.clone();
            let pending = self.pending.clone();
            let location_pending = self.location_pending.clone();
            let outline_pending = self.outline.pending.clone();
            div()
                .debug_selector(|| "file-markdown-content".into())
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .p(px(10.))
                .when(reserve_outline, |content| {
                    content.pr(px(outline::WIDTH + 32.))
                })
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
                        if outline_pending.replace(false) {
                            outline::reveal_heading(&scroll, bounds, window);
                        } else if location_pending.replace(false) || pending.get() {
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
            .on_action(cx.listener(|this, _: &ToggleFind, window, cx| this.toggle_find(window, cx)))
            .on_action(
                cx.listener(|this, _: &NextMatch, window, cx| this.navigate(false, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PreviousMatch, window, cx| this.navigate(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &CloseFind, window, cx| this.close_find(window, cx)))
            .on_action(
                cx.listener(|this, _: &gpui_kit::component::input::Copy, window, cx| {
                    this.pause_review(cx);
                    let text = TextSelection::selected_text(window, cx);
                    if text.is_empty() {
                        cx.propagate();
                    } else {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }),
            )
            .when_some(review_bar, |view, bar| view.child(bar))
            .when(self.open, |view| view.child(self.find_bar(window, cx)))
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
                                    }
                                    if view.open || view.markdown.is_some() {
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    })
                    .id("file-content-viewport")
                    .debug_selector(|| "file-content-viewport".into())
                    .on_scroll_wheel(cx.listener(|this, _, _, cx| this.pause_review(cx)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.pause_review(cx)),
                    )
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
                    )
                    .when(!self.outline.headings.is_empty(), |viewport| {
                        viewport.child(self.render_outline(outline_expanded, cx))
                    }),
            )
            .when_some(self.status_encoding, |view, encoding| {
                view.child(super::files::file_status(conduit::i18n::format(
                    "files.read_only_lines",
                    &[
                        ("arg0", encoding.to_string()),
                        ("arg1", self.source.lines().count().to_string()),
                    ],
                )))
            })
    }
}

impl FilePreview {
    pub(super) fn refresh_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_placeholder(
                conduit::i18n::text_static("file_preview.find_in_current_file"),
                window,
                cx,
            )
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::SourcePosition;

    #[test]
    fn citation_positions_preserve_unicode_and_clamp_to_source() {
        let source = "标题\r\n中😀文\r\nlast";
        for (line, column, expected) in [
            (1, 1, 0..3),
            (2, 2, 11..15),
            (2, 3, 15..18),
            (2, 99, 18..18),
            (99, 1, 20..21),
            (usize::MAX, usize::MAX, 24..24),
        ] {
            assert_eq!(SourcePosition { line, column }.range(source), expected);
        }
        assert_eq!(SourcePosition { line: 1, column: 1 }.range(""), 0..0);
        assert_eq!(SourcePosition { line: 2, column: 1 }.range("a\n"), 2..2);
    }
}
