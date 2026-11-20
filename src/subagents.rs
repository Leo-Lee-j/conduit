//! Provider-independent, persisted snapshots. Tool completion is never agent completion.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Starting,
    Running,
    WaitingApproval,
    WaitingInput,
    Idle,
    Completed,
    Failed,
    Stopped,
    #[default]
    Unknown,
}
impl Status {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::WaitingApproval | Self::WaitingInput
        )
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Stopped)
    }
    pub fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Starting => "subagents.starting",
            Self::Running => "subagents.running",
            Self::WaitingApproval => "subagents.waiting_approval",
            Self::WaitingInput => "subagents.waiting_input",
            Self::Idle => "subagents.idle",
            Self::Completed => "subagents.completed",
            Self::Failed => "subagents.failed",
            Self::Stopped => "subagents.stopped",
            Self::Unknown => "subagents.unknown",
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Agent {
    pub id: String,
    pub parent_id: String,
    pub name: String,
    /// Provider-supplied short description, kept separate from the original task.
    pub title: String,
    /// Explicit provider role/type; a nickname is not a role.
    pub role: String,
    pub task: String,
    pub activity: String,
    pub result: String,
    pub status: Status,
    pub started_at: u64,
    pub updated_at: u64,
    pub finished_at: Option<u64>,
    pub elapsed_ms: Option<u64>,
    /// Owning Conduit run, assigned on ingestion; prevents stale history looking live.
    pub run: String,
    /// Provider turn/task identity; permits completed agents to accept a new task.
    pub task_id: String,
}
impl Agent {
    pub fn new(id: &str, parent_id: &str) -> Self {
        let now = now();
        Self {
            id: id.into(),
            parent_id: parent_id.into(),
            started_at: now,
            updated_at: now,
            ..Default::default()
        }
    }
    pub fn set_status(&mut self, status: Status) {
        if self.status == status {
            return;
        }
        self.status = status;
        self.finished_at = status.terminal().then(now);
    }
    pub fn restart(&mut self, task: &str) {
        if self.task_id == task {
            return;
        }
        self.task_id = task.into();
        self.started_at = now();
        self.finished_at = None;
        self.elapsed_ms = None;
        self.title.clear();
        self.activity.clear();
        self.result.clear();
        self.status = Status::Running;
    }
    pub fn bytes(&self) -> usize {
        240 + [
            &self.id,
            &self.parent_id,
            &self.name,
            &self.title,
            &self.role,
            &self.task,
            &self.activity,
            &self.result,
            &self.run,
            &self.task_id,
        ]
        .iter()
        .map(|s| s.len())
        .sum::<usize>()
    }
    pub fn bounded(&mut self) {
        for text in [
            &mut self.name,
            &mut self.title,
            &mut self.role,
            &mut self.task,
            &mut self.activity,
            &mut self.result,
        ] {
            if text.len() > 8192 {
                let mut end = 8192;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn upsert(agents: &mut Vec<Agent>, mut agent: Agent, run: &str) {
    agent.run = run.into();
    agent.bounded();
    if let Some(old) = agents.iter_mut().find(|a| a.id == agent.id && a.run == run) {
        *old = agent;
    } else {
        // Bound metadata retained in the session index. Prefer evicting old finished records.
        if agents.len() >= 512 {
            if let Some(index) = agents.iter().position(|a| !a.status.active()) {
                agents.remove(index);
            } else {
                return;
            }
        }
        agents.push(agent);
    }
}
pub fn disconnect(agents: &mut [Agent], run: Option<&str>, cancelled: bool) {
    for agent in agents {
        if run.is_none_or(|run| agent.run == run) && agent.status.active() {
            agent
                .elapsed_ms
                .get_or_insert_with(|| now().saturating_sub(agent.started_at) * 1000);
            agent.set_status(if cancelled {
                Status::Stopped
            } else {
                Status::Unknown
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_roundtrip_and_disconnect_never_invent_success() {
        let mut agent = Agent::new("child", "parent");
        agent.title = "Review storage".into();
        agent.role = "reviewer".into();
        agent.task = "Inspect storage\nKeep the original task".into();
        agent.status = Status::Running;
        let mut records = vec![];
        upsert(&mut records, agent.clone(), "run-a");
        upsert(&mut records, agent.clone(), "run-a");
        assert_eq!(records.len(), 1);
        agent.set_status(Status::Completed);
        upsert(&mut records, agent, "run-b");
        let mut restored: Vec<Agent> =
            serde_json::from_str(&serde_json::to_string(&records).unwrap()).unwrap();
        disconnect(&mut restored, None, false);
        assert_eq!(restored[0].title, "Review storage");
        assert_eq!(restored[0].role, "reviewer");
        assert_eq!(restored[1].status, Status::Completed);
        let legacy: Agent = serde_json::from_str(r#"{"id":"legacy","task":"old task"}"#).unwrap();
        assert!(legacy.title.is_empty() && legacy.role.is_empty());
        restored[1].restart("next-task");
        assert!(
            restored[1].title.is_empty(),
            "a new task cannot inherit the old short title"
        );
        assert_eq!(restored[1].role, "reviewer");
        assert_eq!(restored[0].status, Status::Unknown);
        assert_eq!(restored[1].status, Status::Running);
        assert_eq!(records[0].status, Status::Running);
        disconnect(&mut records, Some("run-a"), true);
        assert_eq!(records[0].status, Status::Stopped);
    }
    #[test]
    fn restored_session_metadata_never_claims_a_live_connection() {
        let root = std::env::temp_dir().join(format!(
            "conduit-subagent-store-{}-{}",
            std::process::id(),
            now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        let mut session = crate::app::tests::session();
        let mut agent = Agent::new("child", "parent");
        agent.status = Status::Running;
        upsert(&mut session.subagents, agent, "run");
        let store = crate::store::Store {
            workspaces: vec![crate::domain::Workspace {
                path: root.clone(),
                sessions: vec![session],
            }],
            ..Default::default()
        };
        store.save_to(&path).unwrap();
        let restored = crate::store::Store::load_from(&path).unwrap();
        assert_eq!(
            restored.workspaces[0].sessions[0].subagents[0].status,
            Status::Unknown
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
