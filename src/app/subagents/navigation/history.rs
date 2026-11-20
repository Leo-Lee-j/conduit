use super::*;
use crate::cli_adapter::child_history::Snapshot;

#[derive(Default)]
pub(super) struct Live {
    run: String,
    messages: Vec<ChatItem>,
    index: MessageIndex,
    finalized: HashSet<String>,
}

pub(super) struct Load {
    runtime: ChatRuntime,
    result: std::sync::mpsc::Receiver<Snapshot>,
    binding: crate::subagents::ChildBinding,
    revision: Option<(u64, u64)>,
    local_keys: Vec<String>,
}

impl ConduitApp {
    pub(in crate::app) fn apply_child_event(
        &mut self,
        root: u64,
        run: &str,
        thread: &str,
        turn: &str,
        event: Event,
    ) {
        let key = TaskKey {
            root,
            run: run.into(),
            agent: thread.into(),
        };
        if self.routed_agent(&key).is_none() {
            return;
        }
        // Observe only opened child transcripts; parent metadata remains independently bounded.
        if !self
            .session_control
            .agent_navigation
            .pages
            .keys()
            .any(|key| key.root == root && key.agent == thread)
        {
            return;
        }
        let Some(mut record) = self.session(root).cloned() else {
            return;
        };
        let (item_key, final_item) = match &event {
            Event::Delta { key, .. } => (key.clone(), false),
            Event::Text { key, .. } => (key.clone(), true),
            Event::Tool { key, status, .. } => (
                key.clone(),
                matches!(
                    status,
                    crate::message_status::ToolStatus::Completed
                        | crate::message_status::ToolStatus::Failed
                        | crate::message_status::ToolStatus::Cancelled
                        | crate::message_status::ToolStatus::Interrupted
                ),
            ),
            Event::ToolInput { key, .. } => (key.clone(), false),
            Event::ReasoningSummary { key, snapshot, .. } => (key.clone(), *snapshot),
            _ => return,
        };
        let prefix = format!("native-{turn}");
        let identity = format!("{prefix}-{item_key}");
        let baseline = self
            .saved_child(&key)
            .and_then(|id| self.session(id))
            .map(|session| {
                session
                    .messages
                    .iter()
                    .filter(|item| item.key.starts_with(&format!("{prefix}-")))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let live = self
            .session_control
            .agent_navigation
            .live
            .entry((root, thread.into()))
            .or_default();
        if live.run != run {
            *live = Live {
                run: run.into(),
                messages: baseline,
                ..Default::default()
            };
        }
        if !final_item && live.finalized.contains(&identity) {
            return;
        }
        if final_item {
            live.finalized.insert(identity);
        }
        record.messages = std::mem::take(&mut live.messages);
        super::super::super::events::apply_event_indexed(
            &mut record,
            &prefix,
            event,
            &mut live.index,
        );
        live.messages = record.messages;
        // Bound unpublished live caches; the complete native history remains reloadable.
        if crate::transcript::resident_bytes(&live.messages) > 16 * 1024 * 1024 {
            live.messages.clear();
            live.index = Default::default();
        }
        let items = live.messages.clone();
        if let Some(id) = self.saved_child(&key)
            && self
                .session(id)
                .is_some_and(|session| session.transcript.loaded)
            && let Some(session) = self.session_mut(id)
        {
            for item in items {
                if let Some(old) = session.messages.iter_mut().find(|old| old.key == item.key) {
                    *old = item;
                } else {
                    session.messages.push(item);
                }
            }
            self.invalidate_child_body(id);
            self.session_control.changed(id);
            self.dirty = true;
        }
    }

    pub(in crate::app) fn invalidate_child_body(&mut self, id: u64) {
        let count = self.session(id).map_or(0, |session| session.messages.len());
        for page in self
            .session_control
            .agent_navigation
            .pages
            .values_mut()
            .filter(|page| page.target == Target::SavedTranscript(id))
        {
            if page.count > 0 {
                page.messages.remeasure_items(0..page.count.min(count));
            }
        }
    }

    pub(super) fn child_source(&self, root: u64) -> Option<PathBuf> {
        let route = self.session_route(root).ok()?;
        let source = route
            .env
            .get("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from))
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .or_else(|| std::env::var_os("HOME"))
                    .map(|home| PathBuf::from(home).join(".codex"))
            })?;
        let source = if source.is_absolute() {
            source
        } else {
            let (workspace, _) = self.session_control.locate(&self.store.workspaces, root)?;
            self.store.workspaces[workspace].path.join(source)
        };
        Some(source.canonicalize().unwrap_or(source))
    }

    pub(super) fn history_binding(&self, key: &TaskKey) -> Option<crate::subagents::ChildBinding> {
        let root = self.session(key.root)?;
        if root.provider != Provider::Codex {
            return None;
        }
        let native = root.native_id.as_deref().filter(|id| !id.is_empty())?;
        let agent = self.routed_agent(key)?;
        let mut ancestor = agent;
        let mut visited = HashSet::new();
        while ancestor.parent_id != native {
            if !visited.insert(ancestor.id.as_str()) || ancestor.parent_id.is_empty() {
                return None;
            }
            ancestor = root
                .subagents
                .iter()
                .find(|parent| parent.run == key.run && parent.id == ancestor.parent_id)?;
        }
        Some(crate::subagents::ChildBinding {
            root: key.root,
            root_native: native.into(),
            native: key.agent.clone(),
            parent: agent.parent_id.clone(),
            source: self.child_source(key.root)?,
            idle: false,
        })
    }

    pub(in crate::app) fn request_child_history(&mut self, force: bool, cx: &mut Context<Self>) {
        let Some(key) = self.subagent_route().cloned() else {
            return;
        };
        self.read_child_history(key, force, cx);
    }

    fn child_history_conflict(&self, key: &TaskKey) -> Option<&'static str> {
        let candidates = self
            .store
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.sessions)
            .filter(|session| {
                session.is_subagent
                    && session.provider == Provider::Codex
                    && session.native_id.as_ref() == Some(&key.agent)
            })
            .count();
        if candidates > 1 {
            Some("subagents.ambiguous_identity")
        } else if candidates == 1 && self.saved_child(key).is_none() {
            Some("subagents.identity_changed")
        } else {
            None
        }
    }

    fn read_child_history(&mut self, key: TaskKey, force: bool, cx: &mut Context<Self>) {
        // Synthetic UI fixtures never launch the installed CLI implicitly.
        // Native protocol probes opt in through explicit refresh.
        if cfg!(test) && !force {
            return;
        }
        let navigation = &self.session_control.agent_navigation;
        if navigation.reads.contains_key(&key)
            || navigation.reads.len() >= 2
            || (!force
                && (navigation.refreshed.contains_key(&key)
                    || navigation.history_errors.contains_key(&key)))
        {
            return;
        }
        let Some(binding) = self.history_binding(&key) else {
            return;
        };
        let child = self.saved_child(&key);
        if let Some(reason) = self.child_history_conflict(&key) {
            self.session_control
                .agent_navigation
                .history_errors
                .insert(key, conduit::i18n::text(reason));
            return;
        }
        if child.is_some_and(|id| self.session_busy(id)) {
            return;
        }
        let Some((workspace, _)) = self
            .session_control
            .locate(&self.store.workspaces, key.root)
        else {
            return;
        };
        let executable = match cli::resolve_for_launch(Provider::Codex, &self.store.cli) {
            Ok(executable) => executable.path,
            Err(error) => {
                self.session_control
                    .agent_navigation
                    .history_errors
                    .insert(key, format!("{error:#}"));
                return;
            }
        };
        let Ok(route) = self.session_route(key.root) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let runtime = ChatRuntime::read_child_history(
            executable,
            self.store.workspaces[workspace].path.clone(),
            route,
            key.agent.clone(),
            binding.parent.clone(),
            tx,
        );
        self.session_control
            .agent_navigation
            .final_reads
            .remove(&key);
        self.session_control
            .agent_navigation
            .history_errors
            .remove(&key);
        let local_keys = child
            .and_then(|id| self.session(id))
            .map(|session| {
                session
                    .messages
                    .iter()
                    .filter(|item| !item.key.starts_with("native-"))
                    .map(|item| item.key.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.session_control.agent_navigation.reads.insert(
            key,
            Load {
                local_keys,
                runtime,
                result: rx,
                binding,
                revision: child.map(|id| (id, self.session_control.revision(id))),
            },
        );
        cx.notify();
    }

    pub(in crate::app) fn poll_child_histories(&mut self, cx: &mut Context<Self>) {
        let mut finished = Vec::new();
        for (key, load) in &self.session_control.agent_navigation.reads {
            while let Ok(event) = load.runtime.events.try_recv() {
                if let Event::Finished { error, cancelled } = event {
                    finished.push((
                        key.clone(),
                        error.or_else(|| {
                            cancelled.then(|| conduit::i18n::text("subagents.history_cancelled"))
                        }),
                    ));
                    break;
                }
            }
        }
        for (key, error) in finished {
            let load = self
                .session_control
                .agent_navigation
                .reads
                .remove(&key)
                .unwrap();
            self.session_control
                .agent_navigation
                .refreshed
                .insert(key.clone(), Instant::now());
            if self.history_binding(&key).as_ref() != Some(&load.binding) {
                continue;
            }
            if let Some(error) = error {
                self.session_control
                    .agent_navigation
                    .history_errors
                    .insert(key, error);
            } else if let Ok(snapshot) = load.result.try_recv() {
                // A local send, live update, deletion or replacement wins over this older baseline.
                let current = match load.revision {
                    Some((id, revision)) => {
                        self.saved_child(&key) == Some(id)
                            && (self.session_control.revision(id) == revision
                                || self
                                    .session_control
                                    .agent_navigation
                                    .live
                                    .contains_key(&(key.root, key.agent.clone())))
                            && !self.session_busy(id)
                            && self.session(id).is_some_and(|session| {
                                session
                                    .messages
                                    .iter()
                                    .filter(|item| !item.key.starts_with("native-"))
                                    .map(|item| item.key.clone())
                                    .collect::<Vec<_>>()
                                    == load.local_keys
                            })
                    }
                    None => self.saved_child(&key).is_none(),
                };
                if current {
                    self.install_child_history(&key, load.binding, snapshot);
                }
            }
            cx.notify();
        }
        // Only conversations the user has opened are observed. A bounded round-robin
        // refresh also runs while their page is hidden; it never writes to the CLI.
        let next = self
            .session_control
            .agent_navigation
            .refreshed
            .iter()
            .filter(|(key, at)| {
                at.elapsed() >= Duration::from_secs(5)
                    && (self.session_control.runs.contains_key(&key.root)
                        || self
                            .session_control
                            .agent_navigation
                            .final_reads
                            .contains(*key))
                    && !self
                        .session_control
                        .agent_navigation
                        .reads
                        .contains_key(*key)
                    && !self
                        .session_control
                        .agent_navigation
                        .history_errors
                        .contains_key(*key)
            })
            .min_by_key(|(_, at)| **at)
            .map(|(key, _)| key.clone());
        if let Some(key) = next {
            self.read_child_history(key, true, cx);
        }
        self.request_child_history(false, cx);
    }

    pub(super) fn install_child_history(
        &mut self,
        key: &TaskKey,
        binding: crate::subagents::ChildBinding,
        snapshot: Snapshot,
    ) {
        if self.history_binding(key).as_ref() != Some(&binding) {
            return;
        }
        if let Some(reason) = self.child_history_conflict(key) {
            self.session_control
                .agent_navigation
                .history_errors
                .insert(key.clone(), conduit::i18n::text(reason));
            return;
        }
        let mut binding = binding;
        binding.idle = snapshot.turns.last().is_none_or(|turn| {
            matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            )
        });
        let Some((workspace, _)) = self
            .session_control
            .locate(&self.store.workspaces, key.root)
        else {
            return;
        };
        let existing = self.saved_child(key);
        let Some(mut record) = self.session(key.root).cloned() else {
            return;
        };
        record.id = existing.unwrap_or(self.next_id);
        record.native_id = Some(key.agent.clone());
        record.is_subagent = true;
        record.archived = false;
        record.pinned = false;
        record.subagents.clear();
        record.branching = Default::default();
        record.messages.clear();
        record.transcript = Default::default();
        record.interrupted = false;
        record.usage = Default::default();
        record.title = self
            .routed_agent(key)
            .map(|agent| title(agent).0.to_string())
            .unwrap_or_default();
        record.created_at = snapshot.thread["createdAt"]
            .as_u64()
            .unwrap_or(record.created_at);
        let mut parser = crate::protocol::Protocol::new(
            Provider::Codex,
            self.store.workspaces[workspace]
                .path
                .to_string_lossy()
                .into_owned(),
            String::new(),
            None,
        );
        let mut index = super::super::super::state::MessageIndex::default();
        for turn in snapshot.turns {
            let turn_id = turn["id"].as_str().unwrap_or("");
            let prefix = format!("native-{turn_id}");
            for item in turn["items"].as_array().into_iter().flatten() {
                if item["type"] == "userMessage" {
                    let text = item["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|content| content["type"] == "text")
                        .filter_map(|content| content["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        record
                            .messages
                            .push(ChatItem::from(crate::domain::ChatItemData {
                                key: format!("{prefix}-{}", item["id"].as_str().unwrap_or("user")),
                                kind: ItemKind::User,
                                text,
                                created_at: turn["startedAt"].as_u64().unwrap_or(0),
                                input: String::new(),
                                detail: String::new(),
                                status: Default::default(),
                                approval: None,
                                model: None,
                                reasoning_summary: None,
                                elapsed_ms: None,
                            }));
                    }
                } else {
                    for event in parser.history_item(item) {
                        super::super::super::events::apply_event_indexed(
                            &mut record,
                            &prefix,
                            event,
                            &mut index,
                        );
                    }
                }
            }
        }
        if let Some(live) = self
            .session_control
            .agent_navigation
            .live
            .get(&(key.root, key.agent.clone()))
        {
            for item in &live.messages {
                if let Some(old) = record.messages.iter_mut().find(|old| old.key == item.key) {
                    // Full snapshots and live finals replace deltas. A saved prefix may lag
                    // the stream; it must never truncate already displayed text.
                    if live.finalized.contains(&item.key)
                        || (old.kind == ItemKind::Assistant
                            && item.text.starts_with(old.text.as_str()))
                    {
                        *old = item.clone();
                    }
                } else {
                    record.messages.push(item.clone());
                }
            }
        }
        // Seed the next live delta from the merged baseline instead of overwriting
        // a saved prefix with a new suffix when observation begins mid-message.
        let live = self
            .session_control
            .agent_navigation
            .live
            .entry((key.root, key.agent.clone()))
            .or_default();
        live.run = key.run.clone();
        live.messages = record.messages.clone();
        live.index = Default::default();
        let id = record.id;
        if let Some(old) = existing.and_then(|id| self.session_mut(id)) {
            // Preserve independently chosen child configuration and transcript storage lineage.
            old.messages = record.messages;
            old.transcript.loaded = true;
        } else {
            self.next_id += 1;
            self.store.workspaces[workspace].sessions.push(record);
        }
        self.store.child_bindings.insert(id, binding);
        self.transcripts.errors.remove(&id);
        self.invalidate_child_body(id);
        self.session_control.changed(id);
        if let Some(page) = self.session_control.agent_navigation.pages.get_mut(key) {
            page.target = Target::SavedTranscript(id);
        }
        self.persist();
    }
}
