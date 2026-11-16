//! Bounded background observer for the same authenticated OpenCode HTTP server.
//! ACP does not include child session lifecycle notifications.
use crate::subagents::{Agent, Status};
use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(super) struct Monitor {
    stop: Arc<AtomicBool>,
    pub receiver: mpsc::Receiver<(u64, Vec<Agent>)>,
}
impl Monitor {
    pub fn start(port: u16, password: &str, cwd: &std::path::Path, session: &str) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = stop.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let source = Source::new(port, password, cwd);
        let root = session.to_owned();
        let started_ms = crate::subagents::now().saturating_sub(2) * 1000;
        std::thread::spawn(move || {
            let mut records: BTreeMap<String, Agent> = BTreeMap::new();
            let mut queue = VecDeque::from([root.clone()]);
            let mut failures = 0;
            while !cancelled.load(Ordering::Acquire) {
                let cycle = Instant::now();
                let observed_at = crate::subagents::now_ms();
                let result = (|| -> Result<()> {
                    let statuses = source.get("/session/status", None)?;
                    let permissions = source.get("/permission", None).unwrap_or_default();
                    // Revisit each discovered session fairly; inspect a bounded number per cycle.
                    for _ in 0..queue.len().clamp(1, 4) {
                        if cancelled.load(Ordering::Acquire) {
                            return Ok(());
                        }
                        let parent = queue.pop_front().unwrap_or_else(|| root.clone());
                        queue.push_back(parent.clone());
                        let children =
                            source.get(&format!("/session/{}/children", encode(&parent)), None)?;
                        for child in children.as_array().context("invalid child session list")? {
                            let Some(id) = child["id"].as_str().filter(|id| !id.is_empty()) else {
                                continue;
                            };
                            if child["parentID"] != parent || id == root {
                                continue;
                            }
                            if records.len() >= 128 && !records.contains_key(id) {
                                continue;
                            }
                            let busy =
                                matches!(statuses[id]["type"].as_str(), Some("busy" | "retry"));
                            if !records.contains_key(id)
                                && !busy
                                && child["time"]["updated"]
                                    .as_u64()
                                    .is_some_and(|at| at < started_ms)
                            {
                                continue;
                            }
                            if !records.contains_key(id) {
                                queue.push_back(id.to_owned());
                            }
                            let record = records.entry(id.into()).or_insert_with(|| {
                                let mut agent = Agent::new(id, &parent);
                                agent.status = Status::Starting;
                                agent.started_at = child["time"]["created"]
                                    .as_u64()
                                    .map(|at| at / 1000)
                                    .unwrap_or(agent.started_at);
                                agent
                            });
                            if let Some(name) = child["agent"].as_str() {
                                record.name = name.into();
                            }
                            if let Some(task) = child["title"].as_str() {
                                record.task = task.into();
                            }
                        }
                        if let Some(record) = records.get_mut(&parent) {
                            let status = &statuses[&parent];
                            if matches!(status["type"].as_str(), Some("busy" | "retry")) {
                                if record.status.terminal() {
                                    record.restart(&format!("{}", crate::subagents::now()));
                                }
                                record.status = Status::Running;
                                if let Some(message) = status["message"].as_str() {
                                    record.activity = message.into();
                                }
                            }
                            let messages = source
                                .get(&format!("/session/{}/message", encode(&parent)), Some("8"))?;
                            apply_messages(record, status, &messages);
                            if permissions.as_array().is_some_and(|items| {
                                items.iter().any(|item| item["sessionID"] == parent)
                            }) {
                                record.status = Status::WaitingApproval;
                            }
                            record.bounded();
                        }
                        if cycle.elapsed() > Duration::from_secs(2) {
                            break;
                        }
                    }
                    Ok(())
                })();
                if result.is_err() {
                    failures += 1;
                    if failures >= 3 {
                        for record in records.values_mut() {
                            if record.status.active() {
                                record.set_status(Status::Unknown);
                            }
                        }
                    }
                } else {
                    failures = 0;
                }
                if sender
                    .try_send((observed_at, records.values().cloned().collect()))
                    .is_err()
                    && cancelled.load(Ordering::Acquire)
                {
                    break;
                }
                while cycle.elapsed() < Duration::from_secs(1) && !cancelled.load(Ordering::Acquire)
                {
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        });
        Self { stop, receiver }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
struct Source {
    agent: ureq::Agent,
    base: String,
    auth: String,
    directory: String,
}
impl Source {
    fn new(port: u16, password: &str, cwd: &std::path::Path) -> Self {
        Self {
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .proxy(None)
                    .max_redirects(0)
                    .timeout_global(Some(Duration::from_millis(750)))
                    .build(),
            ),
            base: format!("http://127.0.0.1:{port}"),
            auth: format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("conduit:{password}"))
            ),
            directory: cwd.to_string_lossy().into_owned(),
        }
    }
    fn get(&self, path: &str, limit: Option<&str>) -> Result<Value> {
        let request = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("Authorization", &self.auth)
            .query("directory", &self.directory);
        let request = if let Some(limit) = limit {
            request.query("limit", limit)
        } else {
            request
        };
        Ok(request
            .call()?
            .body_mut()
            .with_config()
            .limit(2 * 1024 * 1024)
            .read_json()?)
    }
}
fn encode(id: &str) -> String {
    id.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn apply_messages(agent: &mut Agent, status: &Value, messages: &Value) {
    let busy = matches!(status["type"].as_str(), Some("busy" | "retry"));
    let latest = messages.as_array().and_then(|entries| {
        entries
            .iter()
            .max_by_key(|entry| entry["info"]["time"]["created"].as_u64().unwrap_or(0))
    });
    let Some(latest) = latest else {
        if !busy && !agent.status.terminal() {
            agent.status = Status::Idle;
        }
        return;
    };
    let info = &latest["info"];
    if !busy {
        if info.get("error").is_some_and(|v| !v.is_null()) {
            agent.set_status(if info["error"]["name"] == "MessageAbortedError" {
                Status::Stopped
            } else {
                Status::Failed
            });
            agent.result = info["error"].to_string();
        } else if info["role"] == "assistant"
            && info["time"]["completed"].as_u64().is_some()
            && info["finish"]
                .as_str()
                .is_some_and(|finish| !matches!(finish, "tool-calls" | "unknown"))
        {
            agent.set_status(Status::Completed);
        } else {
            agent.status = Status::Idle;
        }
    }
    if agent.status.terminal()
        && let Some(end) = info["time"]["completed"].as_u64()
    {
        agent.elapsed_ms = Some(end.saturating_sub(agent.started_at * 1000));
    }
    for part in latest["parts"].as_array().into_iter().flatten() {
        if part["type"] == "tool"
            && let Some(tool) = part["tool"].as_str()
        {
            agent.activity = tool.into();
        }
        if part["type"] == "text"
            && info["role"] == "assistant"
            && let Some(text) = part["text"].as_str()
        {
            agent.result = text.into();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn busy_idle_completed_failed_are_distinct() {
        let mut agent = Agent::new("ses_child", "ses_parent");
        apply_messages(&mut agent, &json!({"type":"idle"}), &json!([]));
        assert_eq!(agent.status, Status::Idle);
        let completed = json!([{"info":{"role":"assistant","time":{"created":10,"completed":30},"finish":"stop"},"parts":[{"type":"text","text":"done"}]}]);
        agent.status = Status::Running;
        apply_messages(&mut agent, &json!({"type":"busy"}), &completed);
        assert_eq!(agent.status, Status::Running);
        apply_messages(&mut agent, &json!({}), &completed);
        assert_eq!(agent.status, Status::Completed);
        assert_eq!(agent.result, "done");
        apply_messages(
            &mut agent,
            &json!({}),
            &json!([{"info":{"role":"assistant","error":{"name":"APIError"}},"parts":[]}]),
        );
        assert_eq!(agent.status, Status::Failed);
    }
}
