//! Session search, snippets, and highlighted results.

use super::*;

#[derive(Clone)]
pub(super) struct SearchHit {
    pub(super) session: u64,
    pub(super) provider: Provider,
    pub(super) workspace_name: String,
    pub(super) title: String,
    pub(super) archived: bool,
    pub(super) snippet: String,
}

/// Byte ranges of `query` occurrences in `text`, matched case-insensitively
/// while keeping ranges aligned to the original text's char boundaries.
pub(super) fn highlight_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    // One shadow byte maps back to the original char's byte start. Lowercasing
    // can change a char's UTF-8 length (e.g. 'İ'), so the map must be per byte.
    let mut shadow = String::new();
    let mut map: Vec<usize> = Vec::new();
    for (index, ch) in text.char_indices() {
        let lowered = ch.to_lowercase().collect::<String>();
        for _ in lowered.bytes() {
            map.push(index);
        }
        shadow.push_str(&lowered);
    }
    map.push(text.len());
    let mut ranges = Vec::new();
    let mut start = 0;
    while let Some(position) = shadow[start..].find(&query) {
        let begin = start + position;
        let end = begin + query.len();
        let (from, to) = (map[begin], map[end]);
        if from < to {
            ranges.push((from, to));
        }
        start = end;
    }
    ranges
}

#[cfg(test)]
pub(super) fn search_snippet(text: &str, query: &str) -> String {
    super::search_worker::snippet(text, query)
}

pub(super) fn highlighted_text(text: &str, query: &str, size: f32, color: u32) -> AnyElement {
    let ranges = highlight_ranges(text, query);
    let mut styled = StyledText::new(text.to_string());
    if !ranges.is_empty() {
        styled = styled.with_highlights(ranges.into_iter().map(|(start, end)| {
            (
                start..end,
                HighlightStyle {
                    color: Some(Hsla::from(rgb(palette().highlight_text))),
                    background_color: Some(Hsla::from(rgb(palette().highlight_bg))),
                    ..Default::default()
                },
            )
        }));
    }
    div()
        .text_size(px(size))
        .text_color(rgb(color))
        .child(styled)
        .into_any_element()
}

