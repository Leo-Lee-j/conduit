//! Read-only Codex history hydration. Never resumes, subscribes to, or starts a thread.
use super::*;
use crate::{domain::Provider, protocol::Protocol};
use serde_json::json;
use std::{
    collections::HashSet,
    sync::mpsc,
    time::{Duration, Instant},
};

pub(crate) enum Snapshot {
    Codex {
        thread: Value,
        turns: Vec<Value>,
    },
    Native {
        created_at: u64,
        messages: Vec<crate::domain::ChatItem>,
        idle: bool,
    },
}

pub(crate) struct Reader {
    protocol: Protocol,
    cwd: PathBuf,
    thread: String,
    parent: String,
    metadata: Option<Value>,
    turns: Vec<Value>,
    cursors: HashSet<String>,
    turn_ids: HashSet<String>,
    bytes: usize,
    request: u64,
    initialized: bool,
    started: Instant,
    result: mpsc::Sender<Snapshot>,
}

impl Reader {
    pub(crate) fn new(
        cwd: PathBuf,
        route: crate::providers::RuntimeRoute,
        thread: String,
        parent: String,
        result: mpsc::Sender<Snapshot>,
    ) -> Self {
        Self {
            protocol: Protocol::new(
                Provider::Codex,
                cwd.to_string_lossy().into_owned(),
                String::new(),
                None,
            )
            .with_options(Default::default(), route),
            cwd,
            thread,
            parent,
            metadata: None,
            turns: Vec::new(),
            cursors: HashSet::new(),
            turn_ids: HashSet::new(),
            bytes: 0,
            request: 1,
            initialized: false,
            started: Instant::now(),
            result,
        }
    }
    fn page(&mut self, cursor: Option<&str>, step: &mut Step) {
        self.request += 1;
        step.writes.push(json!({"id":self.request,"method":"thread/turns/list","params":{
            "threadId":self.thread,"limit":50,"sortDirection":"asc","itemsView":"full","cursor":cursor
        }}));
    }
    fn accept_response(&mut self, value: Value, step: &mut Step) -> anyhow::Result<()> {
        if value["id"].as_u64() != Some(self.request) {
            return Ok(());
        }
        if let Some(error) = value.get("error") {
            anyhow::bail!("{error}");
        }
        let result = value
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("Missing history response"))?;
        if !self.initialized {
            self.initialized = true;
            step.writes.push(json!({"method":"initialized"}));
            self.request += 1;
            step.writes
                .push(json!({"id":self.request,"method":"thread/read","params":{
                    "threadId":self.thread,"includeTurns":false
                }}));
        } else if self.metadata.is_none() {
            let thread = &result["thread"];
            let parent = thread["parentThreadId"].as_str().or_else(|| {
                thread
                    .pointer("/source/subAgent/thread_spawn/parent_thread_id")
                    .and_then(Value::as_str)
            });
            let cwd = thread["cwd"].as_str().map(PathBuf::from);
            anyhow::ensure!(
                thread["id"].as_str() == Some(self.thread.as_str())
                    && parent == Some(self.parent.as_str())
                    && cwd
                        .as_ref()
                        .is_some_and(|cwd| cwd.canonicalize().unwrap_or_else(|_| cwd.clone())
                            == self.cwd.canonicalize().unwrap_or_else(|_| self.cwd.clone())),
                "Child history identity or workspace does not match"
            );
            self.metadata = Some(thread.clone());
            self.page(None, step);
        } else {
            let turns = result["data"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("Missing history page"))?;
            for turn in turns {
                let id = turn["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("Missing turn identity"))?;
                anyhow::ensure!(
                    turn["items"].is_array()
                        && turn.get("itemsView").is_none_or(|view| view == "full"),
                    "Incomplete child history page"
                );
                anyhow::ensure!(self.turn_ids.insert(id.into()), "Repeated history turn");
                self.bytes += turn.to_string().len();
                anyhow::ensure!(
                    self.bytes <= 64 * 1024 * 1024 && self.turns.len() < 10000,
                    "Child history exceeds the read budget"
                );
                self.turns.push(turn.clone());
            }
            if let Some(cursor) = result["nextCursor"].as_str() {
                anyhow::ensure!(
                    !cursor.is_empty() && self.cursors.insert(cursor.into()),
                    "Repeated history cursor"
                );
                self.page(Some(cursor), step);
            } else {
                self.result.send(Snapshot::Codex {
                    thread: self.metadata.take().unwrap(),
                    turns: std::mem::take(&mut self.turns),
                })?;
                step.done = true;
            }
        }
        Ok(())
    }
}

