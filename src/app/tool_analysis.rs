//! Message-version cache. Large payloads are parsed by one background worker.
use super::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Weak},
};

#[derive(Default)]
pub(super) struct Analysis {
    pub files: Vec<EditFileDiff>,
    pub plan: Vec<PlanStep>,
}

impl Analysis {
    fn parse(item: &ChatItem) -> Self {
        Self {
            files: edit_tool_diffs(item).unwrap_or_default(),
            plan: plan_steps_from_input(&item.input).unwrap_or_default(),
        }
    }
    fn bytes(&self) -> usize {
        self.files
            .iter()
            .map(|f| f.path.len() + f.note.len() + f.patch.len())
            .sum::<usize>()
            + self.plan.iter().map(|p| p.text.len()).sum::<usize>()
    }
}

struct Entry {
    identity: Weak<crate::domain::ChatItemData>,
    value: Option<Arc<Analysis>>,
    used: u64,
}

struct Contribution {
    identity: Weak<crate::domain::ChatItemData>,
    ready: bool,
    value: Option<(TurnFileStats, Vec<PlanStep>)>,
}

#[derive(Default)]
struct Turn {
    session: Option<u64>,
    entries: HashMap<String, Contribution>,
    stats: Arc<TurnFileStats>,
    file_owners: HashMap<String, usize>,
}

impl Turn {
    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key)
            && let Some((removed, _)) = entry.value
        {
            let stats = Arc::make_mut(&mut self.stats);
            stats.added -= removed.added;
            stats.removed -= removed.removed;
            for (path, diff) in removed.files {
                if let Some(file) = stats.files.get_mut(&path) {
                    file.added -= diff.added;
                    file.removed -= diff.removed;
                }
                if let Some(owners) = self.file_owners.get_mut(&path) {
                    *owners -= 1;
                    if *owners == 0 {
                        self.file_owners.remove(&path);
                        stats.files.remove(&path);
                    }
                }
            }
        }
    }

    fn finish(
        &mut self,
        key: &str,
        identity: &Weak<crate::domain::ChatItemData>,
        value: &Analysis,
    ) {
        let Some(entry) = self.entries.get(key) else {
            return;
        };
        if entry.ready || !entry.identity.ptr_eq(identity) {
            return;
        }
        // Keep the previous version visible until its replacement is ready, then
        // replace its contribution atomically without counting the edit twice.
        self.remove(key);
        let mut contribution = TurnFileStats::default();
        let stats = Arc::make_mut(&mut self.stats);
        for diff in &value.files {
            let file = contribution.files.entry(diff.path.clone()).or_default();
            file.added += diff.added;
            file.removed += diff.removed;
            contribution.added += diff.added;
            contribution.removed += diff.removed;
            let file = stats.files.entry(diff.path.clone()).or_default();
            file.added += diff.added;
            file.removed += diff.removed;
            stats.added += diff.added;
            stats.removed += diff.removed;
        }
        for path in contribution.files.keys() {
            *self.file_owners.entry(path.clone()).or_default() += 1;
        }
        self.entries.insert(
            key.to_owned(),
            Contribution {
                identity: identity.clone(),
                ready: true,
                value: Some((contribution, value.plan.clone())),
            },
        );
    }
}

#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<usize, Entry>,
    pending: VecDeque<ChatItem>,
    task: Option<Task<()>>,
    clock: u64,
    turn: Turn,
    #[cfg(test)]
    pub parses: usize,
}

