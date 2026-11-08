//! Async transcript loading and a bounded cache of recently opened conversations.
use super::*;
use std::collections::VecDeque;

const MAX_LOADED: usize = 8;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_READS: usize = 2;

#[derive(Default)]
pub(super) struct TranscriptUi {
    tasks: HashMap<u64, Task<()>>,
    requested: VecDeque<u64>,
    pub errors: HashMap<u64, String>,
    pub renames: HashSet<u64>,
    used: HashMap<u64, u64>,
    clock: u64,
}

impl ConduitApp {
    pub(super) fn ensure_selected_transcript(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected_session {
            self.request_transcript(id, cx);
        }
    }

    pub(super) fn request_transcript(&mut self, id: u64, cx: &mut Context<Self>) {
        self.transcripts.clock = self.transcripts.clock.wrapping_add(1);
        self.transcripts.used.insert(id, self.transcripts.clock);
        if self
            .session(id)
            .is_none_or(|session| session.transcript.loaded)
        {
            self.transcripts.errors.remove(&id);
            return;
        }
        if self.transcripts.tasks.contains_key(&id) || self.transcripts.errors.contains_key(&id) {
            return;
        }
        self.transcripts.requested.retain(|pending| *pending != id);
        self.transcripts.requested.push_front(id);
        self.pump_transcripts(cx);
    }

    fn pump_transcripts(&mut self, cx: &mut Context<Self>) {
        while self.transcripts.tasks.len() < MAX_READS {
            let Some(id) = self.transcripts.requested.pop_front() else {
                break;
            };
            let Some(session) = self
                .session(id)
                .filter(|session| !session.transcript.loaded)
                .cloned()
            else {
                continue;
            };
            if self.transcripts.errors.contains_key(&id) {
                continue;
            }
            let reference = session.transcript.reference.clone();
            let recovered = session.interrupted;
            let task = cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        crate::transcript::load(session).map_err(|error| format!("{error:#}"))
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.transcripts.tasks.remove(&id);
                    // Selection, deletion, and import may have changed while the read was running.
                    let current = this.session(id).is_some_and(|session| {
                        !session.transcript.loaded && session.transcript.reference == reference
                    });
                    if current {
                        match result {
                            Ok(loaded) => {
                                if let Some((workspace, index)) =
                                    this.session_control.locate(&this.store.workspaces, id)
                                {
                                    let session =
                                        &mut this.store.workspaces[workspace].sessions[index];
                                    session.messages = loaded.messages;
                                    session.transcript = loaded.transcript;
                                    session.interrupted = loaded.interrupted;
                                }
                                this.transcripts.errors.remove(&id);
                                if recovered {
                                    this.session_control.changed(id);
                                    this.sidebar.dirty_sessions.insert(id);
                                    this.dirty = true;
                                }
                                if this.selected_session == Some(id) {
                                    this.timeline_session = None;
                                    this.outline = MessageOutlineCache::default();
                                    this.at_bottom = true;
                                }
                                if this.transcripts.renames.remove(&id) {
                                    this.ai_rename_session(id, cx);
                                }
                            }
                            Err(error) => {
                                if this.transcripts.renames.remove(&id) {
                                    this.session_control
                                        .naming_errors
                                        .insert(id, format!("无法读取会话内容：{error}"));
                                }
                                this.transcripts.errors.insert(id, error);
                            }
                        }
                    }
                    this.ensure_selected_transcript(cx);
                    this.pump_transcripts(cx);
                    this.trim_transcripts();
                    cx.notify();
                });
            });
            self.transcripts.tasks.insert(id, task);
        }
    }

    pub(super) fn retry_transcript(&mut self, id: u64, cx: &mut Context<Self>) {
        self.transcripts.errors.remove(&id);
        self.request_transcript(id, cx);
        cx.notify();
    }

    pub(super) fn trim_transcripts(&mut self) {
        let mut count = 0;
        let mut bytes = 0;
        let mut candidates = Vec::new();
        for workspace in &self.store.workspaces {
            for session in &workspace.sessions {
                if !session.transcript.loaded || session.messages.is_empty() {
                    continue;
                }
                let size = crate::transcript::resident_bytes(&session.messages);
                count += 1;
                bytes += size;
                if self.selected_session != Some(session.id)
                    && !self.session_control.runs.contains_key(&session.id)
                    && !self.session_control.namings.contains_key(&session.id)
                    && !self.queued_messages.contains_key(&session.id)
                    && !self.transcripts.renames.contains(&session.id)
                {
                    candidates.push((
                        self.transcripts.used.get(&session.id).copied().unwrap_or(0),
                        session.id,
                        size,
                    ));
                }
            }
        }
        candidates.sort_unstable();
        for (_, id, size) in candidates {
            if count <= MAX_LOADED && bytes <= MAX_BYTES {
                break;
            }
            if let Some((workspace, index)) =
                self.session_control.locate(&self.store.workspaces, id)
                && crate::transcript::unload(&mut self.store.workspaces[workspace].sessions[index])
            {
                count -= 1;
                bytes = bytes.saturating_sub(size);
                self.tool_diffs.forget_session(id);
            }
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
