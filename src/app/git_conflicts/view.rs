//! Compact three-pane merge workspace. Keep pane headers and action gutters aligned.
use super::*;

const GUTTER: f32 = 32.;

fn separator() -> Div {
    div().w(px(1.)).h(px(16.)).mx_1().bg(rgb(palette().border))
}

/// Number each block in file coordinates. Read the editor's live scroll offset
/// while painting so a long block's gutter follows keyboard and wheel scrolling.
fn line_numbers(state: Entity<EditorState>, first_line: usize, cx: &App) -> impl IntoElement {
    div()
        .relative()
        .w(px(44.))
        .h_full()
        .flex_shrink_0()
        .overflow_hidden()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(px(13.))
        .border_r_1()
        .border_color(rgb(palette().border))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let editor = state.read(cx);
                    let offset = editor.scroll_offset().y;
                    let line_height = editor.line_height().unwrap_or(px(20.));
                    let count = editor.value().lines().count();
                    let first = (f32::from(-offset) / f32::from(line_height))
                        .floor()
                        .max(0.) as usize;
                    let last = (first
                        + (f32::from(bounds.size.height) / f32::from(line_height)).ceil() as usize
                        + 1)
                    .min(count);
                    let mut style = window.text_style();
                    style.color = rgb(palette().faint).into();
                    let font_size = style.font_size.to_pixels(window.rem_size());
                    for row in first..last {
                        let label = SharedString::from((first_line + row).to_string());
                        let run = style.to_run(label.len());
                        let line = window
                            .text_system()
                            .shape_line(label, font_size, &[run], None);
                        let origin = point(
                            bounds.right() - px(8.) - line.width,
                            bounds.top() + px(8.) + offset + line_height * row as f32,
                        );
                        let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
                    }
                },
            )
            .size_full(),
        )
}