impl Cache {
    pub fn turn(
        &mut self,
        session: u64,
        items: &[ChatItem],
        cx: &mut Context<ConduitApp>,
    ) -> (Arc<TurnFileStats>, Vec<PlanStep>) {
        if self.turn.session != Some(session) {
            self.turn = Turn {
                session: Some(session),
                ..Default::default()
            };
        }
        let current: HashSet<_> = items.iter().map(|item| &item.key).collect();
        let removed: Vec<_> = self
            .turn
            .entries
            .keys()
            .filter(|key| !current.contains(key))
            .cloned()
            .collect();
        for key in removed {
            self.turn.remove(&key);
        }
        let mut plan = Vec::new();
        for item in items {
            let identity = item.identity();
            let entry = self
                .turn
                .entries
                .entry(item.key.clone())
                .or_insert_with(|| Contribution {
                    identity: identity.clone(),
                    ready: false,
                    value: None,
                });
            if !entry.identity.ptr_eq(&identity) {
                entry.identity = identity.clone();
                entry.ready = false;
            }
            if !entry.ready
                && let Some(value) = self.get(item, cx)
            {
                self.turn.finish(&item.key, &identity, &value);
            }
            if let Some((_, steps)) = &self.turn.entries[&item.key].value
                && !steps.is_empty()
            {
                plan = steps.clone();
            }
        }
        (self.turn.stats.clone(), plan)
    }

    pub fn get(&mut self, item: &ChatItem, cx: &mut Context<ConduitApp>) -> Option<Arc<Analysis>> {
        let value = self.request(item, false);
        self.pump(cx);
        value
    }

    /// Interactive previews reuse completed work and queue every cache miss, even
    /// small payloads, so opening a whole turn never performs a batch of UI parses.
    pub fn get_background(
        &mut self,
        item: &ChatItem,
        cx: &mut Context<ConduitApp>,
    ) -> Option<Arc<Analysis>> {
        let value = self.request(item, true);
        self.pump(cx);
        value
    }