impl ConduitApp {
    pub(super) fn open_recent_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sessions: Vec<_> = self
            .store
            .ui
            .recent_sessions
            .iter()
            .filter_map(|id| {
                self.store
                    .workspaces
                    .iter()
                    .enumerate()
                    .find_map(|(workspace, item)| {
                        item.sessions
                            .iter()
                            .find(|session| session.id == *id && !session.archived)
                            .map(|session| {
                                (workspace, session.id, item.name(), session.title.clone())
                            })
                    })
            })
            .take(10)
            .collect();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let sessions = sessions.clone();
            dialog
                .w(px(560.))
                .title("最近会话")
                .content(move |content, _, _| {
                    let mut body = v_flex().w_full().gap_1();
                    for (workspace, id, workspace_name, title) in &sessions {
                        let app = view.clone();
                        let workspace = *workspace;
                        let id = *id;
                        body = body.child(
                            div()
                                .id(("recent-session", id))
                                .px_2()
                                .py_2()
                                .rounded(px(6.))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(palette().surface_hover)))
                                .on_click(move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.switch(workspace, Some(id), window, cx)
                                    });
                                    window.close_dialog(cx);
                                })
                                .child(
                                    h_flex().justify_between().child(title.clone()).child(
                                        div()
                                            .text_color(rgb(palette().faint))
                                            .child(workspace_name.clone()),
                                    ),
                                ),
                        );
                    }
                    content.child(body)
                })
        });
    }

    pub(super) fn refresh_search(&mut self, cx: &mut Context<Self>, debounce: bool) {
        let query = self.search_query.trim().to_lowercase();
        // A content refresh must let the current snapshot finish. Only a new
        // query cancels work; the next poll incrementally catches up changes.
        if self.search_ui.running && self.search_ui.requested_query == query {
            return;
        }
        self.search_ui.cancel.store(true, Ordering::Release);
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_ui.cancel = cancel.clone();
        self.search_ui.errors = 0;
        self.search_ui.generation = self.search_ui.generation.wrapping_add(1);
        let generation = self.search_ui.generation;
        self.search_ui.requested_query = query.clone();
        self.search_ui.running = true;
        self.search_ui.requested = Instant::now();
        if debounce {
            self.search_ui.hits.clear();
        }
        self.search_ui.task = Some(cx.spawn(async move |this, cx| {
            if debounce {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
            // Snapshot after debounce so intermediate keystrokes don't clone the library.
            let Ok((workspaces, revision, cache)) = this.update(cx, |this, _| {
                let revision = this.session_control.global_revision;
                this.search_ui.revision = revision;
                (
                    this.store.workspaces.clone(),
                    revision,
                    std::mem::take(&mut this.search_ui.cache),
                )
            }) else {
                return;
            };
            let result_query = query.clone();
            let (mut results, cache) = cx
                .background_executor()
                .spawn(async move {
                    super::search_worker::collect_cached(&workspaces, &query, &cancel, cache)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.search_ui.generation == generation {
                    this.search_ui.running = false;
                    this.search_ui.cache = cache;
                    this.search_ui.revision = revision;
                    // Stable sessions are published even when another session streams.
                    // Deleted/replaced bodies and stale snippets never escape this filter.
                    this.search_ui.refresh_needed = results.retain_current(&this.store.workspaces);
                    if this.search_ui.result_query != result_query
                        || this.search_ui.matches != results.matches
                    {
                        this.search_ui.result_generation =
                            this.search_ui.result_generation.wrapping_add(1);
                    }
                    this.search_ui.errors = results.errors;
                    this.search_ui.hits = results.hits;
                    this.search_ui.matches = results.matches;
                    this.search_ui.result_query = result_query;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn open_search_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_search(cx, false);
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let viewport_height = f32::from(window.viewport_size().height);
            let height = (viewport_height - 160.).clamp(320., 560.);
            let margin_top = ((viewport_height - height) * 0.5).max(24.);
            dialog
                .w(px(640.))
                .margin_top(px(margin_top))
                .title("搜索会话")
                .content(move |content, _, cx| {
                    let this = view.read(cx);
                    let input = this.search.clone();
                    let query = this.search_query.trim().to_lowercase();
                    let hits = this.search_ui.hits.clone();
                    let mut body = v_flex().w_full().gap_2();
                    body = body.child(
                        Input::new(&input).h(px(34.)).prefix(
                            Icon::new(IconName::Search)
                                .small()
                                .text_color(rgb(palette().faint)),
                        ),
                    );
                    if this.search_ui.errors > 0 {
                        body = body.child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().warning_text))
                                .child(format!(
                                    "{} 个会话的正文无法读取，搜索结果可能不完整；打开会话可重试。",
                                    this.search_ui.errors
                                )),
                        );
                    }
                    if hits.is_empty() {
                        body = body.child(
                            div()
                                .py_2()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().faint))
                                .child(if query.is_empty() {
                                    "输入会话标题或消息内容开始搜索"
                                } else {
                                    "没有匹配的会话"
                                }),
                        );
                    } else {
                        let mut list = v_flex()
                            .id("search-results")
                            .w_full()
                            .max_h(px((height - 150.).max(160.)))
                            .overflow_y_scroll()
                            .gap(px(1.));
                        for hit in hits {
                            let SearchHit {
                                session,
                                provider,
                                workspace_name,
                                title,
                                archived,
                                snippet,
                            } = hit;
                            let app = view.clone();
                            list = list.child(
                                div()
                                    .id(("search-hit", session))
                                    .w_full()
                                    .px_2()
                                    .py_2()
                                    .gap_1()
                                    .rounded(px(8.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(palette().surface_hover)))
                                    .on_click(move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            if let Some((workspace, _)) = this
                                                .session_control
                                                .locate(&this.store.workspaces, session)
                                            {
                                                this.switch(workspace, Some(session), window, cx);
                                            }
                                        });
                                        window.close_dialog(cx);
                                    })
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(provider_icon(provider))
                                            .child(div().flex_1().min_w_0().child(
                                                highlighted_text(
                                                    &title,
                                                    &query,
                                                    13.,
                                                    palette().text,
                                                ),
                                            ))
                                            .when(archived, |row| {
                                                row.child(
                                                    div()
                                                        .text_size(ui_text_size(9.))
                                                        .text_color(rgb(palette().faint))
                                                        .child("已归档"),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .text_size(ui_text_size(10.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(workspace_name),
                                            ),
                                    )
                                    .when(!snippet.is_empty(), |row| {
                                        row.child(highlighted_text(
                                            &snippet,
                                            &query,
                                            11.,
                                            palette().muted,
                                        ))
                                    }),
                            );
                        }
                        body = body.child(list);
                    }
                    content.child(body)
                })
        });
        cx.notify();
    }
}
