//! Consecutive file edits share one disclosure and one chronological diff viewer.
use super::*;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

fn status(items: &[ChatItem]) -> crate::message_status::ToolStatus {
    items
        .iter()
        .filter(|item| item.kind == ItemKind::Tool)
        .find(|item| tool_running(item))
        .or_else(|| items.iter().find(|item| tool_failed(item)))
        .or_else(|| {
            items
                .iter()
                .filter(|item| item.kind == ItemKind::Tool)
                .find(|item| !tool_succeeded(item))
        })
        .or_else(|| items.last())
        .map(|item| item.status.clone())
        .unwrap_or_default()
}

pub(super) fn card(
    session: u64,
    group: rows::Group,
    items: &[ChatItem],
    expansion: &HashSet<(u64, String)>,
    typography: &ChatTypography,
    (diffs, analysis): (&mut DiffCache, &mut tool_analysis::Cache),
    cx: &mut Context<ConduitApp>,
) -> AnyElement {
    let detail_key = (session, items[0].key.clone());
    let expanded = expansion.contains(&detail_key);
    let index = group.start;
    let title = format!(
        "{} · {}",
        conduit::i18n::text_static("chat.edit_file"),
        conduit::i18n::format("tools.changes", &[("arg0", group.count.to_string())]),
    );
    // Queue expensive parses rather than decoding the whole run on the UI thread.
    let values: Vec<_> = items
        .iter()
        .filter(|item| item.kind == ItemKind::Tool)
        .map(|item| (item, analysis.get_background(item, cx)))
        .collect();
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for (_, value) in &values {
        if let Some(value) = value {
            for file in &value.files {
                if seen.insert(file.path.as_str()) {
                    paths.push(file.path.as_str());
                }
            }
        }
    }
    let header = Button::new(SharedString::from(format!(
        "tool-{session}-{}-toggle",
        items[0].key
    )))
    .ghost()
    .w_full()
    .h(px(36.))
    .px_3()
    .cursor_pointer()
    .debug_selector(move || format!("edit-group-toggle-{session}-{index}"))
    .accessibility_label(conduit::i18n::format(
        "chat.tool_call",
        &[
            (
                "arg0",
                conduit::i18n::text_static(if expanded {
                    "chat.collapse"
                } else {
                    "chat.expand"
                })
                .to_string(),
            ),
            ("arg1", title.clone()),
        ],
    ))
    .child(
        h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_left()
                    .text_size(px(13.))
                    .child(title),
            )
            .children(paths.iter().take(2).map(|path| {
                div()
                    .min_w_0()
                    .max_w(px(180.))
                    .truncate()
                    .px_1p5()
                    .rounded(px(4.))
                    .bg(rgb(palette().surface_strong))
                    .text_size(px(10.))
                    .text_color(rgb(palette().muted))
                    .child(file_ref::display_path(Path::new(path)))
            }))
            .when(paths.len() > 2, |header| {
                header.child(format!("+{}", paths.len() - 2))
            })
            .child(
                div()
                    .debug_selector(move || format!("tool-status-{session}-{index}"))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child(status(items).label().to_string()),
            )
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .small()
                .flex_shrink_0()
                .text_color(rgb(palette().faint)),
            ),
    )
    .on_click(cx.listener({
        let key = detail_key.clone();
        move |this, _, _, cx| {
            if !this.expanded_tools.remove(&key) {
                this.expanded_tools.insert(key.clone());
            }
            this.remeasure_session_message(session, index, cx);
            cx.notify();
        }
    }));
    let mut card = v_flex()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
        .child(header);
    if expanded {
        let files: Vec<_> = values
            .iter()
            .filter_map(|(_, value)| value.as_ref())
            .flat_map(|value| value.files.iter().cloned())
            .collect();
        let mut body = v_flex()
            .debug_selector(move || format!("edit-group-body-{session}-{index}"))
            .w_full()
            .min_w_0();
        if !files.is_empty() {
            body = body.child(edit_files_content(
                &detail_key,
                &files,
                typography,
                diffs,
                cx,
            ));
        }
        if values.iter().any(|(_, value)| value.is_none()) {
            body = body.child(
                div()
                    .p_3()
                    .child(conduit::i18n::text_static("tools.reading_changes")),
            );
        }
        // Unknown formats and failed calls retain their input/output inside the group.
        for (item, value) in &values {
            if value.as_ref().is_some_and(|value| value.files.is_empty()) || tool_failed(item) {
                body = body.child(
                    div()
                        .p_3()
                        .font_family(typography.code_font.clone())
                        .text_size(px(typography.code_size))
                        .child(gpui_kit::base::SelectableText::new(
                            SharedString::from(format!("edit-group-output-{session}-{}", item.key)),
                            conduit::i18n::format(
                                "tools.input_output_result",
                                &[
                                    ("input", format_tool_text(&item.input).to_string()),
                                    ("output", format_tool_text(&item.detail).to_string()),
                                ],
                            ),
                        )),
                );
            }
        }
        card = card.child(body);
    }
    card.into_any_element()
}
