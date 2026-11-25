//! Provider task state. An empty snapshot is authoritative, not a missing value.
use crate::domain::{ItemKind, SessionRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) mod claude;
const MAX_TASKS: usize = 256;
const MAX_TEXT: usize = 8192;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    #[default]
    Pending,
    InProgress,
    Completed,
}
impl Status {
    pub(crate) fn from_wire(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "in_progress" | "inProgress" => Self::InProgress,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Task {
    pub text: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub active_form: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
}
impl Task {
    pub(crate) fn label(&self) -> String {
        if self.id.is_empty() {
            self.text.clone()
        } else {
            format!("#{} {}", self.id, self.text)
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        128 + self.text.len()
            + self.id.len()
            + self.description.len()
            + self.active_form.len()
            + self.owner.len()
            + self.blocked_by.iter().map(String::len).sum::<usize>()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub steps: Vec<Task>,
    /// Claude Tasks belong to a session; TodoWrite and ACP/Codex plans belong to a turn.
    pub persistent: bool,
    pub turn: Option<String>,
}
impl Snapshot {
    pub(crate) fn visible<'a>(&'a self, session: &SessionRecord) -> Option<&'a [Task]> {
        (self.persistent || !session.transcript.loaded || self.turn.as_deref() == turn_key(session))
            .then_some(self.steps.as_slice())
    }
    pub(crate) fn apply(&mut self, change: Change, turn: Option<String>) {
        match change {
            Change::Replace {
                mut steps,
                persistent,
            } => {
                if persistent && self.persistent {
                    // TaskList intentionally omits descriptions and activeForm.
                    for task in &mut steps {
                        if let Some(old) = self.steps.iter().find(|old| old.id == task.id) {
                            if task.description.is_empty() {
                                task.description.clone_from(&old.description);
                            }
                            if task.active_form.is_empty() {
                                task.active_form.clone_from(&old.active_form);
                            }
                        }
                    }
                }
                self.steps = steps;
                self.persistent = persistent;
            }
            Change::Upsert(mut task) => {
                self.ensure_tasks();
                if let Some(old) = self.steps.iter_mut().find(|old| old.id == task.id) {
                    if task.active_form.is_empty() {
                        task.active_form.clone_from(&old.active_form);
                    }
                    *old = task;
                } else if self.steps.len() < MAX_TASKS {
                    self.steps.push(task);
                }
            }
            Change::Update(patch) => {
                self.ensure_tasks();
                if patch.deleted {
                    self.steps.retain(|task| task.id != patch.id);
                    for task in &mut self.steps {
                        task.blocked_by.retain(|id| id != &patch.id);
                    }
                } else {
                    if let Some(task) = self.steps.iter_mut().find(|task| task.id == patch.id) {
                        if let Some(text) = patch.text {
                            task.text = text;
                        }
                        if let Some(value) = patch.status {
                            task.status = value;
                        }
                        if let Some(value) = patch.description {
                            task.description = value;
                        }
                        if let Some(value) = patch.active_form {
                            task.active_form = value;
                        }
                        if let Some(value) = patch.owner {
                            task.owner = value;
                        }
                        for id in patch.blocked_by {
                            if id != task.id
                                && !task.blocked_by.contains(&id)
                                && task.blocked_by.len() < MAX_TASKS
                            {
                                task.blocked_by.push(id);
                            }
                        }
                    }
                    for task in &mut self.steps {
                        if patch.blocks.contains(&task.id)
                            && task.id != patch.id
                            && !task.blocked_by.contains(&patch.id)
                            && task.blocked_by.len() < MAX_TASKS
                        {
                            task.blocked_by.push(patch.id.clone());
                        }
                    }
                }
            }
        }
        self.turn = turn;
    }
    fn ensure_tasks(&mut self) {
        if !self.persistent {
            self.steps.clear();
        }
        self.persistent = true;
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Change {
    Replace { steps: Vec<Task>, persistent: bool },
    Upsert(Task),
    Update(Patch),
}
impl Change {
    pub(crate) fn bytes(&self) -> usize {
        match self {
            Self::Replace { steps, .. } => steps.iter().map(Task::bytes).sum(),
            Self::Upsert(task) => task.bytes(),
            Self::Update(p) => p.bytes(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Patch {
    id: String,
    text: Option<String>,
    status: Option<Status>,
    description: Option<String>,
    active_form: Option<String>,
    owner: Option<String>,
    blocked_by: Vec<String>,
    blocks: Vec<String>,
    deleted: bool,
}

impl Patch {
    fn bytes(&self) -> usize {
        128 + self.id.len()
            + [
                &self.text,
                &self.description,
                &self.active_form,
                &self.owner,
            ]
            .into_iter()
            .flatten()
            .map(String::len)
            .sum::<usize>()
            + self
                .blocked_by
                .iter()
                .chain(&self.blocks)
                .map(String::len)
                .sum::<usize>()
    }
}

pub(crate) fn turn_key(session: &SessionRecord) -> Option<&str> {
    session
        .messages
        .iter()
        .rev()
        .find(|item| item.kind == ItemKind::User)
        .map(|item| item.key.as_str())
}
pub(crate) fn apply(session: &mut SessionRecord, change: Change) {
    let turn = turn_key(session).map(str::to_owned);
    session
        .task_list
        .get_or_insert_with(Default::default)
        .apply(change, turn);
}

fn bounded(value: &str) -> String {
    let mut end = value.len().min(MAX_TEXT);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}
fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(bounded)
}
fn id(value: &Value) -> Option<String> {
    let value = value.as_str()?;
    (!value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}
fn ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(MAX_TASKS)
        .filter_map(id)
        .collect()
}

pub(crate) fn plan(input: &Value) -> Option<Vec<Task>> {
    let entries = input
        .get("plan")
        .or_else(|| input.get("todos"))?
        .as_array()?;
    if entries.len() > MAX_TASKS {
        return None;
    }
    entries
        .iter()
        .map(|entry| {
            let text = entry
                .get("step")
                .or_else(|| entry.get("content"))
                .or_else(|| entry.get("activeForm"))?
                .as_str()?
                .trim();
            if text.is_empty() {
                return None;
            }
            Some(Task {
                text: bounded(text),
                status: Status::from_wire(entry["status"].as_str().unwrap_or("pending")),
                active_form: string(entry, "activeForm").unwrap_or_default(),
                ..Default::default()
            })
        })
        .collect()
}

pub(crate) fn wire_plan(steps: Vec<(String, String)>) -> Change {
    Change::Replace {
        steps: steps
            .into_iter()
            .take(MAX_TASKS)
            .filter_map(|(text, status)| {
                let text = text.trim();
                (!text.is_empty()).then(|| Task {
                    text: bounded(text),
                    status: Status::from_wire(&status),
                    ..Default::default()
                })
            })
            .collect(),
        persistent: false,
    }
}

#[cfg(test)]
mod tests;
