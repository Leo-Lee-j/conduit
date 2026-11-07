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
    expanded: HashSet<String>,
    pub rows: Vec<Row>,
    pub revision: std::cell::Cell<u64>,
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
        expanded: &HashSet<(u64, String)>,
    ) -> Option<Vec<Row>> {
        let id = session.map(|session| session.id);
        let mut previous = None;
        if self.session != id {
            previous = Some(std::mem::take(&mut self.rows));
            *self = Self {
                session: id,
                revision: std::cell::Cell::new(self.revision.get().wrapping_add(1)),
                ..Default::default()
            };
        }
        let Some(session) = session else {
            return previous;
        };
        let expanded: HashSet<_> = expanded
            .iter()
            .filter(|(owner, _)| *owner == session.id)
            .map(|(_, key)| key.clone())
            .collect();
        let mut rebuild = self.entries.len() != session.messages.len() || self.expanded != expanded;
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
        self.expanded = expanded;
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
            group.expanded = group.running
                || !group.replied
                || self.expanded.contains(&session.messages[start].key);
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
        let old = self.chat_rows.refresh(session, &self.expanded_tool_groups);
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

#[cfg(test)]
mod tests {
    use super::{ChatItem, HashSet, Rows, SessionRecord};
    fn message(index: usize, kind: &str, text: &str) -> ChatItem {
        serde_json::from_value(serde_json::json!({
            "key":format!("m-{index}"),"kind":kind,"text":text,"status":"已完成",
            "detail":"{\"acpKind\":\"execute\",\"exitCode\":0}"
        }))
        .unwrap()
    }
    fn conversation() -> SessionRecord {
        let mut session = crate::app::tests::session();
        session.messages = vec![message(0, "User", "prompt")];
        session
            .messages
            .extend((1..=400).map(|i| message(i, "Tool", "Bash")));
        session.messages.push(message(401, "Assistant", "reply"));
        session
    }
    #[test]
    fn collapsed_tools_have_one_row_and_streaming_reuses_classification() {
        let mut session = conversation();
        let mut rows = Rows::default();
        let mut expanded = HashSet::new();
        rows.refresh(Some(&session), &expanded);
        assert_eq!(rows.rows.len(), 3);
        assert_eq!(rows.group(1).unwrap().count, 400);
        assert_eq!(rows.row(401), Some(2));
        let count = rows.classifications;
        assert!(rows.refresh(Some(&session), &expanded).is_none());
        session.messages[401].text.push_str(" delta");
        assert!(rows.refresh(Some(&session), &expanded).is_none());
        assert_eq!(rows.classifications, count + 1);
        expanded.insert((session.id, "m-1".into()));
        rows.refresh(Some(&session), &expanded);
        assert_eq!(rows.rows.len(), 402);
        assert_eq!(rows.classifications, count + 1);
        expanded.clear();
        session.messages[200].status = "运行中".into();
        rows.refresh(Some(&session), &expanded);
        assert_eq!(rows.rows.len(), 402);
        session.messages[200].status = "已完成".into();
        rows.refresh(Some(&session), &expanded);
        assert_eq!(rows.rows.len(), 3);
    }
    #[test]
    fn placeholders_and_later_turns_do_not_collapse_unanswered_tools() {
        let mut session = conversation();
        session.messages[401] = message(401, "Assistant", "");
        session.messages.push(message(402, "User", "next turn"));
        session
            .messages
            .push(message(403, "Assistant", "next reply"));
        let mut rows = Rows::default();
        rows.refresh(Some(&session), &HashSet::new());
        assert_eq!(rows.rows.len(), 403);
        assert!(!rows.group(1).unwrap().replied);
        session.messages[401].text = "first reply".into();
        rows.refresh(Some(&session), &HashSet::new());
        assert_eq!(rows.rows.len(), 5);
        assert_eq!(rows.row(402), Some(3));
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn session_switch_restores_stable_reading_anchor_and_bottom_preference(
        cx: &mut gpui_kit::TestAppContext,
    ) {
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

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn dense_collapsed_group_builds_only_visible_rows_and_preserves_jump(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::{ConduitApp, Store, Workspace};
        use gpui_kit::AppContext;
        cx.update(gpui_kit::init);
        let mut session = conversation();
        session.messages.push(message(402, "User", "jump target"));
        session
            .messages
            .push(message(403, "Assistant", "last reply"));
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(app.scroll.item_count(), 5);
            assert!(app.performance.read(cx).mounted_messages() <= 5);
            assert!(app.performance.read(cx).mounted_messages() > 0);
        });
        assert!(cx.debug_bounds("tool-group-header-1-1").is_some());
        cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "m-402", cx)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                let before = app
                    .chat_rows
                    .message(app.scroll.logical_scroll_top().item_ix);
                app.expanded_tool_groups.insert((1, "m-1".into()));
                app.sync_chat_rows();
                assert_eq!(app.scroll.item_count(), 404);
                assert_eq!(
                    app.chat_rows
                        .message(app.scroll.logical_scroll_top().item_ix),
                    before
                );
                assert_eq!(app.chat_rows.row(402), Some(402));
                cx.notify();
            })
        });
    }
}
