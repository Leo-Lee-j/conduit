//! A derived view of durable tool records. Restarting needs no second file store.
use super::*;
use crate::app::tools::{EditFileDiff, EditFileStatus, edit_tool_diffs_with_limit};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

#[derive(Default)]
pub(super) struct Cache {
    sources: Vec<Weak<crate::domain::ChatItemData>>,
    locale: Option<conduit::i18n::Locale>,
    value: Option<Arc<Changes>>,
    task: Option<Task<()>>,
}

struct FileChange {
    status: EditFileStatus,
    added: usize,
    removed: usize,
    latest: Arc<EditFileDiff>,
}

#[derive(Default)]
struct Changes {
    files: BTreeMap<String, FileChange>,
    added: usize,
    removed: usize,
}

impl Changes {
    fn collect(messages: &[ChatItem], root: &str) -> Self {
        let mut changes = Self::default();
        for item in messages {
            if item.kind != ItemKind::Tool || !tool_succeeded(item) || tool_failed(item) {
                continue;
            }
            // Every listed file can open its preview, including files beyond the
            // eight inline previews retained by an ordinary tool card.
            for diff in edit_tool_diffs_with_limit(item, usize::MAX).unwrap_or_default() {
                let path = workspace_relative_path(&diff.path, Some(root));
                changes.added += diff.added;
                changes.removed += diff.removed;
                let diff = Arc::new(diff);
                let file = changes.files.entry(path).or_insert_with(|| FileChange {
                    status: diff.status,
                    added: 0,
                    removed: 0,
                    latest: diff.clone(),
                });
                // A file created in this conversation remains new after later edits.
                if file.status != EditFileStatus::Added || diff.status != EditFileStatus::Modified {
                    file.status = diff.status;
                }
                file.added += diff.added;
                file.removed += diff.removed;
                file.latest = diff;
            }
        }
        changes
    }
}

pub(super) fn refresh(
    state: &Entity<Sections>,
    messages: &[ChatItem],
    root: &str,
    cx: &mut Context<ConduitApp>,
) {
    // Assistant streaming must not restart file analysis. Weak identities also let
    // cold transcripts leave memory without being retained by this panel.
    let tools: Vec<_> = messages
        .iter()
        .filter(|m| m.kind == ItemKind::Tool && tool_succeeded(m))
        .cloned()
        .collect();
    let locale = conduit::i18n::locale();
    let cache = &state.read(cx).files;
    if cache.locale.as_ref() == Some(&locale)
        && cache.sources.len() == tools.len()
        && cache
            .sources
            .iter()
            .zip(&tools)
            .all(|(a, b)| a.ptr_eq(&b.identity()))
    {
        return;
    }
    let sources = tools.iter().map(ChatItem::identity).collect();
    let weak = state.downgrade();
    let root = root.to_owned();
    let task = cx.spawn(async move |app, cx| {
        let value = cx
            .background_executor()
            .spawn(async move { Arc::new(Changes::collect(&tools, &root)) })
            .await;
        let _ = weak.update(cx, |state, _| {
            state.files.value = Some(value);
            state.files.task = None;
        });
        let _ = app.update(cx, |_, cx| cx.notify());
    });
    state.update(cx, |state, _| {
        state.files.sources = sources;
        state.files.locale = Some(locale);
        state.files.task = Some(task);
    });
}

pub(super) fn render(
    session: u64,
    loaded: bool,
    state: &Entity<Sections>,
    cx: &Context<ConduitApp>,
) -> impl IntoElement {
    let sections = state.read(cx);
    let changes = sections.files.value.as_deref();
    let loading = !loaded || sections.files.task.is_some();
    let summary = changes.map_or_else(
        || text("session_data.loading_history").into(),
        |changes| {
            conduit::i18n::format(
                "session_data.files_summary",
                &[
                    ("count", changes.files.len().to_string()),
                    ("added", changes.added.to_string()),
                    ("removed", changes.removed.to_string()),
                ],
            )
        },
    );
    let mut card = card()
        .id("session-data-files-card")
        .test_support()
        .debug_selector(|| "session-data-files-card".into())
        .child(section(
            "session-data-files-toggle",
            "session_data.files",
            summary,
            sections.files_expanded,
            state,
            |state| state.files_expanded = !state.files_expanded,
            cx,
        ));
    if !sections.files_expanded {
        return card;
    }
    let mut rows = disclosure_details().gap_1();
    if let Some(changes) = changes {
        if changes.files.is_empty() && !loading {
            rows = rows.child(caption(text("session_data.files_empty")));
        }
        for (index, (path, change)) in changes.files.iter().enumerate() {
            let (label, color) = match change.status {
                EditFileStatus::Added => ("new", palette().diff_add_text),
                EditFileStatus::Modified => ("changed", palette().highlight_text),
                EditFileStatus::Deleted => ("deleted", palette().diff_remove_text),
            };
            let diff = change.latest.clone();
            let path = path.clone();
            let display_path = path.clone();
            rows = rows.child(
                Button::new(("session-data-file", index))
                    .debug_selector(move || format!("session-data-file-{index}"))
                    .ghost()
                    .w_full()
                    .min_w_0()
                    .h(px(32.))
                    .px_1()
                    .tooltip(format!(
                        "{}\n{}",
                        path,
                        text("session_data.files_open_diff")
                    ))
                    .accessibility_label(format!(
                        "{} · {} · +{} -{}",
                        path, label, change.added, change.removed
                    ))
                    .cursor_pointer()
                    .on_click(cx.listener(move |app, event: &ClickEvent, window, cx| {
                        if event.click_count() == 2 || matches!(event, ClickEvent::Keyboard(_)) {
                            app.open_recorded_file_diff(
                                session,
                                path.clone(),
                                (*diff).clone(),
                                window,
                                cx,
                            );
                        }
                    }))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Icon::new(FullIconName::FileDiff)
                                    .size(px(12.))
                                    .text_color(rgb(palette().muted)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_left()
                                    .text_size(ui_text_size(12.))
                                    .child(display_path),
                            )
                            .child(
                                caption(label)
                                    .text_color(rgb(color))
                                    .px_1()
                                    .rounded(px(4.))
                                    .bg(rgb(color).opacity(0.1))
                                    .flex_shrink_0(),
                            )
                            .child(
                                caption(format!("+{}", change.added))
                                    .text_color(rgb(palette().diff_add_text))
                                    .flex_shrink_0(),
                            )
                            .child(
                                caption(format!("-{}", change.removed))
                                    .text_color(rgb(palette().diff_remove_text))
                                    .flex_shrink_0(),
                            ),
                    ),
            );
        }
    }
    if loading {
        rows = rows.child(caption(text("session_data.files_loading")));
    }
    card = card.child(rows);
    card
}

#[cfg(test)]
mod tests;
