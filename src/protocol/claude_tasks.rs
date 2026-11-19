//! Native task lifecycles update their originating tool card, independently of tool returns.
use super::*;
use crate::{
    message_status::ToolStatus,
    subagents::{Agent, Status},
};

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Task {
    pub agent: bool,
    pub inline: bool,
    pub ambient: bool,
    tool_use_id: String,
    run_id: String,
    background: Option<bool>,
    ends_with_final_response: bool,
}

fn tool_status(status: Status) -> ToolStatus {
    match status {
        Status::Completed => ToolStatus::Completed,
        Status::Failed => ToolStatus::Failed,
        Status::Stopped => ToolStatus::Cancelled,
        Status::Unknown | Status::Idle => ToolStatus::MissingResult,
        _ => ToolStatus::Running,
    }
}

impl Task {
    pub(super) fn has_tool(&self) -> bool {
        !self.tool_use_id.is_empty()
    }
    pub(super) fn tool_event(&self, record: &Agent) -> Event {
        Event::Tool {
            key: if self.tool_use_id.is_empty() {
                format!("claude-task-{}", record.id)
            } else {
                self.tool_use_id.clone()
            },
            title: if self.tool_use_id.is_empty() {
                record.task.clone()
            } else {
                String::new()
            },
            detail: if record.result.is_empty() {
                record.activity.clone()
            } else {
                record.result.clone()
            },
            status: tool_status(record.status),
        }
    }
}

impl Protocol {
    pub(super) fn claude_task_event(&mut self, v: &Value) {
        let id = text(v, "task_id");
        if id.is_empty() {
            return;
        }
        let run = text(v, "run_id");
        if let Some(task) = self.agents.claude_tasks.get(id)
            && !task.run_id.is_empty()
            && (run.is_empty() || run < task.run_id.as_str())
        {
            return;
        }
        let parent = v["parent_task_id"]
            .as_str()
            .filter(|parent| self.agents.records.contains_key(*parent))
            .or(self.thread_id.as_deref())
            .unwrap_or("")
            .to_owned();
        let inline = self.thread_id.as_deref() == Some(parent.as_str()) || parent.is_empty();
        if self.agents.register(id, &parent).is_none() {
            return;
        }
        let previous = self.agents.claude_tasks.get(id).cloned();
        let task = self
            .agents
            .claude_tasks
            .entry(id.into())
            .or_insert_with(|| Task {
                inline,
                ..Default::default()
            });
        let record = self.agents.records.get_mut(id).unwrap();
        if !run.is_empty() && run != task.run_id {
            // SDK run IDs sort in opening order; older frames were rejected above.
            if !task.run_id.is_empty() || record.task_id.is_empty() {
                record.restart(run);
            } else {
                // A structured tool return may have registered this same run first.
                record.task_id = run.into();
            }
            if !task.run_id.is_empty() {
                task.ends_with_final_response = false;
                task.tool_use_id.clear();
                task.background = None;
            }
            task.run_id = run.into();
        } else if record.task_id.is_empty() {
            record.restart(id);
        }
        task.agent |= v["task_type"] == "local_agent" || v["subagent_type"].as_str().is_some();
        task.ambient |= v["ambient"] == true || v["skip_transcript"] == true;
        if let Some(background) = v["is_backgrounded"]
            .as_bool()
            .or_else(|| v["patch"]["is_backgrounded"].as_bool())
        {
            task.background = Some(background);
        }
        if v["parent_task_id"].as_str().is_some() {
            task.inline = inline;
        }
        if let Some(tool) = v["tool_use_id"].as_str().filter(|tool| !tool.is_empty()) {
            task.tool_use_id = tool.into();
        }
        if let Some(name) = v["subagent_type"].as_str() {
            record.name = name.into();
        }
        if let Some(description) = v["description"]
            .as_str()
            .or_else(|| v["patch"]["description"].as_str())
        {
            record.task = description.into();
        }
        match text(v, "subtype") {
            "task_updated" => {
                if let Some(status) = v["patch"]["status"].as_str()
                    && !record.status.terminal()
                {
                    record.set_status(super::subagents::native_status(status));
                }
                if let Some(error) = v["patch"]["error"].as_str() {
                    record.result = error.into();
                }
            }
            "task_progress" if !record.status.terminal() => {
                record.set_status(Status::Running);
                if let Some(activity) = v["summary"]
                    .as_str()
                    .or_else(|| v["last_tool_name"].as_str())
                {
                    record.activity = activity.into();
                }
            }
            "task_notification" => {
                let status = super::subagents::native_status(text(v, "status"));
                if !record.status.terminal() || record.status == status {
                    record.set_status(status);
                    record.result = text(v, "summary").into();
                }
            }
            _ => {}
        }
        if let Some(ms) = v["usage"]["duration_ms"].as_u64() {
            record.elapsed_ms = Some(ms);
        }
        if previous.as_ref() != Some(task) {
            self.agents.invalidate(id);
        }
    }

    pub(super) fn claude_task_tool_event(&self, mut event: Event) -> Event {
        if let Event::Tool {
            key,
            detail,
            status,
            ..
        } = &mut event
            && let Some((id, _)) = self
                .agents
                .claude_tasks
                .iter()
                .find(|(_, task)| !task.ambient && task.inline && task.tool_use_id == *key)
            && let Some(record) = self.agents.records.get(id)
        {
            *status = tool_status(record.status);
            if !record.result.is_empty() {
                *detail = record.result.clone();
            }
        }
        event
    }

    pub(super) fn claude_task_result(&mut self, key: &str, output: &Value, failed: bool) {
        if self.text_only {
            return;
        }
        let agent = output["isAsync"] == true || output["status"] == "async_launched";
        let background_id = output["backgroundTaskId"]
            .as_str()
            .or_else(|| agent.then(|| output["agentId"].as_str()).flatten());
        if let Some((id, task)) = self
            .agents
            .claude_tasks
            .iter()
            .find(|(_, task)| task.tool_use_id == key)
            && let Some(record) = self.agents.records.get_mut(id)
            && !record.status.terminal()
        {
            if failed {
                record.set_status(Status::Failed);
            } else if background_id.is_none() && output["interrupted"] == true {
                record.set_status(Status::Stopped);
            } else if background_id.is_none()
                && (task.background == Some(false) || output["status"] == "completed")
            {
                record.set_status(Status::Completed);
            }
        }
        if failed {
            return;
        }
        if let Some(id) = background_id.filter(|id| !id.is_empty()) {
            if !self.agents.records.contains_key(id) {
                self.claude_task_event(&json!({"subtype":"task_started","task_id":id,
                    "tool_use_id":key,"task_type":if agent {"local_agent"} else {"local_bash"}}));
            }
            if let Some(task) = self.agents.claude_tasks.get_mut(id) {
                task.background = Some(true);
                if task.tool_use_id.is_empty() {
                    task.tool_use_id = key.into();
                }
                task.ends_with_final_response |= output["backgroundEndsWithFinalResponse"] == true;
            }
        }
    }

    pub(super) fn finish_claude_background_tasks(&mut self) {
        for (id, task) in &self.agents.claude_tasks {
            if task.ends_with_final_response
                && let Some(record) = self.agents.records.get_mut(id)
                && record.status.active()
            {
                record.set_status(Status::Stopped);
            }
        }
    }
}

#[cfg(test)]
#[path = "claude_tasks/tests.rs"]
mod tests;
