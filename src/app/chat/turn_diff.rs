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

pub(super) fn file_edits(
    session: &SessionRecord,
    turn: Option<&str>,
    path: &str,
) -> Vec<EditFileDiff> {
    turn_messages(session, turn)
        .iter()
        .filter(|item| item.kind == ItemKind::Tool)
        .filter_map(edit_tool_diffs)
        .flatten()
        .filter(|diff| diff.path == path)
        .collect()
}

pub(super) fn open(path: String, edits: Vec<EditFileDiff>, window: &mut Window, cx: &mut App) {
    if edits.is_empty() {
        return;
    }
    let mode = if window.viewport_size().width < px(800.) {
        DiffMode::Unified
    } else {
        DiffMode::Split
    };
    let preview = cx.new(|cx| TurnDiffPreview::new(path, edits, mode, cx));
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
}

impl TurnDiffPreview {
    fn new(path: String, edits: Vec<EditFileDiff>, mode: DiffMode, cx: &mut Context<Self>) -> Self {
        let parsed = DiffFile::parse(&edits[0].patch)
            .ok()
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
                "并排",
                FullIconName::Columns2,
                DiffMode::Split,
            ),
            (
                "turn-diff-unified",
                "统一",
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
                    .accessibility_label(format!("{label}显示本轮 Diff"))
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
                            .tooltip("上一次编辑")
                            .accessibility_label("上一次编辑")
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
                        .child(format!("编辑 {} / {}", self.selected + 1, self.edits.len())),
                )
                .when(self.edits.len() > 1, |row| {
                    row.child(
                        Button::new("turn-diff-next-edit")
                            .debug_selector(|| "turn-diff-next-edit".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::ChevronRight)
                            .tooltip("下一次编辑")
                            .accessibility_label("下一次编辑")
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
                        .child(
                            div()
                                .text_size(px(12.))
                                .child("无法解析差异，显示原始补丁。"),
                        )
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
                                    .child("本轮文件修改"),
                            ),
                    )
                    .child(
                        Button::new("turn-diff-close")
                            .debug_selector(|| "turn-diff-close".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Close)
                            .tooltip("关闭 (Esc)")
                            .accessibility_label("关闭文件 Diff")
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
                                .child("记录已截断"),
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
                        .child(div().flex_1().child("修改前"))
                        .child(div().flex_1().child("修改后")),
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
    }
}
