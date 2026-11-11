//! Convert observed failing checks into local tasks without changing GitHub.
use super::{Check, PullRequest};
use crate::workflow::{Criterion, TaskStatus, WorkItem, Workflow};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSource {
    pub pull_request: String,
    pub head: String,
    pub name: String,
    pub url: Option<String>,
}

impl Check {
    pub fn needs_attention(&self) -> bool {
        matches!(
            self.state.as_str(),
            "FAILURE" | "ERROR" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED"
        )
    }
}

impl PullRequest {
    pub fn check_source(&self, index: usize) -> Option<CheckSource> {
        let check = self.checks.get(index)?;
        Some(CheckSource {
            pull_request: self.url.clone(),
            head: self.head.clone(),
            name: check.name.clone(),
            url: check.url.clone(),
        })
    }

    pub fn check_task(
        &self,
        index: usize,
        workspace: &Path,
        created_at: u64,
    ) -> Result<WorkItem, String> {
        let check = self
            .checks
            .get(index)
            .filter(|check| check.needs_attention())
            .ok_or_else(|| conduit::i18n::text("github.check_task_unavailable"))?;
        let source = self.check_source(index).unwrap();
        if check.name.len() > 4096
            || self.branch.len() > 1024
            || self.url.len() > 8192
            || check.url.as_ref().is_some_and(|url| url.len() > 8192)
        {
            return Err(conduit::i18n::text("github.output_limit"));
        }
        // Keep remote names and URLs inside a quoted data block, separate from instructions.
        let evidence = serde_json::to_string_pretty(&serde_json::json!({
            "pull_request": self.url,
            "pull_request_number": self.number,
            "head": self.head,
            "branch": self.branch,
            "local_head_at_fetch": self.local_head,
            "fetched_at_unix": self.fetched_at,
            "check": check.name,
            "state": check.state,
            "details": check.url,
        }))
        .map_err(|error| error.to_string())?;
        let evidence = evidence
            .lines()
            .map(|line| format!("    {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let name: String = check.name.chars().take(45).collect();
        let goal = conduit::i18n::format("github.check_task_goal", &[("evidence", evidence)]);
        if goal.len() > 32 * 1024 {
            return Err(conduit::i18n::text("github.output_limit"));
        }
        Ok(WorkItem {
            id: 0,
            workspace: workspace.into(),
            title: conduit::i18n::format("github.check_task_title", &[("name", name)]),
            goal,
            branch: Some(self.branch.clone()),
            sessions: Vec::new(),
            criteria: [
                "github.check_task_diagnosis",
                "github.check_task_fix",
                "github.check_task_verify",
            ]
            .into_iter()
            .map(|key| Criterion {
                text: conduit::i18n::text(key),
                done: false,
            })
            .collect(),
            status: TaskStatus::Planned,
            archived: false,
            created_at,
            origin: None,
            check_source: Some(source),
        })
    }
}

impl Workflow {
    pub fn check_task_id(&self, workspace: &Path, source: &CheckSource) -> Option<u64> {
        self.tasks
            .iter()
            .find(|task| task.workspace == workspace && task.check_source.as_ref() == Some(source))
            .map(|task| task.id)
    }

    pub fn import_check_task(&mut self, task: WorkItem) -> Result<u64, String> {
        let source = task
            .check_source
            .as_ref()
            .ok_or_else(|| conduit::i18n::text("github.check_task_unavailable"))?;
        if let Some(id) = self.check_task_id(&task.workspace, source) {
            // Preserve user edits, linked drafts, acceptance and archived/completed state.
            return Ok(id);
        }
        self.save_task(task)
    }
}

#[cfg(test)]
mod tests;
