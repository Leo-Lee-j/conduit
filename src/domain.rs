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

/// Cumulative consumption and an independent latest-request context snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    /// Model context window; 0 when the CLI has not reported it.
    pub context_window: u64,
    /// Latest reported request size, never cumulative consumption. Missing on
    /// legacy sessions or providers that do not report current context.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    /// Provider-reported output for this user turn, excluding resumed history.
    #[serde(default)]
    pub turn_output_tokens: Option<u64>,
    #[serde(default)]
    pub turn_elapsed_ms: Option<u64>,
    #[serde(default)]
    pub context_window_estimated: bool,
    /// Output and wall time from reliably measured turns in this session.
    #[serde(default)]
    pub session_output_tokens: u64,
    #[serde(default)]
    pub session_elapsed_ms: u64,
}

impl Usage {
    /// Missing/reset cumulative baselines cannot produce a trustworthy speed.
    pub fn sample_turn(mut self, output_baseline: Option<u64>, elapsed_ms: u64) -> Self {
        self.turn_output_tokens = self.turn_output_tokens.or_else(|| {
            output_baseline.and_then(|baseline| self.output_tokens.checked_sub(baseline))
        });
        self.turn_elapsed_ms = Some(elapsed_ms);
        self
    }

    pub fn context_ratio(&self) -> Option<f64> {
        self.context_tokens
            .filter(|_| self.context_window > 0)
            .map(|tokens| (tokens as f64 / self.context_window as f64).clamp(0.0, 1.0))
    }

    /// Rebuild the session sample from completed turns plus the current turn.
    /// Repeated usage events replace the current sample rather than adding it twice.
    pub fn with_session_speed(mut self, completed: Usage) -> Self {
        self.session_output_tokens = completed.session_output_tokens;
        self.session_elapsed_ms = completed.session_elapsed_ms;
        if let (Some(output), Some(elapsed)) = (self.turn_output_tokens, self.turn_elapsed_ms)
            && elapsed > 0
        {
            self.session_output_tokens = self.session_output_tokens.saturating_add(output);
            self.session_elapsed_ms = self.session_elapsed_ms.saturating_add(elapsed);
        }
        self
    }

    /// Weighted session wall-clock average, NOT a live model generation rate.
    pub fn average_output_speed(&self) -> Option<u64> {
        let output = self.session_output_tokens;
        let elapsed = self.session_elapsed_ms;
        (output > 0 && elapsed > 0)
            .then(|| (output as f64 * 1_000.0 / elapsed as f64).round() as u64)
    }
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
    /// Last reported usage and persisted session speed samples.
    #[serde(default, skip_serializing_if = "crate::domain::usage_is_zero")]
    pub usage: Usage,
}

