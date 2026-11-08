//! Read-only previews of the recorded edits behind a turn's file statistics.
use super::*;
use gpui_kit::component::tooltip::Tooltip;

#[cfg(test)]
mod tests;

/// Bind the preview to the turn that supplied the clicked row, even if another
/// turn has started since the popover opened. Repeated edits stay chronological.
fn turn_messages<'a>(session: &'a SessionRecord, turn: Option<&str>) -> &'a [ChatItem] {
    let start = match turn {
        Some(key) => {
            let Some(index) = session
                .messages
                .iter()
                .position(|item| item.kind == ItemKind::User && item.key == key)
            else {
                return &[];
            };
            index + 1
        }
        None => 0,
    };
    let messages = &session.messages[start..];
    let end = messages
        .iter()
        .position(|item| item.kind == ItemKind::User)
        .unwrap_or(messages.len());
    &messages[..end]
}

/// Keep the clicked turn's immutable tool versions while the shared worker finishes.
/// Consume completed analyses in order so cache eviction cannot restart earlier work.
struct FileEdits {
    path: String,
    pending: std::collections::VecDeque<ChatItem>,
    edits: Vec<EditFileDiff>,
}

impl FileEdits {
    fn new(session: &SessionRecord, turn: Option<&str>, path: &str) -> Self {
        Self {
            path: path.to_owned(),
            pending: turn_messages(session, turn)
                .iter()
                .filter(|item| item.kind == ItemKind::Tool)
                .cloned()
                .collect(),
            edits: Vec::new(),
        }
    }

    fn advance(&mut self, cache: &mut tool_analysis::Cache, cx: &mut Context<ConduitApp>) {
        while let Some(item) = self.pending.front() {
            let Some(value) = cache.get_background(item, cx) else {
                break;
            };
            self.edits.extend(
                value
                    .files
                    .iter()
                    .filter(|diff| diff.path == self.path)
                    .cloned(),
            );
            self.pending.pop_front();
        }
    }
}

impl ConduitApp {
    pub(in crate::app) fn open_turn_diff(
        &mut self,
        session: u64,
        turn: Option<&str>,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self
            .session_control
            .locate(&self.store.workspaces, session)
            .map(|(workspace, _)| {
                self.store.workspaces[workspace]
                    .path
                    .to_string_lossy()
                    .into_owned()
            });
        let display_path = workspace_relative_path(path, root.as_deref());
        let Some(session) = self.session(session) else {
            return;
        };
        let mut edits = FileEdits::new(session, turn, path);
        edits.advance(&mut self.tool_analysis, cx);
        if edits.pending.is_empty() && edits.edits.is_empty() {
            return;
        }
        open(display_path, edits, cx.entity(), window, cx);
    }
}

fn open(
    path: String,
    edits: FileEdits,
    app: Entity<ConduitApp>,
    window: &mut Window,
    cx: &mut App,
) {
    let mode = if window.viewport_size().width < px(800.) {
        DiffMode::Unified
    } else {
        DiffMode::Split
    };
    let preview = cx.new(|cx| TurnDiffPreview::new(path, edits, mode, &app, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        let viewport = window.viewport_size();
        let width = (f32::from(viewport.width) - 48.).clamp(240., 1120.);
        let height = (f32::from(viewport.height) - 72.).clamp(200., 780.);
        let preview = preview.clone();
        dialog
            .entrance(gpui_kit::component::dialog::DialogEntrance::Fade)
            .w(px(width))
            .margin_top(px(((f32::from(viewport.height) - height) * 0.5).max(16.)))
            .p_0()
            .gap_0()
            .close_button(false)
            .content(move |content, _, _| {
                content.child(div().w_full().h(px(height)).child(preview.clone()))
            })
    });
}

struct TurnDiffPreview {
    path: String,
    edits: Vec<EditFileDiff>,
    selected: usize,
    state: Entity<DiffState>,
    parse_failed: bool,
    pending: Option<FileEdits>,
    _analysis_subscription: Subscription,
}