impl ConflictBrowser {
    fn header(&self, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .h(px(44.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                Icon::new(FullIconName::GitMerge)
                    .size_4()
                    .text_color(rgb(palette().muted)),
            )
            .child(div().font_semibold().child(tr("git_conflicts.title")))
            .child(separator())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(palette().muted))
                    .child(self.selected.clone().unwrap_or_default()),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "git_conflicts.files_remaining",
                        &[("count", self.paths.len().to_string())],
                    )),
            )
            .child(
                Button::new("conflict-close")
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::Close)
                    .accessibility_label(tr("git_conflicts.close"))
                    .tooltip(tr("git_conflicts.drafts_kept"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
    }

    fn file_list(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
        if compact {
            return h_flex()
                .id("conflict-file-tabs")
                .w_full()
                .h(px(40.))
                .flex_shrink_0()
                .px_2()
                .gap_1()
                .overflow_x_scroll()
                .bg(rgb(palette().sidebar_bg))
                .border_b_1()
                .border_color(rgb(palette().border))
                .children(self.paths.iter().enumerate().map(|(index, path)| {
                    let open = path.clone();
                    Button::new(SharedString::from(format!("conflict-file-{index}")))
                        .debug_selector(move || format!("conflict-file-{index}"))
                        .ghost()
                        .small()
                        .flex_shrink_0()
                        .label(path.rsplit('/').next().unwrap_or(path).to_string())
                        .accessibility_label(path.clone())
                        .tooltip(path.clone())
                        .when(self.selected.as_ref() == Some(path), |button| {
                            button.bg(rgb(palette().surface_hover))
                        })
                        .disabled(self.loading || self.saving)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.selected.as_ref() != Some(&open) {
                                this.open_file(open.clone(), None, window, cx);
                            }
                        }))
                }))
                .into_any_element();
        }
        let mut list = v_flex()
            .id("conflict-files")
            .min_h_0()
            .overflow_y_scroll()
            .bg(rgb(palette().sidebar_bg))
            .w(px(192.))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(tr("git_conflicts.files")),
            );
        for (index, path) in self.paths.iter().enumerate() {
            let selected = self.selected.as_ref() == Some(path);
            let open = path.clone();
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            list = list.child(
                Button::new(SharedString::from(format!("conflict-file-{index}")))
                    .ghost()
                    .h(px(52.))
                    .rounded_none()
                    .justify_start()
                    .text_left()
                    .accessibility_label(path.clone())
                    .tooltip(path.clone())
                    .disabled(self.loading || self.saving)
                    .debug_selector(move || format!("conflict-file-{index}"))
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .cursor_pointer()
                    .border_l_2()
                    .border_color(rgb(if selected {
                        palette().focus
                    } else {
                        palette().sidebar_bg
                    }))
                    .when(selected, |row| row.bg(rgb(palette().surface_hover)))
                    .child(
                        Icon::new(FullIconName::File)
                            .size_4()
                            .flex_shrink_0()
                            .text_color(rgb(palette().muted)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().truncate().child(name.to_string()))
                            .when(!parent.is_empty(), |column| {
                                column.child(
                                    div()
                                        .truncate()
                                        .text_size(ui_text_size(11.))
                                        .text_color(rgb(palette().muted))
                                        .child(parent.to_string()),
                                )
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.selected.as_ref() != Some(&open) {
                            this.open_file(open.clone(), None, window, cx);
                        }
                    })),
            );
        }
        list.into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let changes: Vec<_> = self
            .draft()
            .map(|draft| {
                draft
                    .blocks
                    .iter()
                    .enumerate()
                    .filter(|(_, editor)| editor.block.changed)
                    .map(|(i, _)| i)
                    .collect()
            })
            .unwrap_or_default();
        let position = changes
            .iter()
            .position(|index| *index == self.current)
            .map_or(0, |i| i + 1);
        let has_plan = self.draft().is_some_and(|draft| draft.file.plan.is_some());
        let mut row = h_flex()
            .w_full()
            .min_h(px(40.))
            .flex_shrink_0()
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(rgb(palette().border));
        for (id, label, icon, forward) in [
            (
                "merge-previous",
                "git_conflicts.previous",
                IconName::ChevronUp,
                false,
            ),
            (
                "merge-next",
                "git_conflicts.next",
                IconName::ChevronDown,
                true,
            ),
        ] {
            row = row.child(
                Button::new(id)
                    .ghost()
                    .small()
                    .compact()
                    .icon(icon)
                    .accessibility_label(tr(label))
                    .tooltip(tr(label))
                    .disabled(changes.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(forward, cx))),
            );
        }
        row.child(
            div()
                .min_w(px(40.))
                .text_center()
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().muted))
                .child(format!("{position} / {}", changes.len())),
        )
        .child(separator())
        .child(
            Button::new("merge-simple")
                .debug_selector(|| "merge-simple".into())
                .ghost()
                .small()
                .icon(IconName::Check)
                .label(tr("git_conflicts.apply_simple"))
                .disabled(self.busy(cx) || !has_plan)
                .on_click(cx.listener(|this, _, window, cx| this.apply_simple(window, cx))),
        )
        .child(div().flex_1())
        .child(
            Button::new("merge-context")
                .ghost()
                .small()
                .label(tr(if self.expanded {
                    "git_conflicts.fold_context"
                } else {
                    "git_conflicts.show_context"
                }))
                .disabled(!has_plan)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.expanded = !this.expanded;
                    cx.notify();
                })),
        )
        .child(
            Button::new("merge-reload")
                .ghost()
                .small()
                .compact()
                .icon(IconName::RefreshCw)
                .accessibility_label(tr("git_conflicts.reload"))
                .tooltip(tr("git_conflicts.reload_hint"))
                .disabled(self.busy(cx) || self.selected.is_none())
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(path) = this.selected.clone() {
                        this.drafts.remove(&path);
                        this.open_file(path, None, window, cx);
                    }
                })),
        )
    }

    fn pane_header(&self, slot: usize, cx: &mut Context<Self>) -> AnyElement {
        let result = slot == 1;
        let title = if result {
            tr("git_conflicts.result")
        } else {
            self.side_label(slot == 0, false)
        };
        let mut header = v_flex()
            .flex_1()
            .min_w_0()
            .h(px(64.))
            .justify_center()
            .px_3()
            .gap_1()
            .when(result, |header| header.bg(rgb(palette().surface_strong)))
            .child(div().truncate().font_semibold().child(title));
        if result {
            header = header.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(tr("git_conflicts.editable")),
            );
        } else {
            header = header.child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(tr("git_conflicts.readonly")),
                    )
                    .child(
                        Button::new(if slot == 0 {
                            "merge-accept-ours"
                        } else {
                            "merge-accept-theirs"
                        })
                        .ghost()
                        .small()
                        .label(self.side_label(slot == 0, true))
                        .disabled(self.busy(cx) || self.draft().is_none())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save(
                                if slot == 0 {
                                    ResolutionContent::Ours
                                } else {
                                    ResolutionContent::Theirs
                                },
                                cx,
                            );
                        })),
                    ),
            );
        }
        header.into_any_element()
    }

    fn pane_headers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().sidebar_bg))
            .child(self.pane_header(0, cx))
            .child(div().w(px(GUTTER)).flex_shrink_0())
            .child(self.pane_header(1, cx))
            .child(div().w(px(GUTTER)).flex_shrink_0())
            .child(self.pane_header(2, cx))
    }

    fn gutter(
        &self,
        index: usize,
        side: usize,
        editor: &BlockEditor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let decision = editor.block.decisions[side];
        let changed = if side == 0 {
            editor.block.ours != editor.block.base
        } else {
            editor.block.theirs != editor.block.base
        };
        let disabled = self.busy(cx) || editor.manual || !changed;
        let label = tr(if decision == Decision::Apply {
            "git_conflicts.undo_apply"
        } else {
            "git_conflicts.apply_side"
        });
        v_flex()
            .w(px(GUTTER))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .pt_1()
            .gap_1()
            .border_x_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .when(changed, |gutter| {
                gutter
                    .child(
                        Button::new(SharedString::from(format!("merge-apply-{index}-{side}")))
                            .debug_selector(move || format!("merge-apply-{index}-{side}"))
                            .ghost()
                            .small()
                            .compact()
                            .icon(if decision == Decision::Apply {
                                IconName::Undo2
                            } else if side == 0 {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronLeft
                            })
                            .accessibility_label(label.clone())
                            .tooltip(label)
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(index, side, Decision::Apply, window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("merge-ignore-{index}-{side}")))
                            .debug_selector(move || format!("merge-ignore-{index}-{side}"))
                            .ghost()
                            .small()
                            .compact()
                            .icon(if decision == Decision::Ignore {
                                IconName::Undo2
                            } else {
                                IconName::Close
                            })
                            .accessibility_label(tr("git_conflicts.ignore_side"))
                            .tooltip(tr("git_conflicts.ignore_side"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.change(index, side, Decision::Ignore, window, cx)
                            })),
                    )
            })
            .into_any_element()
    }

    fn block_caption(
        &self,
        index: usize,
        editor: &BlockEditor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pending = if editor.manual {
            !editor.reviewed
        } else {
            editor.block.pending()
        };
        h_flex()
            .h(px(28.))
            .w_full()
            .min_w_0()
            .px_2()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(if pending {
                        palette().warning_text
                    } else {
                        palette().muted
                    }))
                    .child(format!(
                        "{} · {}",
                        tr(if editor.manual {
                            "git_conflicts.manual"
                        } else if editor.block.conflict {
                            "git_conflicts.conflict"
                        } else {
                            "git_conflicts.change"
                        }),
                        tr(if pending {
                            "git_conflicts.pending"
                        } else {
                            "git_conflicts.resolved"
                        })
                    )),
            )
            .when(editor.manual && !editor.reviewed, |row| {
                row.child(
                    Button::new(SharedString::from(format!("merge-review-{index}")))
                        .debug_selector(move || format!("merge-review-{index}"))
                        .ghost()
                        .small()
                        .compact()
                        .icon(IconName::Check)
                        .tooltip(tr("git_conflicts.mark_reviewed"))
                        .accessibility_label(tr("git_conflicts.mark_reviewed"))
                        .disabled(self.busy(cx))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(draft) =
                                this.selected.as_ref().and_then(|p| this.drafts.get_mut(p))
                            {
                                draft.blocks[index].reviewed = true;
                            }
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new(SharedString::from(format!("merge-reset-{index}")))
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::Undo2)
                    .tooltip(tr("git_conflicts.reset_block"))
                    .accessibility_label(tr("git_conflicts.reset_block"))
                    .disabled(self.busy(cx))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(draft) =
                            this.selected.as_ref().and_then(|p| this.drafts.get_mut(p))
                        {
                            let editor = &mut draft.blocks[index];
                            editor.block.reset();
                            Self::sync(editor, window, cx);
                        }
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn merge_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.draft() else {
            return div().flex_1().into_any_element();
        };
        if let Some(note) = &draft.file.note {
            return v_flex()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .p_4()
                .gap_3()
                .child(
                    Icon::new(FullIconName::File)
                        .size_6()
                        .text_color(rgb(palette().muted)),
                )
                .child(note.clone())
                .child(
                    div()
                        .text_color(rgb(palette().muted))
                        .child(tr("git_conflicts.choose_whole_file")),
                )
                .into_any_element();
        }
        let mut body = v_flex()
            .id("merge-blocks")
            .debug_selector(|| "merge-blocks".into())
            .track_scroll(&self.scroll)
            .flex_1()
            .w_full()
            .min_h_0()
            .overflow_y_scroll()
            .bg(rgb(palette().canvas));
        let mut result_line = 1;
        for (index, editor) in draft.blocks.iter().enumerate() {
            let first_result_line = result_line;
            let result = editor.panes[1].read(cx).value();
            result_line += result.bytes().filter(|byte| *byte == b'\n').count();
            if !editor.block.changed
                && !editor.manual
                && !self.expanded
                && editor.block.base.lines().count() > 6
            {
                body = body.child(
                    Button::new(SharedString::from(format!("merge-context-{index}")))
                        .ghost()
                        .w_full()
                        .label(conduit::i18n::format(
                            "git_conflicts.unchanged_lines",
                            &[("count", editor.block.base.lines().count().to_string())],
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.expanded = true;
                            cx.notify();
                        })),
                );
                continue;
            }
            let changed = editor.block.changed || editor.manual;
            let unresolved = if editor.manual {
                !editor.reviewed
            } else {
                editor.block.pending()
            };
            let lines = [&editor.block.ours, result.as_ref(), &editor.block.theirs]
                .iter()
                .map(|s| s.lines().count() + 1)
                .max()
                .unwrap_or(1);
            let height = lines.clamp(2, 40) as f32 * 20. + 8.;
            let mut panes = Vec::new();
            for slot in 0..3 {
                let side_changed = match slot {
                    0 => editor.block.ours != editor.block.base,
                    2 => editor.block.theirs != editor.block.base,
                    _ => changed,
                };
                let background = if !side_changed {
                    palette().canvas
                } else if unresolved && editor.block.conflict {
                    palette().danger_bg
                } else if unresolved {
                    palette().warning_bg
                } else {
                    palette().surface
                };
                let mut pane = v_flex()
                    .id(SharedString::from(format!("merge-pane-{index}-{slot}")))
                    .debug_selector(move || format!("merge-pane-{index}-{slot}"))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(rgb(background));
                if changed {
                    pane = pane.child(if slot == 1 {
                        self.block_caption(index, editor, cx)
                    } else {
                        div()
                            .h(px(28.))
                            .w_full()
                            .px_3()
                            .flex()
                            .items_center()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "{} {}",
                                tr("git_conflicts.line"),
                                if slot == 0 {
                                    editor.block.ours_line
                                } else {
                                    editor.block.theirs_line
                                }
                            ))
                            .into_any_element()
                    });
                }
                let first_line = match slot {
                    0 => editor.block.ours_line,
                    2 => editor.block.theirs_line,
                    _ => first_result_line,
                };
                pane = pane.child(
                    h_flex()
                        .w_full()
                        .h(px(height))
                        .items_start()
                        .child(line_numbers(editor.panes[slot].clone(), first_line, cx))
                        .child(
                            div().flex_1().min_w_0().h_full().child(
                                Editor::new(&editor.panes[slot])
                                    .appearance(false)
                                    .bordered(false)
                                    .h(relative(1.))
                                    .readonly(slot != 1 || self.busy(cx))
                                    .text_size(px(13.))
                                    .line_height(px(20.))
                                    .p_0()
                                    .aria_label(tr(match slot {
                                        0 => "git_conflicts.ours",
                                        1 => "git_conflicts.result",
                                        _ => "git_conflicts.theirs",
                                    })),
                            ),
                        ),
                );
                panes.push(pane.into_any_element());
            }
            let mut panes = panes.into_iter();
            body = body.child(
                h_flex()
                    .w_full()
                    .h(px(height + if changed { 28. } else { 0. }))
                    .flex_shrink_0()
                    .items_start()
                    .when(changed, |row| {
                        row.border_y_1().border_color(rgb(if self.current == index {
                            palette().focus
                        } else {
                            palette().border
                        }))
                    })
                    .child(panes.next().unwrap())
                    .child(self.gutter(index, 0, editor, cx))
                    .child(panes.next().unwrap())
                    .child(self.gutter(index, 1, editor, cx))
                    .child(panes.next().unwrap()),
            );
        }
        // Continue column rules through the unused editor area, as in a native merge editor.
        body.child(
            h_flex()
                .w_full()
                .min_h(px(40.))
                .flex_1()
                .items_stretch()
                .child(div().flex_1())
                .child(
                    div()
                        .w(px(GUTTER))
                        .border_x_1()
                        .border_color(rgb(palette().border))
                        .bg(rgb(palette().surface_strong)),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .w(px(GUTTER))
                        .border_x_1()
                        .border_color(rgb(palette().border))
                        .bg(rgb(palette().surface_strong)),
                )
                .child(div().flex_1()),
        )
        .into_any_element()
    }

    fn footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_plan = self.draft().is_some_and(|draft| draft.file.plan.is_some());
        h_flex()
            .w_full()
            .min_h(px(48.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .border_t_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                div()
                    .size(px(6.))
                    .rounded_full()
                    .bg(rgb(if self.pending() == 0 {
                        palette().git_modified_text
                    } else {
                        palette().warning_text
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(if self.saving {
                        tr("git_conflicts.saving")
                    } else if has_plan {
                        conduit::i18n::format(
                            "git_conflicts.blocks_remaining",
                            &[("count", self.pending().to_string())],
                        )
                    } else {
                        tr("git_conflicts.choose_version")
                    }),
            )
            .child(
                Button::new("merge-close")
                    .small()
                    .label(tr("git_conflicts.close"))
                    .tooltip(tr("git_conflicts.drafts_kept"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                Button::new("merge-save")
                    .debug_selector(|| "merge-save".into())
                    .primary()
                    .small()
                    .label(tr("git_conflicts.save"))
                    .disabled(self.busy(cx) || !has_plan || self.pending() > 0)
                    .on_click(cx.listener(|this, _, _, cx| this.save_result(cx))),
            )
    }
}

impl Render for ConflictBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = f32::from(window.viewport_size().width) < 1100.;
        let mut view = v_flex()
            .id("git-conflicts-dialog")
            .debug_selector(|| "git-conflicts-dialog".into())
            .key_context("GitDiffDialog")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .bg(rgb(palette().surface))
            .text_color(rgb(palette().text))
            .text_size(ui_text_size(12.))
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &NextDiff, _, cx| this.navigate(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousDiff, _, cx| this.navigate(false, cx)))
            .child(self.header(cx));
        if let Some(error) = &self.error {
            view = view.child(
                div()
                    .w_full()
                    .px_3()
                    .py_2()
                    .bg(rgb(palette().danger_bg))
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if self.paths.is_empty() {
            return view.child(
                v_flex()
                    .flex_1()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .gap_3()
                    .child(
                        Icon::new(IconName::Check)
                            .size_6()
                            .text_color(rgb(palette().git_modified_text)),
                    )
                    .child(tr("git_conflicts.all_resolved")),
            );
        }
        if compact && self.paths.len() > 1 {
            view = view.child(self.file_list(true, cx));
        }
        let mut content = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .overflow_hidden();
        if self.loading {
            content = content.child(
                h_flex()
                    .size_full()
                    .justify_center()
                    .gap_2()
                    .child(Spinner::new())
                    .child(tr("git_conflicts.loading")),
            );
        } else {
            content = content.child(self.toolbar(cx));
            if let Some(draft) = self.draft() {
                for (show, key) in [
                    (
                        draft.file.snapshot.integration == Some(git::IntegrationState::Rebase),
                        "git_conflicts.rebase_hint",
                    ),
                    (
                        draft
                            .blocks
                            .iter()
                            .any(|block| block.block.worktree.is_some()),
                        "git_conflicts.worktree_preserved",
                    ),
                ] {
                    if show {
                        content = content.child(
                            div()
                                .w_full()
                                .px_3()
                                .py_2()
                                .text_size(ui_text_size(11.))
                                .bg(rgb(palette().warning_bg))
                                .text_color(rgb(palette().warning_text))
                                .child(tr(key)),
                        );
                    }
                }
                content = content.child(self.pane_headers(cx));
            }
            content = content.child(self.merge_body(cx)).child(self.footer(cx));
        }
        view.child(
            h_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .items_stretch()
                .when(!compact && self.paths.len() > 1, |row| {
                    row.child(self.file_list(false, cx))
                })
                .child(content),
        )
    }
}
