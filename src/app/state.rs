//! Shared transient state for sessions, tasks, and panels.

use super::*;

/// Minimum time the startup page stays up once armed, so a CLI history scan
/// that finishes instantly cannot flash the page for a single frame.
pub(super) const STARTUP_MIN_DISPLAY: Duration = Duration::from_millis(300);
/// Upper bound on the startup page: a stalled scan must not lock the user
/// out of the app, whose sidebar then shows per-workspace import spinners.
pub(super) const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// Startup page state: gates the main layout until the first CLI history
/// scan merges, then releases it after [`STARTUP_MIN_DISPLAY`] (or
/// [`STARTUP_TIMEOUT`] when the scan never reports back).
pub(super) struct StartupState {
    /// When the page was armed; bounds the minimum display and the timeout.
    pub(super) started: Instant,
    /// The initial history scan has merged its results into the store.
    pub(super) imports_done: bool,
}

/// Composer routing while there is no session for the selected engine yet.
/// Applied only when the first message creates that engine's conversation.
#[derive(Clone)]
pub(super) struct ComposerSelection {
    pub(super) engine: Provider,
    pub(super) channel: ModelChannel,
    pub(super) model: Option<String>,
    pub(super) effort: ReasoningEffort,
    pub(super) opencode: crate::protocol::OpenCodeSettings,
}

impl ComposerSelection {
    pub(super) fn new(engine: Provider) -> Self {
        Self {
            engine,
            channel: ModelChannel::Cli,
            model: None,
            effort: ReasoningEffort::for_provider(engine),
            opencode: Default::default(),
        }
    }

    pub(super) fn from_session(session: &SessionRecord) -> Self {
        Self {
            engine: session.provider,
            channel: session.channel.clone(),
            model: session.model.clone(),
            effort: session.reasoning_effort,
            opencode: session.opencode.clone(),
        }
    }

    pub(super) fn apply(self, session: &mut SessionRecord) {
        debug_assert_eq!(self.engine, session.provider);
        session.channel = self.channel;
        session.model = self.model;
        session.reasoning_effort = self.effort;
        session.opencode = self.opencode;
    }

    pub(super) fn set_model(&mut self, model: Option<String>) {
        if self.model != model {
            self.effort = ReasoningEffort::for_provider(self.engine);
            self.opencode.variant = None;
            if let Some(options) = self.opencode.options.as_array_mut() {
                options.retain(|o| o["id"] != "effort");
            }
        }
        self.model = model;
    }
}

pub(super) struct SessionSummary {
    pub(super) id: u64,
    pub(super) provider: Provider,
    pub(super) message_count: usize,
}