fn usage_is_zero(usage: &Usage) -> bool {
    *usage == Usage::default()
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

#[cfg(test)]
mod usage_tests {
    use super::Usage;

    #[test]
    fn speed_counts_only_output_not_input_cache_or_history() {
        let usage = Usage {
            input_tokens: 100_000,
            cached_input_tokens: 95_000,
            output_tokens: 10_000,
            total_tokens: 110_000,
            ..Default::default()
        }
        .sample_turn(Some(9_000), 10_000)
        .with_session_speed(Usage::default());
        assert_eq!(usage.turn_output_tokens, Some(1_000));
        assert_eq!(usage.average_output_speed(), Some(100));
    }

    #[test]
    fn imported_history_or_reset_does_not_manufacture_a_speed() {
        let usage = Usage {
            output_tokens: 900_000,
            ..Default::default()
        };
        assert_eq!(usage.sample_turn(None, 10_000).average_output_speed(), None);
        assert_eq!(
            usage
                .sample_turn(Some(1_000_000), 10_000)
                .average_output_speed(),
            None
        );
        assert_eq!(usage.sample_turn(Some(0), 0).average_output_speed(), None);
    }

    #[test]
    fn provider_turn_output_wins_over_cumulative_baseline() {
        let usage = Usage {
            output_tokens: 20_000,
            turn_output_tokens: Some(500),
            ..Default::default()
        }
        .sample_turn(Some(10_000), 10_000)
        .with_session_speed(Usage::default());
        assert_eq!(usage.average_output_speed(), Some(50));
        // An event sample stays fixed while tools or approvals are pending.
        assert_eq!(usage.average_output_speed(), Some(50));
    }

    #[test]
    fn session_speed_is_weighted_across_turns_and_replaces_stream_updates() {
        let first = Usage {
            turn_output_tokens: Some(100),
            ..Default::default()
        }
        .sample_turn(None, 1_000)
        .with_session_speed(Usage::default());
        let interim = Usage {
            turn_output_tokens: Some(300),
            ..Default::default()
        }
        .sample_turn(None, 3_000)
        .with_session_speed(first);
        assert_eq!(interim.session_output_tokens, 400);
        let final_sample = Usage {
            turn_output_tokens: Some(900),
            ..Default::default()
        }
        .sample_turn(None, 9_000)
        .with_session_speed(first);
        assert_eq!(final_sample.session_output_tokens, 1_000);
        assert_eq!(final_sample.session_elapsed_ms, 10_000);
        assert_eq!(final_sample.average_output_speed(), Some(100));
        let third = Usage {
            turn_output_tokens: Some(100),
            ..Default::default()
        }
        .sample_turn(None, 10_000)
        .with_session_speed(final_sample);
        assert_eq!(third.average_output_speed(), Some(55));
        let missing = Usage::default()
            .sample_turn(None, 90_000)
            .with_session_speed(third);
        assert_eq!(missing.average_output_speed(), Some(55));
        assert_eq!(missing.session_elapsed_ms, 20_000);
        let restored: Usage =
            serde_json::from_str(&serde_json::to_string(&missing).unwrap()).unwrap();
        assert_eq!(restored.average_output_speed(), Some(55));
    }

    #[test]
    fn known_zero_output_turn_counts_time_but_unknown_and_zero_time_do_not() {
        let base = Usage {
            session_output_tokens: 100,
            session_elapsed_ms: 1_000,
            ..Default::default()
        };
        let zero = Usage {
            turn_output_tokens: Some(0),
            ..Default::default()
        }
        .sample_turn(None, 1_000)
        .with_session_speed(base);
        assert_eq!(zero.average_output_speed(), Some(50));
        let unknown = Usage::default()
            .sample_turn(None, 9_000)
            .with_session_speed(base);
        assert_eq!(unknown.average_output_speed(), Some(100));
        let untimed = Usage {
            turn_output_tokens: Some(1_000),
            ..Default::default()
        }
        .sample_turn(None, 0)
        .with_session_speed(base);
        assert_eq!(untimed.average_output_speed(), Some(100));
    }

    #[test]
    fn context_uses_latest_request_and_can_shrink_independently_of_totals() {
        let mut usage = Usage {
            total_tokens: 300_000,
            context_tokens: Some(40_000),
            context_window: 200_000,
            ..Default::default()
        };
        assert_eq!(usage.context_ratio(), Some(0.2));
        usage.total_tokens = 400_000;
        usage.context_tokens = Some(10_000);
        assert_eq!(usage.context_ratio(), Some(0.05));
        usage.context_tokens = Some(300_000);
        assert_eq!(usage.context_ratio(), Some(1.0));
    }

    #[test]
    fn legacy_usage_does_not_treat_total_as_context() {
        let usage: Usage = serde_json::from_str(
            r#"{
            "input_tokens":200000,"cached_input_tokens":100000,
            "output_tokens":100000,"total_tokens":300000,"context_window":200000
        }"#,
        )
        .unwrap();
        assert_eq!(usage.context_ratio(), None);
        assert_eq!(usage.average_output_speed(), None);
        let restored: Usage =
            serde_json::from_str(&serde_json::to_string(&usage).unwrap()).unwrap();
        assert_eq!(restored, usage);
    }
}
