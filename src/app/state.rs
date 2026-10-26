//! Shared transient state for sessions, tasks, and panels.

use super::*;

pub(super) struct SessionSummary {
    pub(super) id: u64,
    pub(super) provider: Provider,
    pub(super) native_id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) message_count: usize,
}

impl From<&SessionRecord> for SessionSummary {
    fn from(session: &SessionRecord) -> Self {
        Self {
            id: session.id,
            provider: session.provider,
            native_id: session.native_id.clone(),
            model: session.model.clone(),
            message_count: session.messages.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum PlanStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
}

impl PlanStatus {
    pub(super) fn from_wire(status: &str) -> Self {
        match status {
            "completed" => Self::Completed,
            "in_progress" | "inProgress" => Self::InProgress,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PlanStep {
    pub(super) text: String,
    pub(super) status: PlanStatus,
}

/// File churn of the latest turn, shown in the floating stats pill above the
/// composer. Paths stay sorted so repeated edits merge into one row.
#[derive(Clone, Debug, Default)]
pub(super) struct TurnFileStats {
    pub(super) files: BTreeMap<String, FileStat>,
    pub(super) added: usize,
    pub(super) removed: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FileStat {
    pub(super) added: usize,
    pub(super) removed: usize,
}

/// Most recent plan emitted inside the latest user turn. Live Codex turns keep
/// it in `ConduitApp::plans`; imported history falls back to the tool input.
pub(super) fn plan_from_messages(session: &SessionRecord) -> Vec<PlanStep> {
    let start = session
        .messages
        .iter()
        .rposition(|item| item.kind == ItemKind::User)
        .unwrap_or(0);
    session.messages[start..]
        .iter()
        .rev()
        .filter(|item| item.kind == ItemKind::Tool)
        .find_map(|item| plan_steps_from_input(&item.input))
        .unwrap_or_default()
}

pub(super) fn plan_steps_from_input(input: &str) -> Option<Vec<PlanStep>> {
    let value = serde_json::from_str::<serde_json::Value>(input).ok()?;
    let entries = value
        .get("plan")
        .or_else(|| value.get("todos"))?
        .as_array()?;
    let steps = entries
        .iter()
        .filter_map(|entry| {
            let text = entry
                .get("step")
                .or_else(|| entry.get("content"))
                .or_else(|| entry.get("activeForm"))
                .and_then(serde_json::Value::as_str)?;
            if text.trim().is_empty() {
                return None;
            }
            Some(PlanStep {
                text: text.to_string(),
                status: PlanStatus::from_wire(
                    entry
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("pending"),
                ),
            })
        })
        .collect::<Vec<_>>();
    (!steps.is_empty()).then_some(steps)
}

/// Editor diffs carry absolute paths; the stats panel shows paths relative to
/// the workspace so long drive prefixes do not crowd out the counts.
pub(super) fn workspace_relative_path(path: &str, workspace: Option<&str>) -> String {
    // Workspaces opened through the Windows picker keep the `\\?\` verbatim
    // prefix, while editor tools report plain drive paths.
    let normalize = |value: &str| {
        let value = value.replace('\\', "/");
        match value.strip_prefix("//?/") {
            Some(stripped) => stripped.to_string(),
            None => value,
        }
    };
    let normalized = normalize(path);
    let Some(workspace) = workspace else {
        return normalized;
    };
    let root = normalize(workspace);
    let root = root.trim_end_matches('/');
    if !root.is_empty()
        && normalized
            .get(..root.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(root))
    {
        let rest = normalized[root.len()..].trim_start_matches('/');
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    normalized
}

pub(super) fn turn_file_stats(session: &SessionRecord) -> TurnFileStats {
    let start = session
        .messages
        .iter()
        .rposition(|item| item.kind == ItemKind::User)
        .unwrap_or(0);
    let mut stats = TurnFileStats::default();
    for item in &session.messages[start..] {
        if item.kind != ItemKind::Tool {
            continue;
        }
        let Some(diffs) = edit_tool_diffs(item) else {
            continue;
        };
        for diff in diffs {
            let file = stats.files.entry(diff.path).or_default();
            file.added += diff.added;
            file.removed += diff.removed;
            stats.added += diff.added;
            stats.removed += diff.removed;
        }
    }
    stats
}

pub(super) struct ActiveRun {
    pub(super) runtime: ChatRuntime,
    /// Git worktree shared by this turn's files and index.
    pub(super) worktree: PathBuf,
    pub(super) approvals: ApprovalQueue,
    pub(super) expanded_approvals: HashSet<String>,
    pub(super) prefix: String,
    pub(super) status: String,
    /// What the engine is doing right now, shown above the composer while
    /// the turn is still running.
    pub(super) activity: String,
    /// Model selected for this turn; `None` means the CLI default.
    pub(super) model: Option<String>,
    pub(super) message_index: MessageIndex,
    /// Usage reported by the running turn, plus turn-local token accounting
    /// used to derive the streaming tokens/s figure.
    pub(super) usage: Usage,
    pub(super) turn_tokens_at_start: u64,
    pub(super) turn_started: Instant,
    pub(super) last_usage_event: Instant,
}

impl ActiveRun {
    pub(super) fn new(
        runtime: ChatRuntime,
        worktree: PathBuf,
        prefix: String,
        usage: Usage,
        model: Option<String>,
    ) -> Self {
        let now = Instant::now();
        Self {
            runtime,
            worktree,
            approvals: ApprovalQueue::default(),
            expanded_approvals: HashSet::new(),
            prefix,
            status: "正在连接 CLI…".into(),
            activity: "正在启动 CLI…".into(),
            model,
            message_index: MessageIndex::default(),
            usage,
            turn_tokens_at_start: usage.total_tokens,
            turn_started: now,
            last_usage_event: now,
        }
    }

    pub(super) fn cancel(&mut self) {
        self.runtime.cancel();
        self.approvals.clear();
        self.expanded_approvals.clear();
        self.status = "正在停止…".into();
        self.activity = "正在停止…".into();
    }
}

/// Append-only message lookup, shared by every event in an active run.
#[derive(Default)]
pub(super) struct MessageIndex {
    pub(super) keys: HashMap<String, usize>,
    pub(super) indexed_len: usize,
}

impl MessageIndex {
    pub(super) fn find(&mut self, messages: &[ChatItem], key: &str) -> Option<usize> {
        if self.indexed_len > messages.len() {
            self.keys.clear();
            self.indexed_len = 0;
        }
        for (index, item) in messages.iter().enumerate().skip(self.indexed_len) {
            self.keys.entry(item.key.clone()).or_insert(index);
        }
        self.indexed_len = messages.len();
        self.keys.get(key).copied()
    }
}

pub(super) struct CommitDraft {
    pub(super) input: Entity<TextareaState>,
    pub(super) _subscription: Subscription,
}

pub(super) struct CommitGeneration {
    pub(super) path: PathBuf,
    pub(super) runtime: ChatRuntime,
    pub(super) started: Instant,
    pub(super) original: String,
    pub(super) parts: Vec<(String, String)>,
    pub(super) error: Option<String>,
}

pub(super) struct GitOperation {
    pub(super) probe: git::OperationProbe,
    pub(super) worktree: PathBuf,
    pub(super) draft: String,
}

/// One-shot background request that proposes a short session title.
pub(super) struct SessionNaming {
    pub(super) runtime: ChatRuntime,
    pub(super) started: Instant,
    pub(super) parts: Vec<(String, String)>,
    pub(super) error: Option<String>,
}

pub(super) struct GitFeedback {
    pub(super) text: String,
    pub(super) error: bool,
}

pub(super) fn update_commit_text(
    parts: &mut Vec<(String, String)>,
    key: String,
    text: String,
    snapshot: bool,
) {
    let index = parts
        .iter()
        .position(|(id, _)| id == &key)
        .unwrap_or_else(|| {
            parts.push((key, String::new()));
            parts.len() - 1
        });
    if snapshot {
        parts[index].1 = text;
    } else {
        parts[index].1.push_str(&text);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    General,
    Appearance,
    Session,
    Git,
    Cli,
    Provider(Provider),
    Models,
}

/// Independent tool windows sharing the application's right-hand dock.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailView {
    Tool,
    Git,
}

/// Drag payload (and invisible drag preview) for the right-sidebar edge.
#[derive(Clone)]
pub(super) struct DetailResize;

impl Render for DetailResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
