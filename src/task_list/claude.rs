//! Correlates acknowledged task calls. Never infer an ID from call order.
use super::*;
use std::collections::VecDeque;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Tracker {
    pending: VecDeque<(String, Call)>,
    completed: VecDeque<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Call {
    Create(Task),
    Update(Patch),
    List,
    Get(String),
    Todo(Vec<Task>),
}
impl Call {
    fn bytes(&self) -> usize {
        match self {
            Self::Create(task) => task.bytes(),
            Self::Update(patch) => patch.bytes(),
            Self::Todo(tasks) => tasks.iter().map(Task::bytes).sum(),
            Self::Get(id) => id.len(),
            Self::List => 0,
        }
    }
}

pub(crate) fn is_task_tool(name: &str) -> bool {
    matches!(
        name,
        "TaskCreate"
            | "TaskUpdate"
            | "TaskList"
            | "TaskGet"
            | "TodoWrite"
            | "todo_write"
            | "update_plan"
    )
}

fn task(value: &Value) -> Option<Task> {
    let text = string(value, "subject")?;
    if text.trim().is_empty() {
        return None;
    }
    Some(Task {
        id: id(&value["id"])?,
        text,
        status: Status::from_wire(value["status"].as_str().unwrap_or("pending")),
        description: string(value, "description").unwrap_or_default(),
        active_form: string(value, "activeForm").unwrap_or_default(),
        owner: string(value, "owner").unwrap_or_default(),
        blocked_by: ids(&value["blockedBy"]),
    })
}
fn patch(value: &Value) -> Option<Patch> {
    let status = value["status"].as_str();
    Some(Patch {
        id: id(&value["taskId"])?,
        text: string(value, "subject").filter(|s| !s.trim().is_empty()),
        status: status
            .filter(|s| matches!(*s, "pending" | "in_progress" | "completed"))
            .map(Status::from_wire),
        description: string(value, "description"),
        active_form: string(value, "activeForm"),
        owner: string(value, "owner"),
        blocked_by: ids(&value["addBlockedBy"]),
        blocks: ids(&value["addBlocks"]),
        deleted: status == Some("deleted"),
    })
}

impl Tracker {
    pub(crate) fn accept(&mut self, value: &Value) -> Vec<Change> {
        if !value["parent_tool_use_id"].is_null() || value["isSidechain"] == true {
            return vec![];
        }
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return vec![];
        };
        let mut changes = Vec::new();
        for block in blocks {
            match (value["type"].as_str(), block["type"].as_str()) {
                (Some("assistant"), Some("tool_use")) => self.input(block),
                (Some("user"), Some("tool_result")) => {
                    let output = if blocks.len() == 1 {
                        value
                            .get("tool_use_result")
                            .or_else(|| value.get("toolUseResult"))
                            .unwrap_or(&Value::Null)
                    } else {
                        &Value::Null
                    };
                    if let Some(change) = self.result(block, output) {
                        changes.push(change);
                    }
                }
                _ => {}
            }
        }
        changes
    }
    fn input(&mut self, block: &Value) {
        let Some(key) = id(&block["id"]) else {
            return;
        };
        if self.completed.contains(&key) || self.pending.iter().any(|(id, _)| id == &key) {
            return;
        }
        let input = &block["input"];
        let call = match block["name"].as_str() {
            Some("TaskCreate") => {
                let Some(text) = string(input, "subject").filter(|s| !s.trim().is_empty()) else {
                    return;
                };
                Call::Create(Task {
                    text,
                    description: string(input, "description").unwrap_or_default(),
                    active_form: string(input, "activeForm").unwrap_or_default(),
                    ..Default::default()
                })
            }
            Some("TaskUpdate") => {
                let Some(patch) = patch(input) else {
                    return;
                };
                Call::Update(patch)
            }
            Some("TaskList") => Call::List,
            Some("TaskGet") => {
                let Some(id) = id(&input["taskId"]) else {
                    return;
                };
                Call::Get(id)
            }
            Some("TodoWrite" | "todo_write" | "update_plan") => {
                let Some(steps) = plan(input) else {
                    return;
                };
                Call::Todo(steps)
            }
            _ => return,
        };
        const MAX_PENDING_BYTES: usize = 1024 * 1024;
        let bytes = call.bytes();
        if bytes > MAX_PENDING_BYTES {
            return;
        }
        let mut pending_bytes: usize = self
            .pending
            .iter()
            .map(|(key, call)| key.len() + call.bytes())
            .sum();
        while self.pending.len() >= MAX_TASKS
            || pending_bytes + key.len() + bytes > MAX_PENDING_BYTES
        {
            let Some((key, call)) = self.pending.pop_front() else {
                break;
            };
            pending_bytes = pending_bytes.saturating_sub(key.len() + call.bytes());
        }
        self.pending.push_back((key, call));
    }
    fn result(&mut self, block: &Value, output: &Value) -> Option<Change> {
        let key = id(&block["tool_use_id"])?;
        let position = self.pending.iter().position(|(id, _)| id == &key)?;
        let (_, call) = self.pending.remove(position)?;
        if self.completed.len() == MAX_TASKS * 2 {
            self.completed.pop_front();
        }
        self.completed.push_back(key);
        if block["is_error"] == true
            || output["success"] == false
            || output.get("error").is_some_and(|e| !e.is_null())
        {
            return None;
        }
        let content = content_text(&block["content"]);
        if content.contains("<tool_use_error>") {
            return None;
        }
        // Some SDK transports omit structured tool_use_result. JSON and the
        // native readable result are accepted only for a correlated task tool.
        let decoded = serde_json::from_str::<Value>(&content).ok();
        let output = if output.is_object() {
            output
        } else {
            decoded.as_ref().unwrap_or(&Value::Null)
        };
        if output["success"] == false || output.get("error").is_some_and(|e| !e.is_null()) {
            return None;
        }
        match call {
            Call::Todo(steps) => Some(Change::Replace {
                steps,
                persistent: false,
            }),
            Call::Create(mut created) => {
                if let Some(actual) = task(&output["task"]) {
                    created.id = actual.id;
                    created.text = actual.text;
                    created.description = actual.description;
                } else if output.is_null() {
                    let rest = content.strip_prefix("Task #")?;
                    let (key, _) = rest.split_once(" created successfully: ")?;
                    created.id = id(&Value::String(key.into()))?;
                } else {
                    return None;
                }
                Some(Change::Upsert(created))
            }
            Call::Update(mut patch) => {
                if output.is_object() {
                    if output["success"] != true || output["taskId"].as_str() != Some(&patch.id) {
                        return None;
                    }
                    // Only fields confirmed by the CLI may alter the current list.
                    let fields = output["updatedFields"].as_array()?;
                    let has = |name: &str| fields.iter().any(|f| f.as_str() == Some(name));
                    if !has("subject") {
                        patch.text = None;
                    }
                    if !has("description") {
                        patch.description = None;
                    }
                    if !has("activeForm") {
                        patch.active_form = None;
                    }
                    if !has("owner") {
                        patch.owner = None;
                    }
                    if !has("blockedBy") {
                        patch.blocked_by.clear();
                    }
                    if !has("blocks") {
                        patch.blocks.clear();
                    }
                    if !has("status") {
                        patch.status = None;
                    }
                    patch.deleted &= has("deleted");
                    if let Some(status) = output["statusChange"]["to"].as_str() {
                        if has("status") {
                            patch.status = Some(Status::from_wire(status));
                        }
                        if has("deleted") {
                            patch.deleted = status == "deleted";
                        }
                    }
                } else if !content.starts_with(&format!("Updated task #{} ", patch.id)) {
                    return None;
                }
                Some(Change::Update(patch))
            }
            Call::List => {
                let steps = if let Some(tasks) = output["tasks"].as_array() {
                    if tasks.len() > MAX_TASKS {
                        return None;
                    }
                    tasks.iter().map(task).collect::<Option<Vec<_>>>()?
                } else if output.is_null() {
                    text_list(&content)?
                } else {
                    return None;
                };
                // Duplicate IDs make the snapshot ambiguous; retain the prior list.
                let mut unique = std::collections::HashSet::new();
                if steps.iter().any(|t| !unique.insert(&t.id)) {
                    return None;
                }
                Some(Change::Replace {
                    steps,
                    persistent: true,
                })
            }
            Call::Get(expected) => {
                let task = if output.is_null() {
                    text_task(&content)?
                } else {
                    task(&output["task"])?
                };
                (task.id == expected).then_some(Change::Upsert(task))
            }
        }
    }
}