impl From<&SessionRecord> for SessionSummary {
    fn from(session: &SessionRecord) -> Self {
        Self {
            id: session.id,
            provider: session.provider,
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

/// File churn of the latest turn, shown in the stats tags above the composer.
/// Paths stay sorted so repeated edits merge into one row.
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
#[cfg(test)]
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
    #[derive(serde::Deserialize)]
    struct Input {
        plan: Option<Vec<serde_json::Value>>,
        todos: Option<Vec<serde_json::Value>>,
    }
    let value = serde_json::from_str::<Input>(input).ok()?;
    let entries = value.plan.or(value.todos)?;
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

#[cfg(test)]
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
    pub(super) pending_save: Option<super::runtime::PendingTurn>,
    pub(super) cleanup_error: Option<String>,
    /// Git worktree shared by this turn's files and index.
    pub(super) worktree: PathBuf,
    pub(super) approvals: ApprovalQueue,
    pub(super) expanded_approvals: HashSet<String>,
    pub(super) question_drafts: HashMap<String, Vec<super::user_input::QuestionDraft>>,
    pub(super) prefix: String,
    pub(super) status: String,
    /// What the engine is doing right now, shown above the composer while
    /// the turn is still running.
    pub(super) activity: String,
    /// Model selected for this turn; `None` means the CLI default.
    pub(super) model: Option<String>,
    /// Snapshot of this turn's effort; picker changes apply only to the next turn.
    pub(super) reasoning_effort: ReasoningEffort,
    pub(super) opencode_variant: Option<String>,
    pub(super) message_index: MessageIndex,
    pub(super) usage: Usage,
    /// None for imported/legacy usage without a trustworthy output baseline.
    pub(super) output_tokens_at_start: Option<u64>,
    /// Fixed session speed baseline; usage events must never accumulate each other.
    pub(super) completed_usage: Usage,
    /// The model/channel window captured when the turn starts. Picker changes
    /// must not reinterpret the context of an already-running request.
    pub(super) context_window_fallback: u64,
    /// Wall time never resets when the provider compacts token usage.
    pub(super) turn_started: Instant,
}

impl ActiveRun {
    pub(super) fn waiting_status(&self) -> &'static str {
        if self.approvals.front().is_some_and(|a| a.is_user_input()) {
            "等待你的回答"
        } else {
            "等待你的审批"
        }
    }
    pub(super) fn new(
        runtime: ChatRuntime,
        worktree: PathBuf,
        prefix: String,
        usage: Usage,
        model: Option<String>,
        reasoning_effort: ReasoningEffort,
    ) -> Self {
        let now = Instant::now();
        let output_tokens_at_start = usage.context_tokens.map(|_| usage.output_tokens);
        let completed_usage = usage;
        let mut usage = usage;
        usage.turn_output_tokens = None;
        usage.turn_elapsed_ms = None;
        Self {
            runtime,
            pending_save: None,
            cleanup_error: None,
            worktree,
            approvals: ApprovalQueue::default(),
            expanded_approvals: HashSet::new(),
            question_drafts: HashMap::new(),
            prefix,
            status: "正在连接 CLI…".into(),
            activity: "正在启动 CLI…".into(),
            model,
            reasoning_effort,
            opencode_variant: None,
            message_index: MessageIndex::default(),
            usage,
            output_tokens_at_start,
            completed_usage,
            context_window_fallback: 0,
            turn_started: now,
        }
    }

    pub(super) fn cancel(&mut self) {
        self.runtime.cancel();
        self.approvals.clear();
        self.expanded_approvals.clear();
        self.question_drafts.clear();
        self.status = if self.cleanup_error.is_some() {
            "进程清理异常，正在后台重试…"
        } else {
            "正在停止…"
        }
        .into();
        self.activity = self.status.clone();
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
    pub(super) tag: Entity<InputState>,
    pub(super) _tag_subscription: Subscription,
    pub(super) _subscription: Subscription,
}

pub(super) struct CommitGeneration {
    pub(super) path: PathBuf,
    pub(super) runtime: ChatRuntime,
    pub(super) started: Instant,
    pub(super) original: String,
    pub(super) last_diagnostic: Option<String>,
    pub(super) parts: Vec<(String, String)>,
    pub(super) error: Option<String>,
    pub(super) cleanup_error: Option<String>,
}

pub(super) struct GitOperation {
    pub(super) probe: git::OperationProbe,
    pub(super) worktree: PathBuf,
    pub(super) draft: String,
    /// Whether the active commit also pushes; None for other Git operations.
    pub(super) commit_push: Option<bool>,
    pub(super) automatic_stage: bool,
}

/// One-shot background request that proposes a short session title.
pub(super) struct SessionNaming {
    pub(super) automatic: bool,
    pub(super) runtime: ChatRuntime,
    pub(super) started: Instant,
    pub(super) parts: Vec<(String, String)>,
    pub(super) error: Option<String>,
    pub(super) cleanup_error: Option<String>,
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

/// Drag payload (and invisible drag preview) for the right-sidebar edge.
#[derive(Clone)]
pub(super) struct DetailResize;

impl Render for DetailResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