    fn request(&mut self, item: &ChatItem, background: bool) -> Option<Arc<Analysis>> {
        self.clock += 1;
        let identity = item.identity();
        let key = identity.as_ptr() as usize;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.clock;
            return entry.value.clone();
        }
        if self.pending.len() >= 128
            || self
                .pending
                .iter()
                .map(|m| m.input.len() + m.detail.len())
                .sum::<usize>()
                >= 16 * 1024 * 1024
        {
            return None;
        }
        if self.clock.is_multiple_of(64) || self.entries.len() >= 1024 {
            self.trim();
        }
        // Small tools keep their immediate UI; expensive first parses never run on the UI thread.
        let value = if !background && item.input.len() + item.detail.len() <= 8 * 1024 {
            #[cfg(test)]
            {
                self.parses += 1;
            }
            Some(Arc::new(Analysis::parse(item)))
        } else {
            self.pending.push_back(item.clone());
            None
        };
        self.entries.insert(
            key,
            Entry {
                identity,
                value: value.clone(),
                used: self.clock,
            },
        );
        value
    }

    fn trim(&mut self) {
        self.entries.retain(|_, e| e.identity.strong_count() != 0);
        let mut bytes: usize = self
            .entries
            .values()
            .filter_map(|e| e.value.as_ref())
            .map(|v| v.bytes())
            .sum();
        while self.entries.len() >= 1024 || bytes > 32 * 1024 * 1024 {
            let Some(key) = self
                .entries
                .iter()
                .filter(|(_, e)| e.value.is_some())
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&key) {
                bytes = bytes.saturating_sub(entry.value.unwrap().bytes());
            }
        }
    }

    fn pump(&mut self, cx: &mut Context<ConduitApp>) {
        if self.task.is_some() || self.pending.is_empty() {
            return;
        }
        let jobs: Vec<_> = self.pending.drain(..self.pending.len().min(8)).collect();
        self.task = Some(cx.spawn(async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    jobs.into_iter()
                        .map(|item| {
                            let value = Arc::new(Analysis::parse(&item));
                            (item.key.clone(), item.identity(), value)
                        })
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.tool_analysis.task = None;
                for (key, identity, value) in results {
                    #[cfg(test)]
                    {
                        this.tool_analysis.parses += 1;
                    }
                    this.tool_analysis.turn.finish(&key, &identity, &value);
                    if let Some(entry) = this
                        .tool_analysis
                        .entries
                        .get_mut(&(identity.as_ptr() as usize))
                    {
                        entry.value = Some(value);
                    }
                    // A newer version of this tool must never receive an old result.
                    if let Some(session) = this.selected_session.and_then(|id| this.session(id))
                        && let Some(index) = session
                            .messages
                            .iter()
                            .position(|m| m.identity().ptr_eq(&identity))
                    {
                        this.remeasure_message(index);
                    }
                }
                this.tool_analysis.trim();
                this.tool_analysis.pump(cx);
                cx.notify();
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::{Analysis, Cache, Contribution, Turn};
    use crate::domain::{ChatItem, ChatItemData, ItemKind};
    fn tool(lines: usize) -> ChatItem {
        ChatItemData {
            key: "tool".into(),
            kind: ItemKind::Tool,
            text: "Write".into(),
            input:
                serde_json::json!({"file_path":"large.rs","content":"中文 line\n".repeat(lines)})
                    .to_string(),
            detail: String::new(),
            status: Default::default(),
            approval: None,
            created_at: 0,
            elapsed_ms: None,
            model: None,
        }
        .into()
    }
    #[test]
    fn versions_reuse_completed_tools_and_only_reparse_changed_tool() {
        let _locale = crate::test_support::chinese();
        let mut cache = Cache::default();
        let first = tool(2);
        let mut second = tool(3);
        for _ in 0..100 {
            cache.request(&first, false).unwrap();
            cache.request(&second, false).unwrap();
        }
        assert_eq!(cache.parses, 2);
        second.input = serde_json::json!({"file_path":"large.rs","content":"new"}).to_string();
        cache.request(&first, false).unwrap();
        cache.request(&second, false).unwrap();
        assert_eq!(cache.parses, 3);
    }
    #[test]
    fn large_payload_is_queued_and_preview_counts_every_line() {
        let _locale = crate::test_support::chinese();
        let item = tool(8000);
        let mut cache = Cache::default();
        assert!(cache.request(&item, false).is_none());
        assert!(cache.request(&item, false).is_none());
        assert_eq!(cache.pending.len(), 1);
        assert_eq!(cache.parses, 0);
        let value = Analysis::parse(&item);
        assert_eq!(value.files[0].added, 8000);
        assert!(value.files[0].truncated);
        assert!(value.files[0].patch.lines().count() <= 403);
        let mut long_line = item;
        long_line.input =
            serde_json::json!({"file_path":"large.rs","content":"文".repeat(1_000_000)})
                .to_string();
        let value = Analysis::parse(&long_line);
        assert!(value.files[0].patch.len() < 66_000);
        assert_eq!(value.files[0].added, 1);
    }

    #[test]
    fn turn_statistics_update_one_contribution_and_reject_removed_versions() {
        let _locale = crate::test_support::chinese();
        let first = tool(8000);
        let mut second = tool(20);
        second.key = "second".into();
        let mut turn = Turn::default();
        for item in [&first, &second] {
            let identity = item.identity();
            turn.entries.insert(
                item.key.clone(),
                Contribution {
                    identity: identity.clone(),
                    ready: false,
                    value: None,
                },
            );
            turn.finish(&item.key, &identity, &Analysis::parse(item));
        }
        assert_eq!(turn.stats.added, 8020);
        assert_eq!(turn.stats.files.len(), 1);
        let old = turn.stats.clone();
        turn.finish(&first.key, &first.identity(), &Analysis::parse(&first));
        assert!(std::sync::Arc::ptr_eq(&old, &turn.stats));
        let updated = tool(9000);
        let entry = turn.entries.get_mut(&first.key).unwrap();
        entry.identity = updated.identity();
        entry.ready = false;
        // A late result for the old version cannot erase or replace the visible edit.
        turn.finish(&first.key, &first.identity(), &Analysis::parse(&first));
        assert_eq!(turn.stats.added, 8020);
        turn.finish(
            &updated.key,
            &updated.identity(),
            &Analysis::parse(&updated),
        );
        assert_eq!(turn.stats.added, 9020);
        assert_eq!(old.added, 8020);
        turn.remove(&first.key);
        assert_eq!(turn.stats.added, 20);
        assert_eq!(turn.stats.files.len(), 1);
        turn.finish(&first.key, &first.identity(), &Analysis::parse(&first));
        assert_eq!(turn.stats.added, 20);
        turn.remove(&second.key);
        assert!(turn.stats.files.is_empty());
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn assistant_deltas_reuse_stats_and_background_results_finish(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        use crate::app::{ConduitApp, Store, Workspace};
        cx.update(gpui_kit::init);
        let mut session = crate::app::tests::session();
        session.messages = (0..80)
            .map(|i| {
                let mut item = tool(8000);
                item.key = format!("t-{i}");
                item
            })
            .collect();
        session
            .messages
            .push(crate::transcript::tests::message("reply", "stream", 0));
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = ConduitApp::with_store(
                window,
                cx,
                Ok(Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir(),
                        sessions: vec![session],
                    }],
                    ..Default::default()
                }),
                false,
                false,
            );
            app.writable = false;
            app.selected_session = Some(1);
            app
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                assert_eq!(app.tool_analysis.turn.stats.added, 640_000);
                let parses = app.tool_analysis.parses;
                let stats = app.tool_analysis.turn.stats.clone();
                for _ in 0..20 {
                    app.session_mut(1)
                        .unwrap()
                        .messages
                        .last_mut()
                        .unwrap()
                        .text
                        .push_str(" delta");
                    app.turn_stats_tags(1, cx);
                }
                assert_eq!(app.tool_analysis.parses, parses);
                assert!(std::sync::Arc::ptr_eq(
                    &stats,
                    &app.tool_analysis.turn.stats
                ));
                // Streaming a large non-edit tool must not hide completed edits.
                let mut command = tool(0);
                command.key = "t-command".into();
                command.text = "Bash".into();
                command.input = "{}".into();
                command.detail = "command output\n".repeat(8000);
                app.session_mut(1).unwrap().messages.push(command);
                let items: Vec<_> = app
                    .session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .filter(|item| item.kind == ItemKind::Tool)
                    .cloned()
                    .collect();
                let (stats, _) = app.tool_analysis.turn(1, &items, cx);
                assert_eq!(stats.added, 640_000);
                assert_eq!(stats.files.len(), 1);
                assert!(app.turn_stats_tags(1, cx).is_some());

                // Finishing the turn replaces message identities. Keep the old
                // counts visible while these final versions are being parsed.
                for item in &mut app.session_mut(1).unwrap().messages {
                    if item.kind == ItemKind::Tool {
                        item.status = "运行中".into();
                    }
                }
                crate::app::finish_tool_cards(app.session_mut(1).unwrap(), "t", "已完成");
                let items: Vec<_> = app
                    .session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .filter(|item| item.kind == ItemKind::Tool)
                    .cloned()
                    .collect();
                let (stats, _) = app.tool_analysis.turn(1, &items, cx);
                assert_eq!(stats.added, 640_000);
                assert!(app.turn_stats_tags(1, cx).is_some());
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                assert_eq!(app.tool_analysis.turn.stats.added, 640_000);
                assert_eq!(app.tool_analysis.turn.stats.files.len(), 1);
                assert!(app.turn_stats_tags(1, cx).is_some());
                // A new turn or another session must not inherit the old totals.
                let (stats, _) = app.tool_analysis.turn(1, &[], cx);
                assert!(stats.files.is_empty());
                let (stats, _) = app.tool_analysis.turn(2, &[], cx);
                assert!(stats.files.is_empty());
            })
        });
    }
}
