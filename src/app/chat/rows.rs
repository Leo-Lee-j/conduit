//! Display rows are independent of transcript positions: a collapsed group is one row.
use super::super::*;
use std::sync::Weak;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::app) struct Group {
    pub start: usize,
    pub end: usize,
    pub count: usize,
    pub failures: usize,
    pub running: bool,
    pub replied: bool,
    pub expanded: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Shape {
    empty: bool,
    activity: bool,
    running: bool,
    failed: bool,
    user: bool,
    reply: bool,
}
struct Entry {
    identity: Weak<crate::domain::ChatItemData>,
    key: String,
    shape: Shape,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct Row {
    pub message: usize,
    pub key: String,
    pub group: Option<Group>,
}
#[derive(Default)]
pub(in crate::app) struct Rows {
    session: Option<u64>,
    entries: Vec<Entry>,
    expansion: HashMap<String, bool>,
    pub rows: Vec<Row>,
    pub revision: std::cell::Cell<u64>,
    pub mapping_revision: u64,
    #[cfg(test)]
    pub classifications: usize,
}
impl Rows {
    pub fn invalidate(&self) {
        self.revision.set(self.revision.get().wrapping_add(1));
    }
    pub fn message(&self, row: usize) -> usize {
        self.rows
            .get(row)
            .map_or(self.entries.len(), |row| row.message)
    }
    pub fn row(&self, message: usize) -> Option<usize> {
        (!self.rows.is_empty()).then(|| {
            self.rows
                .partition_point(|row| row.message <= message)
                .saturating_sub(1)
        })
    }
    pub fn group(&self, message: usize) -> Option<Group> {
        self.row(message).and_then(|row| self.rows[row].group)
    }
    pub fn refresh(
        &mut self,
        session: Option<&SessionRecord>,
        expansion: &HashMap<(u64, String), bool>,
    ) -> Option<Vec<Row>> {
        let id = session.map(|session| session.id);
        let mut previous = None;
        if self.session != id {
            previous = Some(std::mem::take(&mut self.rows));
            *self = Self {
                session: id,
                revision: std::cell::Cell::new(self.revision.get().wrapping_add(1)),
                mapping_revision: self.mapping_revision.wrapping_add(1),
                ..Default::default()
            };
        }
        let Some(session) = session else {
            return previous;
        };
        let expansion: HashMap<_, _> = expansion
            .iter()
            .filter(|((owner, _), _)| *owner == session.id)
            .map(|((_, key), expanded)| (key.clone(), *expanded))
            .collect();
        let mut rebuild =
            self.entries.len() != session.messages.len() || self.expansion != expansion;
        if rebuild {
            self.invalidate();
        }
        self.entries.truncate(session.messages.len());
        for (index, item) in session.messages.iter().enumerate() {
            let identity = item.identity();
            if self
                .entries
                .get(index)
                .is_some_and(|entry| entry.identity.ptr_eq(&identity))
            {
                continue;
            }
            self.invalidate();
            #[cfg(test)]
            {
                self.classifications += 1;
            }
            let shape = Shape {
                empty: is_empty_chat_placeholder(item),
                activity: is_groupable_activity(item),
                running: tool_running(item),
                failed: matches!(item.kind, ItemKind::Tool | ItemKind::Reasoning)
                    && tool_failed(item),
                user: item.kind == ItemKind::User,
                reply: item.kind == ItemKind::Assistant && !item.text.trim().is_empty(),
            };
            // A replaced item may keep its shape but have a different stable key.
            rebuild |= self
                .entries
                .get(index)
                .is_none_or(|entry| entry.shape != shape)
                || self
                    .entries
                    .get(index)
                    .is_some_and(|entry| entry.key != item.key);
            let entry = Entry {
                identity,
                key: item.key.clone(),
                shape,
            };
            if index < self.entries.len() {
                self.entries[index] = entry;
            } else {
                self.entries.push(entry);
            }
        }
        if !rebuild {
            return previous;
        }
        self.expansion = expansion;
        let old = std::mem::take(&mut self.rows);
        if previous.is_none() {
            previous = Some(old);
        }
        // Compute reply availability once, without crossing a user-turn boundary.
        let mut replies = vec![false; self.entries.len() + 1];
        let mut replied = false;
        for (index, entry) in self.entries.iter().enumerate().rev() {
            if entry.shape.user {
                replied = false;
            }
            replies[index] = replied;
            replied |= entry.shape.reply;
        }
        let mut index = 0;
        while index < self.entries.len() {
            let shape = self.entries[index].shape;
            if shape.empty {
                index += 1;
                continue;
            }
            if !shape.activity {
                self.rows.push(Row {
                    message: index,
                    key: session.messages[index].key.clone(),
                    group: None,
                });
                index += 1;
                continue;
            }
            let start = index;
            let mut group = Group {
                start,
                ..Default::default()
            };
            while index < self.entries.len() {
                let shape = self.entries[index].shape;
                if !shape.activity && !shape.empty {
                    break;
                }
                if shape.activity {
                    group.count += 1;
                    group.failures += usize::from(shape.failed);
                    group.running |= shape.running;
                    group.end = index + 1;
                }
                index += 1;
            }
            // replies[end - 1] includes a following assistant but never a later user turn.
            group.replied = replies[group.end - 1];
            // Automatic defaults never override a user's explicit disclosure choice.
            group.expanded = self
                .expansion
                .get(&session.messages[start].key)
                .copied()
                .unwrap_or(group.running || !group.replied);
            if group.expanded {
                for message in start..group.end {
                    if self.entries[message].shape.empty {
                        continue;
                    }
                    self.rows.push(Row {
                        message,
                        key: session.messages[message].key.clone(),
                        group: Some(group),
                    });
                }
            } else {
                self.rows.push(Row {
                    message: start,
                    key: session.messages[start].key.clone(),
                    group: Some(group),
                });
            }
        }
        if previous.as_ref().is_some_and(|old| {
            old.len() != self.rows.len()
                || old
                    .iter()
                    .zip(&self.rows)
                    .any(|(a, b)| a.message != b.message || a.key != b.key)
        }) {
            self.mapping_revision = self.mapping_revision.wrapping_add(1);
        }
        previous
    }
}

impl ConduitApp {
    pub(in crate::app) fn sync_chat_rows(&mut self) {
        let top = self.scroll.logical_scroll_top();
        let anchor = self
            .chat_rows
            .rows
            .get(top.item_ix)
            .map(|row| row.key.clone());
        let anchor_message = self.chat_rows.message(top.item_ix);
        let session = self
            .selected_session
            .and_then(|id| self.session_control.locate(&self.store.workspaces, id))
            .map(|(w, s)| &self.store.workspaces[w].sessions[s]);
        let old = self.chat_rows.refresh(session, &self.tool_group_expansion);
        let rows = &self.chat_rows.rows;
        if self.timeline_session != self.selected_session {
            self.scroll.reset(rows.len());
            self.timeline_session = self.selected_session;
        } else if let Some(old) = old
            && old != *rows
        {
            let prefix = old.iter().zip(rows).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            self.scroll
                .splice(prefix..old.len() - suffix, rows.len() - prefix - suffix);
            if !self.at_bottom
                && let Some(anchor) = anchor
            {
                let row = rows
                    .iter()
                    .position(|row| row.key == anchor)
                    .or_else(|| self.chat_rows.row(anchor_message));
                if let Some(row) = row {
                    self.scroll.scroll_to(ListOffset {
                        item_ix: row,
                        offset_in_item: top.offset_in_item,
                    });
                }
            }
        }
        if self.at_bottom || self.pending_message_jump.is_some() {
            self.restore_reading_position = false;
        }
        if self.restore_reading_position && !self.chat_rows.rows.is_empty() {
            if let Some((key, offset, _)) = self
                .selected_session
                .and_then(|id| self.reading_positions.get(&id))
            {
                let row = self
                    .chat_rows
                    .rows
                    .iter()
                    .position(|row| row.key == *key)
                    .or_else(|| {
                        self.selected_session
                            .and_then(|id| self.session(id))
                            .and_then(|session| {
                                session.messages.iter().position(|item| item.key == *key)
                            })
                            .and_then(|message| self.chat_rows.row(message))
                    });
                if let Some(row) = row {
                    self.scroll.scroll_to(ListOffset {
                        item_ix: row,
                        offset_in_item: *offset,
                    });
                }
            }
            self.restore_reading_position = false;
        }
    }
    pub(in crate::app) fn remeasure_message(&self, index: usize) {
        // Async tool analysis and expansion can change layout without replacing a message.
        self.chat_rows.invalidate();
        if let Some(row) = self.chat_rows.row(index)
            && row < self.scroll.item_count()
        {
            self.scroll.remeasure_items(row..row + 1);
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::ChatItem;
    fn message(index: usize, kind: &str, text: &str) -> ChatItem {
        serde_json::from_value(serde_json::json!({
            "key":format!("m-{index}"),"kind":kind,"text":text,"status":"已完成",
            "detail":"{\"acpKind\":\"execute\",\"exitCode\":0}"
        }))
        .unwrap()
    }

    #[gpui_kit::test]
    fn session_switch_restores_stable_reading_anchor_and_bottom_preference(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "reading-anchor");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.session_mut(1).unwrap().messages =
                    (0..120).map(|i| message(i, "User", "history")).collect();
                app.session_mut(2).unwrap().messages = (0..20)
                    .map(|i| message(i, "User", "other history"))
                    .collect();
                app.sync_chat_rows();
                app.at_bottom = false;
                app.scroll.scroll_to(super::ListOffset {
                    item_ix: 40,
                    offset_in_item: super::px(7.),
                });
                for _ in 0..10 {
                    app.switch(1, Some(2), window, cx);
                    app.sync_chat_rows();
                    assert!(app.at_bottom);
                    app.switch(0, Some(1), window, cx);
                    app.sync_chat_rows();
                    let top = app.scroll.logical_scroll_top();
                    assert_eq!(app.chat_rows.rows[top.item_ix].key, "m-40");
                    assert_eq!(top.offset_in_item, super::px(7.));
                    assert!(!app.at_bottom);
                }
                app.switch(1, Some(2), window, cx);
                app.sync_chat_rows();
                app.session_mut(1)
                    .unwrap()
                    .messages
                    .insert(0, message(999, "User", "inserted"));
                app.switch(0, Some(1), window, cx);
                app.sync_chat_rows();
                assert_eq!(
                    app.chat_rows.rows[app.scroll.logical_scroll_top().item_ix].key,
                    "m-40"
                );
                app.jump_to_message(1, "m-80", cx);
                app.sync_chat_rows();
                assert_eq!(
                    app.chat_rows.rows[app.scroll.logical_scroll_top().item_ix].key,
                    "m-80"
                );
            })
        });
    }
}
