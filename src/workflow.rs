//! Project tasks, explicit session presets and bounded conversation handoffs.
use crate::{
    domain::{ItemKind, PermissionLevel, Provider, SessionRecord},
    providers::{ModelChannel, ReasoningEffort},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_CONTEXT_BYTES: usize = 128 * 1024;
const MAX_ITEMS: usize = 500;

fn error(key: &'static str) -> String {
    conduit::i18n::text(key)
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    #[default]
    Planned,
    Active,
    Review,
    Done,
}
impl TaskStatus {
    pub const ALL: [Self; 4] = [Self::Planned, Self::Active, Self::Review, Self::Done];
    pub fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Planned => "workflow.planned",
            Self::Active => "workflow.active",
            Self::Review => "workflow.review",
            Self::Done => "workflow.done",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Criterion {
    pub text: String,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: u64,
    pub workspace: PathBuf,
    pub title: String,
    pub goal: String,
    pub branch: Option<String>,
    pub sessions: Vec<u64>,
    pub criteria: Vec<Criterion>,
    pub status: TaskStatus,
    pub archived: bool,
    pub created_at: u64,
    #[serde(default)]
    pub origin: Option<PathBuf>,
}

impl WorkItem {
    pub fn set_status(&mut self, status: TaskStatus) -> Result<(), String> {
        if status == TaskStatus::Done && self.criteria.iter().any(|c| !c.done) {
            return Err(error("workflow.criteria_incomplete"));
        }
        self.status = status;
        Ok(())
    }

    pub fn prompt(&self) -> String {
        let mut prompt = conduit::i18n::format(
            "workflow.task_prompt",
            &[
                ("title", self.title.clone()),
                ("goal", self.goal.clone()),
                ("workspace", self.workspace.display().to_string()),
            ],
        );
        for criterion in &self.criteria {
            prompt.push_str(&format!(
                "\n- [{}] {}",
                if criterion.done { "x" } else { " " },
                criterion.text
            ));
        }
        prompt
    }
}

/// No credentials or native conversation IDs are copied into a reusable preset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub id: u64,
    pub workspace: PathBuf,
    pub name: String,
    pub prompt: String,
    pub provider: Provider,
    pub channel: ModelChannel,
    pub model: Option<String>,
    pub effort: ReasoningEffort,
    #[serde(default)]
    pub acp_variant: Option<String>,
    pub permission: PermissionLevel,
    pub archived: bool,
}
impl Preset {
    pub fn apply(&self, session: &mut SessionRecord) {
        session.provider = self.provider;
        session.channel = self.channel.clone();
        session.model = self.model.clone();
        session.reasoning_effort = self.effort;
        session.permission = self.permission;
        // Dynamic ACP options and composer variants belong to their original session.
        session.opencode = Default::default();
        session.opencode.variant = self.acp_variant.clone();
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Workflow {
    pub tasks: Vec<WorkItem>,
    pub presets: Vec<Preset>,
    pub artifacts: Vec<crate::artifacts::Artifact>,
    pub environments: Vec<crate::environments::Environment>,
    next_id: u64,
}

impl Workflow {
    fn allocate(&mut self) -> Result<u64, String> {
        let max = self
            .tasks
            .iter()
            .map(|t| t.id)
            .chain(self.presets.iter().map(|p| p.id))
            .chain(self.artifacts.iter().map(|a| a.id))
            .chain(self.environments.iter().map(|e| e.id))
            .max()
            .unwrap_or(0)
            .max(self.next_id);
        self.next_id = max.checked_add(1).ok_or_else(|| error("workflow.limit"))?;
        Ok(self.next_id)
    }
    pub fn save_task(&mut self, mut task: WorkItem) -> Result<u64, String> {
        task.title = task.title.trim().to_owned();
        task.goal = task.goal.trim().to_owned();
        if task.title.is_empty()
            || task.title.len() > 256
            || task.goal.len() > 32 * 1024
            || task.criteria.len() > 50
            || task.criteria.iter().any(|c| c.text.len() > 1024)
        {
            return Err(error("workflow.task_limit"));
        }
        if task.id == 0 {
            if self.tasks.len() >= MAX_ITEMS {
                return Err(error("workflow.limit"));
            }
            task.id = self.allocate()?;
            let id = task.id;
            self.tasks.push(task);
            Ok(id)
        } else {
            let previous = self
                .tasks
                .iter_mut()
                .find(|t| t.id == task.id && t.workspace == task.workspace)
                .ok_or_else(|| error("workflow.missing"))?;
            // Editing goals or criteria requires fresh human acceptance.
            if task.goal != previous.goal
                || task
                    .criteria
                    .iter()
                    .map(|c| &c.text)
                    .ne(previous.criteria.iter().map(|c| &c.text))
            {
                for criterion in &mut task.criteria {
                    criterion.done = false;
                }
                if task.status == TaskStatus::Done {
                    task.status = TaskStatus::Review;
                }
            }
            let id = task.id;
            *previous = task;
            Ok(id)
        }
    }
    pub fn save_preset(&mut self, mut preset: Preset) -> Result<u64, String> {
        preset.name = preset.name.trim().to_owned();
        if preset.name.is_empty()
            || preset.name.len() > 256
            || preset.prompt.trim().is_empty()
            || preset.prompt.len() > 32 * 1024
        {
            return Err(error("workflow.preset_limit"));
        }
        if self.presets.len() >= MAX_ITEMS {
            return Err(error("workflow.limit"));
        }
        preset.id = self.allocate()?;
        let id = preset.id;
        self.presets.push(preset);
        Ok(id)
    }

    pub fn save_artifact(
        &mut self,
        mut artifact: crate::artifacts::Artifact,
    ) -> Result<u64, String> {
        if self.artifacts.len() >= MAX_ITEMS
            || artifact.title.trim().is_empty()
            || artifact.title.len() > 256
            || artifact.target.len() > 8192
        {
            return Err(error("artifact.invalid"));
        }
        artifact.id = self.allocate()?;
        let id = artifact.id;
        self.artifacts.push(artifact);
        Ok(id)
    }

    pub fn save_environment(
        &mut self,
        mut environment: crate::environments::Environment,
    ) -> Result<u64, String> {
        environment.validate().map_err(|e| e.to_string())?;
        if self.environments.len() >= 50 {
            return Err(error("workflow.limit"));
        }
        environment.id = self.allocate()?;
        let id = environment.id;
        self.environments.push(environment);
        Ok(id)
    }
}

/// Explicit visible-message transfer, not a copy of hidden native engine state.
pub fn handoff(
    workspace: &Path,
    session: &SessionRecord,
    through: usize,
) -> Result<String, String> {
    if !session.transcript.loaded || through >= session.messages.len() {
        return Err(error("workflow.transcript_unavailable"));
    }
    let intro = conduit::i18n::format(
        "workflow.handoff_intro",
        &[
            ("workspace", workspace.display().to_string()),
            ("title", session.title.clone()),
            ("id", session.id.to_string()),
            ("engine", session.provider.label().into()),
            ("through", (through + 1).to_string()),
        ],
    );
    let mut blocks = Vec::new();
    let mut size = intro.len() + 512;
    let mut omitted = 0;
    for (index, item) in session.messages[..=through].iter().enumerate().rev() {
        if !matches!(item.kind, ItemKind::User | ItemKind::Assistant) {
            continue;
        }
        let role = conduit::i18n::text_static(if item.kind == ItemKind::User {
            "workflow.user"
        } else {
            "workflow.assistant"
        });
        let header = format!("\n\n### {} · {role}\n", index + 1);
        if size + header.len() + item.text.len() > MAX_CONTEXT_BYTES {
            omitted += 1;
            continue;
        }
        size += header.len() + item.text.len();
        blocks.push(format!("{header}{}", item.text));
    }
    let mut output = intro;
    if omitted > 0 {
        output.push_str(&conduit::i18n::format(
            "workflow.context_omitted",
            &[("count", omitted.to_string())],
        ));
    }
    for block in blocks.into_iter().rev() {
        output.push_str(&block);
    }
    if output.len() > MAX_CONTEXT_BYTES {
        return Err(error("workflow.context_limit"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoff_keeps_the_selected_boundary_and_reports_missing_content() {
        let mut session = crate::app::tests::session();
        session.messages = [
            ("User", "first".to_owned()),
            ("Tool", "private tool payload".to_owned()),
            ("Assistant", "answer".to_owned()),
            ("User", "later".to_owned()),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (kind, text))| {
            serde_json::from_value(
                serde_json::json!({"key": format!("message-{i}"), "kind": kind, "text": text}),
            )
            .unwrap()
        })
        .collect();
        let context = handoff(Path::new("C:/project"), &session, 2).unwrap();
        assert!(context.contains("first") && context.contains("answer"));
        assert!(!context.contains("private tool payload") && !context.contains("later"));
        session.messages[2].text = "长".repeat(MAX_CONTEXT_BYTES);
        let context = handoff(Path::new("C:/project"), &session, 2).unwrap();
        assert!(context.len() <= MAX_CONTEXT_BYTES);
        assert!(context.contains(&conduit::i18n::format(
            "workflow.context_omitted",
            &[("count", "1".into())]
        )));
        session.transcript.loaded = false;
        assert!(handoff(Path::new("C:/project"), &session, 2).is_err());
    }

    #[test]
    fn edited_acceptance_is_invalidated_and_workspace_identity_is_preserved() {
        let mut workflow = Workflow::default();
        let item = WorkItem {
            id: 0,
            workspace: PathBuf::from("C:/one"),
            title: "修复".into(),
            goal: "goal".into(),
            branch: None,
            sessions: vec![7],
            criteria: vec![Criterion {
                text: "verified".into(),
                done: false,
            }],
            status: TaskStatus::Review,
            archived: false,
            created_at: 1,
            origin: None,
        };
        let id = workflow.save_task(item).unwrap();
        assert!(workflow.tasks[0].set_status(TaskStatus::Done).is_err());
        workflow.tasks[0].criteria[0].done = true;
        workflow.tasks[0].set_status(TaskStatus::Done).unwrap();
        let mut edited = workflow.tasks[0].clone();
        edited.goal = "new goal".into();
        workflow.save_task(edited).unwrap();
        assert_eq!(workflow.tasks[0].id, id);
        assert_eq!(workflow.tasks[0].status, TaskStatus::Review);
        assert!(!workflow.tasks[0].criteria[0].done);
        let mut alien = workflow.tasks[0].clone();
        alien.workspace = PathBuf::from("C:/two");
        assert!(workflow.save_task(alien).is_err());
        let restored: Workflow =
            serde_json::from_str(&serde_json::to_string(&workflow).unwrap()).unwrap();
        assert_eq!(restored.tasks[0].sessions, vec![7]);
    }
}