fn text_task(content: &str) -> Option<Task> {
    let (heading, rest) = content.split_once('\n')?;
    let (key, label) = heading.strip_prefix("Task #")?.split_once(": ")?;
    let (status, rest) = rest.strip_prefix("Status: ")?.split_once('\n')?;
    if !matches!(status, "pending" | "in_progress" | "completed") {
        return None;
    }
    let description = rest.strip_prefix("Description: ")?;
    let (description, blocked_by) = match description.rsplit_once("\nBlocked by: ") {
        Some((body, ids)) => (
            body,
            ids.split(',')
                .take(MAX_TASKS)
                .filter_map(|key| id(&Value::String(key.trim().trim_start_matches('#').into())))
                .collect(),
        ),
        None => (description, vec![]),
    };
    Some(Task {
        id: id(&Value::String(key.into()))?,
        text: bounded(label),
        status: Status::from_wire(status),
        description: bounded(description),
        blocked_by,
        ..Default::default()
    })
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.into();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
fn text_list(content: &str) -> Option<Vec<Task>> {
    if content.trim() == "No tasks found" {
        return Some(vec![]);
    }
    let lines: Vec<_> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() || lines.len() > MAX_TASKS {
        return None;
    }
    lines
        .into_iter()
        .map(|line| {
            let (key, rest) = line.strip_prefix('#')?.split_once(" [")?;
            let (status, mut text) = rest.split_once("] ")?;
            if !matches!(status, "pending" | "in_progress" | "completed") {
                return None;
            }
            let mut blocked_by = vec![];
            if let Some((label, suffix)) = text.rsplit_once(" [blocked by ") {
                blocked_by = suffix
                    .strip_suffix(']')?
                    .split(',')
                    .map(|id| id.trim().trim_start_matches('#').to_owned())
                    .collect();
                text = label;
            }
            // Owner is ambiguous with a parenthesized subject in readable output;
            // preserve the complete label until a structured snapshot supplies it.
            Some(Task {
                id: id(&Value::String(key.into()))?,
                text: bounded(text),
                status: Status::from_wire(status),
                blocked_by,
                ..Default::default()
            })
        })
        .collect()
}
