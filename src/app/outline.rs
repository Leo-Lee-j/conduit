//! Incremental message outline and transcript navigation.

use super::*;

pub(super) struct MessageOutlineEntry {
    pub(super) key: String,
    pub(super) index: usize,
    pub(super) label: String,
}

#[derive(Default)]
pub(super) struct MessageOutlineCache {
    pub(super) session: Option<u64>,
    pub(super) indexed_len: usize,
    pub(super) entries: Vec<MessageOutlineEntry>,
    pub(super) active: Option<usize>,
    pub(super) scroll: UniformListScrollHandle,
}

impl MessageOutlineCache {
    pub(super) fn active_for_item(&self, index: usize) -> Option<usize> {
        (!self.entries.is_empty()).then(|| {
            self.entries
                .partition_point(|entry| entry.index <= index)
                .saturating_sub(1)
        })
    }

    pub(super) fn refresh(&mut self, session: Option<&SessionRecord>) {
        let id = session.map(|session| session.id);
        let count = session.map_or(0, |session| session.messages.len());
        if self.session != id || count < self.indexed_len {
            *self = Self {
                session: id,
                ..Default::default()
            };
        }
        if let Some(session) = session {
            // Submitted user messages are immutable; only inspect appended items.
            self.entries
                .extend(message_outline_entries(session, self.indexed_len));
        }
        self.indexed_len = count;
    }
}

/// Tooltip text for an outline marker: long prompts are ellipsized so the
/// popup fits to the right even at the minimum window width with a maximized
/// navigation panel. Placement preferences otherwise flip when the popup is
/// wider than the available space.
pub(super) fn outline_tooltip(label: &str) -> SharedString {
    let mut preview: String = label.chars().take(20).collect();
    if label.chars().count() > 20 {
        preview.push('…');
    }
    SharedString::from(preview)
}

pub(super) fn outline_marker_width(label: &str) -> f32 {
    // Keep short prompts visibly light while allowing longer prompts to form a
    // stronger visual rhythm. The cap prevents one verbose message from
    // pushing the outline into the chat column.
    (8. + label.chars().count().min(20) as f32 * 0.8).min(24.)
}

pub(super) fn message_outline_entries(
    session: &SessionRecord,
    start: usize,
) -> Vec<MessageOutlineEntry> {
    session
        .messages
        .iter()
        .enumerate()
        .skip(start)
        .filter(|(_, item)| item.kind == ItemKind::User)
        .map(|(index, item)| {
            // Bound preview work even when a user pastes a very large prompt.
            let mut chars = item
                .text
                .split_whitespace()
                .flat_map(|word| std::iter::once(' ').chain(word.chars()))
                .skip(1);
            let mut preview: String = chars.by_ref().take(100).collect();
            if chars.next().is_some() {
                preview.push('…');
            }
            MessageOutlineEntry {
                key: item.key.clone(),
                index,
                label: if preview.is_empty() {
                    "（空消息）".into()
                } else {
                    preview
                },
            }
        })
        .collect()
}

impl ConduitApp {
    pub(super) fn scroll_to_end(&self) {
        self.scroll.scroll_to_end();
    }

    pub(super) fn jump_to_message(&mut self, session: u64, key: &str, cx: &mut Context<Self>) {
        if self.selected_session != Some(session) {
            return;
        }
        let Some(index) = self.session(session).and_then(|session| {
            session
                .messages
                .iter()
                .position(|item| item.kind == ItemKind::User && item.key == key)
        }) else {
            return;
        };
        self.sync_chat_rows();
        let Some(row) = self.chat_rows.row(index) else {
            return;
        };
        self.scroll.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(0.),
        });
        self.at_bottom = false;
        self.pending_message_jump = Some(index);
        self.message_focus = Some((session, key.into(), Instant::now()));
        cx.notify();
    }

    pub(super) fn message_outline(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let session_id = self.outline.session.unwrap_or_default();
        let entries = &self.outline.entries;
        let top = self
            .chat_rows
            .message(self.scroll.logical_scroll_top().item_ix);
        let current = self.outline.active_for_item(top).unwrap_or(0);
        if self.outline.active != Some(current) {
            self.outline.active = Some(current);
            self.outline
                .scroll
                .scroll_to_item(current, ScrollStrategy::Nearest);
        }
        let app = cx.entity();
        let rail = uniform_list(
            ("message-outline", session_id),
            entries.len(),
            move |range, _, cx| {
                app.update(cx, |this, cx| {
                    range
                        .map(|ordinal| {
                            let entry = &this.outline.entries[ordinal];
                            let active = current == ordinal;
                            let marker_width = outline_marker_width(&entry.label);
                            let key = entry.key.clone();
                            Button::new(SharedString::from(format!("outline-{session_id}-{key}")))
                                .ghost()
                                .w_full()
                                .h(px(26.))
                                .p_0()
                                .flex_shrink_0()
                                .cursor_pointer()
                                // Always open toward the message area, not
                                // over the workspace navigation on the left.
                                .tooltip(outline_tooltip(&entry.label))
                                .tooltip_placement(Placement::Right)
                                .accessibility_label(format!(
                                    "跳转到用户消息 {}：{}",
                                    ordinal + 1,
                                    entry.label
                                ))
                                .child(
                                    h_flex().w_full().justify_end().child(
                                        div()
                                            .w(px(if active {
                                                (marker_width + 2.).min(26.)
                                            } else {
                                                marker_width
                                            }))
                                            .h(px(3.))
                                            .rounded_full()
                                            .bg(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().faint
                                            })),
                                    ),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.jump_to_message(session_id, &key, cx);
                                }))
                        })
                        .collect::<Vec<_>>()
                })
            },
        )
        .w_full()
        .h(px(entries.len() as f32 * 26.))
        .max_h(relative(1.))
        .min_h_0()
        .track_scroll(&self.outline.scroll);
        v_flex()
            .w(px(32.))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .py_4()
            .justify_center()
            .child(rail)
            .into_any_element()
    }
}
