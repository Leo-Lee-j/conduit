//! Incremental message outline and transcript navigation.

use super::*;

#[derive(Clone)]
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
    fn track_item(&mut self, index: usize) -> bool {
        let active = self.active_for_item(index);
        if self.active == active {
            return false;
        }
        self.active = active;
        if let Some(active) = active {
            self.scroll.scroll_to_item(active, ScrollStrategy::Nearest);
        }
        true
    }

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

impl shell::TimelineSource {
    pub(in crate::app) fn with_outline<R>(
        &self,
        app: &ConduitApp,
        read: impl FnOnce(&MessageOutlineCache) -> R,
    ) -> R {
        match self {
            Self::Main => read(&app.outline),
            Self::Session(state) => read(&state.borrow().outline),
        }
    }

    fn with_outline_mut<R>(
        &self,
        app: &mut ConduitApp,
        update: impl FnOnce(&mut MessageOutlineCache) -> R,
    ) -> R {
        match self {
            Self::Main => update(&mut app.outline),
            Self::Session(state) => update(&mut state.borrow_mut().outline),
        }
    }

    pub(in crate::app) fn refresh_outline(&self, app: &mut ConduitApp) {
        match self {
            Self::Main => {
                let session = app
                    .store
                    .workspaces
                    .iter()
                    .flat_map(|workspace| &workspace.sessions)
                    .find(|session| Some(session.id) == app.selected_session);
                app.outline.refresh(session);
            }
            Self::Session(state) => {
                let mut state = state.borrow_mut();
                let session = app.session(state.session);
                state.outline.refresh(session);
            }
        }
    }

    fn outline_top(&self, app: &ConduitApp) -> usize {
        match self {
            Self::Main => app
                .chat_rows
                .message(app.scroll.logical_scroll_top().item_ix),
            Self::Session(state) => {
                let state = state.borrow();
                state
                    .rows
                    .message(state.scroll.logical_scroll_top().item_ix)
            }
        }
    }

    /// Called after wheel layout settles as well as when a gesture starts.
    pub(in crate::app) fn refresh_active(
        &self,
        app: &mut ConduitApp,
        cx: &mut Context<ConduitApp>,
    ) {
        let top = self.outline_top(app);
        if self.with_outline_mut(app, |outline| outline.track_item(top)) {
            cx.notify();
        }
    }

    pub(in crate::app) fn jump_to_outline_message(
        &self,
        session: u64,
        key: &str,
        app: &mut ConduitApp,
        cx: &mut Context<ConduitApp>,
    ) {
        // A queued click from an old main-session rail must not navigate its replacement.
        if self.id(app) != Some(session) {
            return;
        }
        let Self::Session(state) = self else {
            app.jump_to_message(session, key, cx);
            return;
        };
        let Some(index) = app.session(session).and_then(|session| {
            session
                .messages
                .iter()
                .position(|item| item.kind == ItemKind::User && item.key == key)
        }) else {
            return;
        };
        self.refresh(app);
        self.refresh_outline(app);
        {
            let mut state = state.borrow_mut();
            let Some(row) = state.rows.row(index) else {
                return;
            };
            state.scroll.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
            state.at_bottom = false;
            state.pending_message_jump = Some(index);
        }
        self.refresh_active(app, cx);
        cx.notify();
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
                    conduit::i18n::text_static("outline.empty_message").into()
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
        self.restore_reading_position = false;
        self.pending_message_jump = Some(index);
        self.message_focus = Some((session, key.into(), Instant::now()));
        cx.notify();
    }

    pub(super) fn message_outline(
        &mut self,
        source: shell::TimelineSource,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let top = source.outline_top(self);
        let (session_id, count, current, scroll) = source.with_outline_mut(self, |outline| {
            outline.track_item(top);
            (
                outline.session.unwrap_or_default(),
                outline.entries.len(),
                outline.active,
                outline.scroll.clone(),
            )
        });
        let app = cx.entity();
        let rail =
            uniform_list(
                ("message-outline", session_id),
                count,
                move |range, _, cx| {
                    app.update(cx, |this, cx| {
                        if source.id(this) != Some(session_id) {
                            return Vec::new();
                        }
                        range
                            .filter_map(|ordinal| {
                                let entry = source.with_outline(this, |outline| {
                                    outline.entries.get(ordinal).cloned()
                                })?;
                                let active = current == Some(ordinal);
                                let marker_width = outline_marker_width(&entry.label);
                                let key = entry.key.clone();
                                let source = source.clone();
                                Some(
                                    Button::new(SharedString::from(format!(
                                        "outline-{session_id}-{key}"
                                    )))
                                    .debug_selector(move || {
                                        format!("outline-marker-{session_id}-{ordinal}")
                                    })
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
                                    .accessibility_label(conduit::i18n::format(
                                        "outline.jump_to_user_message",
                                        &[
                                            ("arg0", format!("{}", ordinal + 1)),
                                            ("arg1", entry.label.to_string()),
                                        ],
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
                                        source.jump_to_outline_message(session_id, &key, this, cx);
                                    })),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                },
            )
            .w_full()
            .h(px(count as f32 * 26.))
            .max_h(relative(1.))
            .min_h_0()
            .track_scroll(&scroll);
        v_flex()
            .id(("message-outline-rail", session_id))
            .debug_selector(move || format!("message-outline-rail-{session_id}"))
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

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