impl TurnDiffPreview {
    fn new(
        path: String,
        mut request: FileEdits,
        mode: DiffMode,
        app: &Entity<ConduitApp>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Updating this already-open view cannot reopen a dismissed dialog. Dropping
        // the view also drops its subscription and pending message snapshots.
        let subscription = cx.observe(app, |this, app, cx| {
            let Some(pending) = this.pending.as_mut() else {
                return;
            };
            app.update(cx, |app, cx| pending.advance(&mut app.tool_analysis, cx));
            if pending.pending.is_empty() {
                this.edits = std::mem::take(&mut pending.edits);
                this.pending = None;
                let files = this
                    .edits
                    .first()
                    .and_then(|edit| DiffFile::parse(&edit.patch).ok())
                    .filter(|files| !files.is_empty());
                this.parse_failed = files.is_none();
                this.state.update(cx, |state, cx| {
                    state.set_files(files.unwrap_or_default(), cx)
                });
                cx.notify();
            }
        });
        let edits = if request.pending.is_empty() {
            std::mem::take(&mut request.edits)
        } else {
            Vec::new()
        };
        let pending = (!request.pending.is_empty()).then_some(request);
        let parsed = edits
            .first()
            .and_then(|edit| DiffFile::parse(&edit.patch).ok())
            .filter(|files| !files.is_empty());
        let parse_failed = parsed.is_none();
        let state = cx.new(|cx| {
            DiffState::new(parsed.unwrap_or_default(), cx)
                .with_mode(mode)
                .with_context_lines(None)
        });
        Self {
            path,
            edits,
            selected: 0,
            state,
            parse_failed,
            pending,
            _analysis_subscription: subscription,
        }
    }

    fn select_edit(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.edits.get(index) else {
            return;
        };
        let parsed = DiffFile::parse(&edit.patch)
            .ok()
            .filter(|files| !files.is_empty());
        self.parse_failed = parsed.is_none();
        self.selected = index;
        self.state.update(cx, |state, cx| {
            state.set_files(parsed.unwrap_or_default(), cx)
        });
        self.focus_diff(window, cx);
        cx.notify();
    }

    fn focus_diff(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.parse_failed {
            self.state.read(cx).focus_handle(cx).focus(window, cx);
        }
    }
}