impl CliAdapter for Reader {
    fn launch(&mut self) -> anyhow::Result<LaunchSpec> {
        self.protocol.prepare()?;
        self.started = Instant::now();
        Ok(LaunchSpec {
            args: self.protocol.args(),
            environment: self.protocol.environment().clone(),
            input: InitialInput::Json(self.protocol.initialize()),
            completion: Completion::ProtocolEvent,
        })
    }
    fn accept(&mut self, value: Value) -> Step {
        let mut step = Step::new();
        if let Err(error) = self.accept_response(value, &mut step) {
            step.writes.clear();
            step.done = true;
            step.error = Some(format!("{error:#}"));
        }
        step
    }
    fn startup_pending(&self) -> bool {
        !self.initialized
    }
    fn poll(&mut self) -> anyhow::Result<Vec<Value>> {
        anyhow::ensure!(
            self.started.elapsed() < Duration::from_secs(45),
            "Child history read timed out"
        );
        Ok(Vec::new())
    }
    fn automatic_approval(&self, _: &Approval) -> Option<bool> {
        Some(false)
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        self.protocol.approval_response(approval, allow)
    }
    fn user_input_response(
        &self,
        _: &Approval,
        _: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        None
    }
    fn enable_full_access(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reader() -> (Reader, mpsc::Receiver<Snapshot>) {
        let (tx, rx) = mpsc::channel();
        (
            Reader::new(
                PathBuf::from("C:/work"),
                Default::default(),
                "child".into(),
                "parent".into(),
                tx,
            ),
            rx,
        )
    }
    fn metadata() -> Value {
        json!({"id":2,"result":{"thread":{"id":"child","parentThreadId":"parent","cwd":"C:/work"}}})
    }
    #[test]
    fn child_history_reads_all_pages_without_starting_or_resuming() {
        let (mut reader, rx) = reader();
        let init = reader.accept(json!({"id":1,"result":{}}));
        assert_eq!(init.writes[1]["method"], "thread/read");
        let first = reader.accept(metadata());
        assert_eq!(first.writes[0]["method"], "thread/turns/list");
        assert_eq!(first.writes[0]["params"]["itemsView"], "full");
        let second = reader.accept(json!({"id":3,"result":{"data":[{"id":"a","items":[],"itemsView":"full"}],"nextCursor":"next"}}));
        assert!(!second.done);
        assert_eq!(second.writes[0]["params"]["cursor"], "next");
        assert!(
            reader
                .accept(json!({"id":4,"result":{"data":[{"id":"b","items":[]}],"nextCursor":null}}))
                .done
        );
        let Snapshot::Codex { turns, .. } = rx.try_recv().unwrap() else {
            panic!("expected Codex history")
        };
        assert_eq!(turns.len(), 2);
    }
    #[test]
    fn child_history_rejects_identity_mismatch_partial_pages_and_cursor_loops() {
        for bad in ["parent", "workspace", "partial", "loop"] {
            let (mut reader, rx) = reader();
            reader.accept(json!({"id":1,"result":{}}));
            let mut meta = metadata();
            if bad == "parent" {
                meta["result"]["thread"]["parentThreadId"] = json!("other");
            }
            if bad == "workspace" {
                meta["result"]["thread"]["cwd"] = json!("C:/other");
            }
            let mut step = reader.accept(meta);
            if bad == "partial" {
                step = reader.accept(
                    json!({"id":3,"result":{"data":[{"id":"a","items":[],"itemsView":"summary"}]}}),
                );
            }
            if bad == "loop" {
                reader.accept(json!({"id":3,"result":{"data":[],"nextCursor":"same"}}));
                step = reader.accept(json!({"id":4,"result":{"data":[],"nextCursor":"same"}}));
            }
            assert!(step.done && step.error.is_some(), "{bad}");
            assert!(step.writes.is_empty());
            assert!(rx.try_recv().is_err());
        }
    }
}
