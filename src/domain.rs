use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Claude,
    Codex,
    OpenCode,
}

impl Provider {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::OpenCode];
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
        }
    }

    #[cfg(test)]
    pub fn executable(self) -> Option<PathBuf> {
        crate::cli::resolve(self, &crate::cli::CliSettings::default())
            .ok()
            .map(|cli| cli.path)
    }
}

/// Session permission level. The mapping to each CLI differs: Codex uses
/// `sandbox=read-only | workspace-write | danger-full-access`, Claude uses
/// `--permission-mode default | acceptEdits | bypassPermissions`.
/// Claude read-only uses a restricted tool list and rejects approval requests.
/// FullAccess also disables Codex approval gates and uses Claude bypass mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionLevel {
    /// View only: Codex read-only sandbox.
    ReadOnly,
    /// Read plus edits inside the workspace (default).
    #[default]
    WorkspaceWrite,
    /// Unrestricted filesystem/network access; shown with a warning.
    FullAccess,
}

impl PermissionLevel {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "仅可查看",
            Self::WorkspaceWrite => "工作区内修改",
            Self::FullAccess => "完全权限",
        }
    }

    pub fn codex_approval_policy(self) -> &'static str {
        match self {
            Self::FullAccess | Self::ReadOnly => "never",
            Self::WorkspaceWrite => "on-request",
        }
    }

    pub fn codex_sandbox(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::FullAccess => "danger-full-access",
        }
    }

    pub fn claude_permission_mode(self) -> &'static str {
        match self {
            Self::ReadOnly => "default",
            Self::WorkspaceWrite => "acceptEdits",
            Self::FullAccess => "bypassPermissions",
        }
    }
}

/// Token usage snapshot for the running turn / session.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    /// Model context window; 0 when the CLI has not reported it.
    pub context_window: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: u64,
    pub provider: Provider,
    pub created_at: u64,
    pub exit_code: Option<u32>,
    #[serde(default)]
    pub title: String,
    /// Hidden from the sidebar unless archived sessions are shown.
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub native_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub channel: crate::providers::ModelChannel,
    #[serde(default)]
    pub reasoning_effort: crate::providers::ReasoningEffort,
    #[serde(default)]
    pub opencode: crate::protocol::OpenCodeSettings,
    #[serde(default)]
    pub messages: Vec<ChatItem>,
    #[serde(default)]
    pub interrupted: bool,
    /// Permission level applied to the next turn of this session.
    #[serde(default)]
    pub permission: PermissionLevel,
    /// Last reported token usage; transient, not used for restore.
    #[serde(default, skip_serializing_if = "crate::domain::usage_is_zero")]
    pub usage: Usage,
}

fn usage_is_zero(usage: &Usage) -> bool {
    usage.total_tokens == 0
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ItemKind {
    User,
    Assistant,
    /// Provider-supplied reasoning summary; never raw private chain-of-thought.
    Reasoning,
    Tool,
    Notice,
    /// CLI stream error shown as an actionable inline message card.
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatItem {
    /// Total wall time of the CLI turn, recorded when it finishes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    pub key: String,
    /// Unix timestamp for when this visible item first appeared.
    #[serde(default)]
    pub created_at: u64,
    pub kind: ItemKind,
    pub text: String,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub status: String,
    /// Permission decisions are independent of the tool's execution status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ToolApproval>,
    /// Model that produced this reply; absent for messages written by older
    /// builds or imported without model metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolApproval {
    pub request: String,
    pub status: String,
}

impl SessionRecord {
    /// Match exact provider item IDs first, then a matching Claude tool in this
    /// turn. Some permission requests precede the tool-start event, so create a
    /// placeholder with the provider key for the later event to update.
    pub fn record_tool_approval(
        &mut self,
        prefix: &str,
        request: &str,
        title: &str,
        detail: &str,
        status: &str,
    ) {
        let turn = format!("{prefix}-");
        let details = serde_json::from_str::<serde_json::Value>(detail).unwrap_or_default();
        let native_key = ["itemId", "tool_use_id", "toolCallId"]
            .iter()
            .find_map(|key| details[*key].as_str().filter(|id| !id.is_empty()));
        let key = native_key.map(|key| format!("{turn}{key}"));
        let tool_name = title.strip_prefix("请求使用 ").unwrap_or(title);
        let index = self
            .messages
            .iter()
            .position(|item| {
                item.kind == ItemKind::Tool
                    && item.key.starts_with(&turn)
                    && item.approval.as_ref().is_some_and(|a| a.request == request)
            })
            .or_else(|| {
                self.messages
                    .iter()
                    .position(|item| item.kind == ItemKind::Tool && key.as_ref() == Some(&item.key))
            })
            .or_else(|| {
                // Never attach a decision to an unrelated or previous-turn tool.
                if native_key.is_some() {
                    return None;
                }
                self.messages.iter().rposition(|item| {
                    item.kind == ItemKind::Tool
                        && item.key.starts_with(&turn)
                        && item.text == tool_name
                        && item.approval.is_none()
                        && (details["input"].is_null()
                            || item.input.trim().is_empty()
                            || serde_json::from_str::<serde_json::Value>(&item.input)
                                .ok()
                                .as_ref()
                                == Some(&details["input"]))
                })
            })
            .unwrap_or_else(|| {
                self.messages.push(ChatItem {
                    elapsed_ms: None,
                    key: key.unwrap_or_else(|| format!("{turn}permission-{request}")),
                    created_at: 0,
                    kind: ItemKind::Tool,
                    text: tool_name.into(),
                    input: details
                        .get("input")
                        .or_else(|| details.get("toolInput"))
                        .and_then(|value| serde_json::to_string_pretty(value).ok())
                        .unwrap_or_default(),
                    detail: String::new(),
                    status: String::new(),
                    approval: None,
                    model: None,
                });
                self.messages.len() - 1
            });
        self.messages[index].approval = Some(ToolApproval {
            request: request.into(),
            status: status.into(),
        });
    }

    /// Fold notices written by earlier builds into their tool cards on load.
    pub fn migrate_approval_notices(&mut self) {
        for mut item in std::mem::take(&mut self.messages) {
            let decision = if item.kind == ItemKind::Notice {
                item.text
                    .strip_prefix("已提交允许本次操作：")
                    .map(|title| ("已允许", title))
                    .or_else(|| {
                        item.text
                            .strip_prefix("已提交拒绝本次操作：")
                            .map(|title| ("已拒绝", title))
                    })
            } else {
                None
            };
            if let Some((status, title)) = decision
                && let Some((prefix, _)) = item.key.rsplit_once("-approval-")
            {
                let (prefix, request, title, detail) = (
                    prefix.to_owned(),
                    item.key.clone(),
                    title.to_owned(),
                    item.detail.clone(),
                );
                self.record_tool_approval(&prefix, &request, &title, &detail, status);
            } else {
                if item.kind == ItemKind::Tool
                    && let Some(placeholder) = self
                        .messages
                        .iter_mut()
                        .find(|previous| previous.key == item.key)
                {
                    if item.approval.is_none() {
                        item.approval = placeholder.approval.take();
                    }
                    *placeholder = item;
                } else {
                    self.messages.push(item);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub path: PathBuf,
    pub sessions: Vec<SessionRecord>,
}

impl Workspace {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}