impl Render for TurnDiffPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pending.is_some() || self.edits.is_empty() {
            return v_flex()
                .id("turn-diff-preview")
                .debug_selector(|| "turn-diff-preview".into())
                .size_full()
                .min_w_0()
                .gap_3()
                .p_4()
                .bg(rgb(palette().surface))
                .text_color(rgb(palette().text))
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(div().flex_1().min_w_0().truncate().child(self.path.clone()))
                        .child(
                            Button::new("turn-diff-close")
                                .debug_selector(|| "turn-diff-close".into())
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .accessibility_label(conduit::i18n::text_static(
                                    "chat.turn_diff.close_file_diff",
                                ))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
                .child(div().debug_selector(|| "turn-diff-loading".into()).child(
                    if self.pending.is_some() {
                        conduit::i18n::text_static("chat.turn_diff.loading_file_changes")
                    } else {
                        conduit::i18n::text_static("chat.turn_diff.no_file_changes_to_display")
                    },
                ))
                .into_any_element();
        }
        let selected = self.selected;
        let edit = &self.edits[self.selected];
        let mode = self.state.read(cx).mode();
        let mut toolbar = h_flex()
            .w_full()
            .flex_shrink_0()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().sidebar_bg));
        for (id, label, icon, value) in [
            (
                "turn-diff-split",
                conduit::i18n::text_static("chat.turn_diff.side_by_side"),
                FullIconName::Columns2,
                DiffMode::Split,
            ),
            (
                "turn-diff-unified",
                conduit::i18n::text_static("chat.turn_diff.unified"),
                FullIconName::Rows3,
                DiffMode::Unified,
            ),
        ] {
            toolbar = toolbar.child(
                Button::new(id)
                    .debug_selector(move || id.into())
                    .ghost()
                    .small()
                    .label(label)
                    .icon(icon)
                    .selected(mode == value)
                    .accessibility_label(conduit::i18n::format(
                        "chat.turn_diff.show_changes_from_this_turn",
                        &[("label", label.to_string())],
                    ))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.state.update(cx, |state, cx| state.set_mode(value, cx));
                        this.focus_diff(window, cx);
                        cx.notify();
                    })),
            );
        }
        toolbar = toolbar.child(div().flex_1()).child(
            h_flex()
                .items_center()
                .gap_2()
                .when(self.edits.len() > 1, |row| {
                    row.child(
                        Button::new("turn-diff-previous-edit")
                            .debug_selector(|| "turn-diff-previous-edit".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::ChevronLeft)
                            .tooltip(conduit::i18n::text_static("chat.turn_diff.previous_edit"))
                            .accessibility_label(conduit::i18n::text_static(
                                "chat.turn_diff.previous_edit",
                            ))
                            .disabled(self.selected == 0)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_edit(this.selected.saturating_sub(1), window, cx);
                            })),
                    )
                })
                .child(
                    div()
                        .debug_selector(move || format!("turn-diff-edit-{selected}"))
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::format(
                            "chat.turn_diff.edit",
                            &[
                                ("arg0", format!("{}", self.selected + 1)),
                                ("arg1", format!("{}", self.edits.len())),
                            ],
                        )),
                )
                .when(self.edits.len() > 1, |row| {
                    row.child(
                        Button::new("turn-diff-next-edit")
                            .debug_selector(|| "turn-diff-next-edit".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::ChevronRight)
                            .tooltip(conduit::i18n::text_static("chat.turn_diff.next_edit"))
                            .accessibility_label(conduit::i18n::text_static(
                                "chat.turn_diff.next_edit",
                            ))
                            .disabled(self.selected + 1 == self.edits.len())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.select_edit(this.selected + 1, window, cx);
                            })),
                    )
                }),
        );
        let content = if self.parse_failed {
            div()
                .id("turn-diff-raw")
                .debug_selector(|| "turn-diff-raw".into())
                .size_full()
                .overflow_y_scroll()
                .p_3()
                .child(
                    v_flex()
                        .gap_3()
                        .child(div().text_size(px(12.)).child(conduit::i18n::text_static(
                            "chat.turn_diff.unable_to_parse_the_diff_showing_the",
                        )))
                        .child(gpui_kit::base::SelectableText::new(
                            "turn-diff-patch",
                            edit.patch.clone(),
                        )),
                )
                .into_any_element()
        } else {
            div()
                .debug_selector(move || {
                    if mode == DiffMode::Split {
                        "turn-diff-split-body"
                    } else {
                        "turn-diff-unified-body"
                    }
                    .into()
                })
                .size_full()
                .child(
                    Diff::new(&self.state)
                        .header_visible(false)
                        .line_number(true)
                        .syntax_highlight(true)
                        .change_indicator(DiffChangeIndicator::Bars)
                        .hunk_separator(DiffHunkSeparator::LineInfo)
                        .hover_highlight(DiffHoverHighlight::Line)
                        .soft_wrap(true),
                )
                .into_any_element()
        };
        v_flex()
            .id("turn-diff-preview")
            .debug_selector(|| "turn-diff-preview".into())
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .bg(rgb(palette().surface))
            .text_color(rgb(palette().text))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(
                        Icon::new(FullIconName::FileDiff)
                            .small()
                            .text_color(rgb(palette().muted)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .id("turn-diff-path")
                                    .tooltip({
                                        let path = self.path.clone();
                                        move |window, cx| {
                                            Tooltip::new(path.clone()).build(window, cx)
                                        }
                                    })
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .child(self.path.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().faint))
                                    .child(conduit::i18n::text_static(
                                        "chat.turn_diff.file_changes_in_this_turn",
                                    )),
                            ),
                    )
                    .child(
                        Button::new("turn-diff-close")
                            .debug_selector(|| "turn-diff-close".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Close)
                            .tooltip(conduit::i18n::text_static("chat.turn_diff.close_esc"))
                            .accessibility_label(conduit::i18n::text_static(
                                "chat.turn_diff.close_file_diff",
                            ))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
            .child(toolbar)
            .child(
                h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .text_size(px(11.))
                    .child(
                        div()
                            .text_color(rgb(palette().muted))
                            .child(edit.note.clone()),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().diff_add_text))
                            .child(format!("+{}", edit.added)),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().diff_remove_text))
                            .child(format!("-{}", edit.removed)),
                    )
                    .when(edit.truncated, |row| {
                        row.child(
                            div()
                                .debug_selector(|| "turn-diff-truncated".into())
                                .text_color(rgb(palette().warning_text))
                                .child(conduit::i18n::text_static(
                                    "chat.turn_diff.record_truncated",
                                )),
                        )
                    }),
            )
            .when(mode == DiffMode::Split && !self.parse_failed, |view| {
                view.child(
                    h_flex()
                        .w_full()
                        .flex_shrink_0()
                        .px_4()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child(
                            div()
                                .flex_1()
                                .child(conduit::i18n::text_static("chat.turn_diff.before")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(conduit::i18n::text_static("chat.turn_diff.after")),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .child(content),
            )
            .into_any_element()
    }
}
