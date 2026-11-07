//! Git snapshots, bounded comparisons, and explicit repository operations.
//!
//! Snapshot probes only execute non-mutating branch/ref queries, status and diff.
//! `--no-optional-locks` keeps `status`/`diff` from taking the index lock, so
//! viewing the panel never competes with the CLI's own Git commands. Output is
//! bounded and every probe runs on its own thread with a timeout and a killed
//! process tree, mirroring the CLI version probe.
use crate::child::{Child, available};
use anyhow::{Context, Result, anyhow, bail};
mod branches;
mod compare;
mod discard;
mod log;
mod selected;
pub use branches::{
    BranchAction, BranchContext, BranchInfo, BranchRequest, IntegrationState, repository_scope,
};
pub use compare::{
    Comparison, ComparisonKind, ComparisonRequest, load_comparison, load_comparison_diff,
};
pub use log::{
    CommitDetails, CommitDiffRequest, History, HistoryQuery, LogCommit, load_commit_details,
    load_commit_diff, load_history,
};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

/// Files listed in the panel; beyond this only a note is shown.
pub const MAX_FILES: usize = 2_000;
pub const MAX_BRANCHES: usize = 2_000;
/// Bytes of the staged diff kept for commit-message generation.
pub const MAX_DIFF_BYTES: usize = 200 * 1024;
/// Hard cap on the bytes read from one Git command (the rest is drained).
const MAX_COMMAND_BYTES: usize = 512 * 1024;
/// Upper bound for a single file diff shown in the diff window.
const MAX_FILE_DIFF_BYTES: usize = 4 * 1024 * 1024;
/// Context large enough that Git emits the whole file on both sides, so the
/// viewer can reveal unchanged lines instead of merely widening hunk context.
const FULL_FILE_CONTEXT_LINES: usize = 1_000_000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const READ_CHUNK: usize = 64 * 1024;

/// Resolve a worktree's local scope without starting Git on the UI thread.
/// A `.git` file marks a linked worktree; directories inside one repository
/// share its index, while separate linked worktrees have separate indexes.
pub fn worktree_root(path: &Path) -> PathBuf {
    path.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(path)
        .to_path_buf()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangedFile {
    /// Porcelain v1 index status (`X`).
    pub index: char,
    /// Porcelain v1 worktree status (`Y`).
    pub worktree: char,
    pub path: String,
    /// Previous path for renames/copies.
    pub original: Option<String>,
}

impl ChangedFile {
    /// Status code that best describes the change for display purposes.
    fn code(&self) -> char {
        if self.worktree != ' ' && self.worktree != '?' {
            self.worktree
        } else {
            self.index
        }
    }

    /// Collapsed sidebar group: untracked and added files count as additions,
    /// deletions stay separate, and every other status is a modification.
    pub fn kind(&self) -> ChangeKind {
        match self.code() {
            '?' | 'A' => ChangeKind::Added,
            'D' => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        }
    }

    pub fn staged(&self) -> bool {
        self.index != ' ' && self.index != '?'
    }

    pub fn unstaged(&self) -> bool {
        self.worktree != ' ' && self.worktree != '?'
    }

    pub fn untracked(&self) -> bool {
        self.index == '?' && self.worktree == '?'
    }
}

/// Collapsed change groups shown in the Git sidebar, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "新增",
            Self::Modified => "修改",
            Self::Deleted => "删除",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub branch: String,
    pub tags: Vec<String>,
    pub files: Vec<ChangedFile>,
    /// Staged patch used only for commit-message generation.
    pub staged_diff: String,
    pub notes: Vec<String>,
    pub target: CommitTarget,
}

/// Filter the bounded generation patch using exact Git-quoted file headers.
/// Include both sides of renames, including when rename detection is disabled.
pub fn selected_staged_patch(snapshot: &Snapshot, selected: &[String]) -> String {
    let mut headers = std::collections::HashSet::new();
    for file in snapshot
        .files
        .iter()
        .filter(|file| file.staged() && selected.contains(&file.path))
    {
        let original = file.original.as_deref().unwrap_or(&file.path);
        for quote_unicode in [false, true] {
            for (from, to) in [
                (original, file.path.as_str()),
                (original, original),
                (&file.path, &file.path),
            ] {
                headers.insert(format!(
                    "diff --git {} {}",
                    quote_diff_path(&format!("a/{from}"), quote_unicode),
                    quote_diff_path(&format!("b/{to}"), quote_unicode)
                ));
            }
        }
    }
    let mut include = false;
    let mut patch = String::new();
    for line in snapshot.staged_diff.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            include = headers.contains(line.trim_end_matches(['\r', '\n']));
        }
        if include {
            patch.push_str(line);
        }
    }
    patch
}

fn quote_diff_path(path: &str, quote_unicode: bool) -> String {
    if !path
        .bytes()
        .any(|b| b < 32 || b == 127 || b == b'"' || b == b'\\' || (quote_unicode && b >= 128))
    {
        return path.into();
    }
    let mut out = String::from("\"");
    for ch in path.chars() {
        match ch {
            '\x07' => out.push_str("\\a"),
            '\x08' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\x0b' => out.push_str("\\v"),
            '\x0c' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            ch if ch.is_ascii_control() || (quote_unicode && !ch.is_ascii()) => {
                for b in ch.encode_utf8(&mut [0; 4]).bytes() {
                    out.push_str(&format!("\\{b:03o}"));
                }
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileDiffScope {
    Staged,
    Unstaged,
    #[default]
    All,
}

impl FileDiffScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "已暂存（待提交）",
            Self::Unstaged => "未暂存",
            Self::All => "全部修改",
        }
    }
}

/// Patch for one file in the selected comparison scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub patch: String,
    pub notes: Vec<String>,
}

/// Identity of the index and HEAD the user reviewed before clicking Commit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitTarget {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub staged: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub remote: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub current: String,
    pub detached: bool,
    pub branches: Vec<Branch>,
    pub truncated: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub context: BranchContext,
    pub branch_info: Vec<BranchInfo>,
    pub integration: Option<IntegrationState>,
    pub conflicts: Vec<String>,
    pub remotes: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Operation {
    Stage {
        files: Vec<String>,
        target: CommitTarget,
    },
    /// Refresh only paths already in the index; never add untracked files.
    StageTracked {
        files: Vec<String>,
        target: CommitTarget,
    },
    Unstage {
        files: Vec<String>,
        target: CommitTarget,
    },
    Discard {
        file: ChangedFile,
        target: CommitTarget,
    },
    Branch(Box<BranchRequest>),
    Commit {
        /// None commits the full index; Some commits only these staged paths.
        files: Option<Vec<String>>,
        message: String,
        target: CommitTarget,
        push: bool,
        tag: Option<String>,
    },
    RetryTag(TagRetry),
}

#[derive(Clone, Debug)]
pub struct TagRetry {
    pub branch: String,
    pub remote: String,
    pub remote_url: String,
    pub merge: String,
    pub tag: String,
    pub oid: String,
}

#[derive(Clone, Debug)]
pub struct RemoteTags {
    pub branch: String,
    pub remote: String,
    pub names: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OperationResult {
    pub retry_completed: bool,
    pub retry_tag: Option<TagRetry>,
    pub committed: bool,
    pub message: String,
    pub error: Option<String>,
    pub worktree_created: Option<PathBuf>,
}

impl Snapshot {
    pub fn staged_count(&self) -> usize {
        self.files.iter().filter(|file| file.staged()).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.files.iter().filter(|file| file.unstaged()).count()
    }

    pub fn untracked_count(&self) -> usize {
        self.files.iter().filter(|file| file.untracked()).count()
    }

    pub fn is_clean(&self) -> bool {
        self.files.is_empty()
    }
}

/// Background probe for one workspace snapshot, modelled on `cli::VersionProbe`.
pub struct Probe<T> {
    pub path: PathBuf,
    pub events: Receiver<Result<T, String>>,
    cancelled: Arc<AtomicBool>,
}

pub type GitProbe = Probe<Snapshot>;
pub type BranchProbe = Probe<Repository>;
pub type OperationProbe = Probe<OperationResult>;
pub type RemoteTagsProbe = Probe<RemoteTags>;

impl<T: Send + 'static> Probe<T> {
    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn test_pending(path: PathBuf) -> Self {
        let (_, events) = mpsc::channel();
        Self {
            path,
            events,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }
    fn start_with(path: PathBuf, load: fn(&Path, &AtomicBool) -> Result<T>) -> Self {
        Self::spawn(path, load)
    }
    fn spawn(
        path: PathBuf,
        load: impl FnOnce(&Path, &AtomicBool) -> Result<T> + Send + 'static,
    ) -> Self {
        let (sender, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let workspace = path.clone();
        thread::spawn(move || {
            let result = load(&workspace, &cancel).map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        Self {
            path,
            events,
            cancelled,
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl GitProbe {
    pub fn start(path: PathBuf, cached_tags: Option<Vec<String>>) -> Self {
        Self::spawn(path, move |path, cancel| match cached_tags {
            Some(tags) => load_with_tags(path, cancel, Some(tags)),
            None => load(path, cancel),
        })
    }
}

impl BranchProbe {
    pub fn start(path: PathBuf) -> Self {
        Self::start_with(path, load_repository)
    }
}

impl RemoteTagsProbe {
    pub fn start(path: PathBuf) -> Self {
        Self::start_with(path, load_remote_tags)
    }
}

impl OperationProbe {
    pub fn start(path: PathBuf, operation: Operation) -> Self {
        Self::spawn(path, move |workspace, cancel| {
            Ok(operate(workspace, operation, cancel))
        })
    }
}

impl<T> Drop for Probe<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Lightweight branch metadata, also read while the changes drawer is closed.
pub fn load_repository(workspace: &Path, cancel: &AtomicBool) -> Result<Repository> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!("未找到 git.exe；请安装 Git 或将其加入 PATH 后重试"))?;
    let inside = run_git(
        &git,
        workspace,
        &["rev-parse", "--is-inside-work-tree"],
        cancel,
    )?;
    if inside.trim() != "true" {
        bail!("该工作区不是 Git 仓库");
    }
    let expected_context = branches::context(&git, workspace, cancel)?;
    // symbolic-ref preserves the branch name even before the first commit.
    let symbolic = run_git(
        &git,
        workspace,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancel,
    );
    let (current, detached) = match symbolic {
        Ok(name) => (name.trim().to_string(), false),
        Err(_) => (
            run_git(&git, workspace, &["rev-parse", "--short", "HEAD"], cancel)?
                .trim()
                .to_string(),
            true,
        ),
    };
    let refs = run_git(
        &git,
        workspace,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)%00%(symref)",
            "refs/heads",
            "refs/remotes",
        ],
        cancel,
    )?;
    let mut branches = parse_branches(&refs);
    if !detached
        && !branches
            .iter()
            .any(|branch| !branch.remote && branch.name == current)
    {
        branches.insert(
            0,
            Branch {
                name: current.clone(),
                remote: false,
            },
        );
    }
    let truncated = branches.len() > MAX_BRANCHES || refs.len() >= MAX_COMMAND_BYTES;
    // Keep the selected branch visible even in repositories with many refs.
    branches.sort_by_key(|branch| branch.remote || branch.name != current);
    branches.truncate(MAX_BRANCHES);
    let upstream = if detached {
        None
    } else {
        run_git(
            &git,
            workspace,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
            cancel,
        )
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    };
    let ahead = if let Some(upstream) = &upstream {
        run_git(
            &git,
            workspace,
            &["rev-list", "--count", &format!("{upstream}..HEAD")],
            cancel,
        )
    } else {
        run_git(
            &git,
            workspace,
            &["rev-list", "--count", "HEAD", "--not", "--remotes"],
            cancel,
        )
    }
    .ok()
    .and_then(|value| value.trim().parse().ok())
    .unwrap_or(0);
    let (context, branch_info, integration, conflicts, remotes) =
        branches::metadata(&git, workspace, cancel)?;
    if context != expected_context {
        bail!("读取期间分支或 HEAD 已变化，请刷新后重试");
    }
    Ok(Repository {
        current,
        detached,
        branches,
        truncated,
        upstream,
        ahead,
        context,
        branch_info,
        integration,
        conflicts,
        remotes,
    })
}

fn parse_branches(refs: &str) -> Vec<Branch> {
    refs.lines()
        .filter_map(|line| {
            let (name, symbolic) = line.split_once('\0')?;
            if !symbolic.is_empty() {
                return None;
            }
            let (name, remote) = if let Some(name) = name.strip_prefix("refs/heads/") {
                (name, false)
            } else {
                (name.strip_prefix("refs/remotes/")?, true)
            };
            Some(Branch {
                name: name.to_string(),
                remote,
            })
        })
        .collect()
}

/// Locate `git.exe` through PATH, then the usual Windows install locations.
pub fn resolve_git() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join("git.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let mut candidates = vec![];
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("Git/cmd/git.exe"));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Programs/Git/cmd/git.exe"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

/// Read a read-only snapshot for `workspace`. Never writes to the repository.
pub fn load(workspace: &Path, cancel: &AtomicBool) -> Result<Snapshot> {
    load_with_tags(workspace, cancel, None)
}

fn load_with_tags(
    workspace: &Path,
    cancel: &AtomicBool,
    cached_tags: Option<Vec<String>>,
) -> Result<Snapshot> {
    if !workspace.is_dir() {
        bail!("工作目录不存在：{}", workspace.display());
    }
    let git = resolve_git()
        .ok_or_else(|| anyhow!("未找到 git.exe；请安装 Git 或将其加入 PATH 后重试"))?;
    let inside = match run_git(
        &git,
        workspace,
        &["rev-parse", "--is-inside-work-tree"],
        cancel,
    ) {
        Ok(output) => output,
        Err(error) => {
            let text = format!("{error:#}");
            if text.contains("超时") || text.contains("取消") || text.contains("未找到") {
                return Err(error);
            }
            bail!("该工作区不是 Git 仓库");
        }
    };
    if inside.trim() != "true" {
        bail!("该工作区不是 Git 仓库");
    }
    let branch = run_git(
        &git,
        workspace,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancel,
    )
    .or_else(|_| {
        run_git(&git, workspace, &["rev-parse", "--short", "HEAD"], cancel)
            .map(|hash| format!("HEAD · {}", hash.trim()))
    })
    .map(|output| output.trim().to_string())
    .ok()
    .filter(|branch| !branch.is_empty())
    .unwrap_or_else(|| "（尚无提交）".into());

    let target = commit_target(&git, workspace, cancel)?;

    let status_output = run_git(
        &git,
        workspace,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ],
        cancel,
    )?;
    let mut files = parse_status(status_output.as_bytes());
    let mut notes = vec![];
    if status_output.len() >= MAX_COMMAND_BYTES {
        notes.push("Git 状态输出过大，文件列表可能不完整".into());
    }
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        notes.push(format!("变更文件过多，仅显示前 {MAX_FILES} 项"));
    }

    let staged_diff = run_git(
        &git,
        workspace,
        &[
            "--no-optional-locks",
            "--no-pager",
            "-c",
            "core.quotepath=false",
            "diff",
            "--cached",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
        ],
        cancel,
    )?;
    let staged_diff = truncate_diff(staged_diff, &mut notes);
    if files.iter().any(|file| file.untracked()) {
        notes.push("未跟踪文件按新增文件展示".into());
    }
    if commit_target(&git, workspace, cancel)? != target {
        bail!("读取期间分支或暂存区发生变化，请刷新后重试");
    }
    let tags = match cached_tags {
        Some(tags) => tags,
        None => list_tags(&git, workspace, cancel)?,
    };
    Ok(Snapshot {
        branch,
        tags,
        files,
        staged_diff,
        notes,
        target,
    })
}

fn list_tags(git: &Path, workspace: &Path, cancel: &AtomicBool) -> Result<Vec<String>> {
    let output = run_git(
        git,
        workspace,
        &["for-each-ref", "--format=%(refname:strip=2)", "refs/tags"],
        cancel,
    )?;
    if output.len() >= MAX_COMMAND_BYTES {
        bail!("标签列表过长，无法安全生成版本号");
    }
    let mut tags: Vec<String> = output.lines().map(str::to_owned).collect();
    sort_tags(&mut tags);
    Ok(tags)
}

pub fn sort_tags(tags: &mut [String]) {
    tags.sort_by(|a, b| match (stable_tag(a), stable_tag(b)) {
        (Some(a), Some(b)) => b.cmp(&a),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    });
}

fn stable_tag(tag: &str) -> Option<semver::Version> {
    let version = semver::Version::parse(tag.strip_prefix('v')?).ok()?;
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

pub fn next_tag(
    tags: &[String],
    increment: crate::store::TagIncrement,
    step: usize,
) -> Result<String> {
    let step = u64::try_from(step)
        .ok()
        .filter(|step| *step > 0)
        .ok_or_else(|| anyhow!("步长必须为正整数"))?;
    use crate::store::TagIncrement;
    let Some(mut version) = tags.iter().filter_map(|tag| stable_tag(tag)).max() else {
        return Ok("v0.1.0".into());
    };
    loop {
        match increment {
            TagIncrement::Patch => {
                version.patch = version
                    .patch
                    .checked_add(step)
                    .ok_or_else(|| anyhow!("版本号超出范围"))?
            }
            TagIncrement::Minor => {
                version.minor = version
                    .minor
                    .checked_add(step)
                    .ok_or_else(|| anyhow!("版本号超出范围"))?;
                version.patch = 0;
            }
            TagIncrement::Major => {
                version.major = version
                    .major
                    .checked_add(step)
                    .ok_or_else(|| anyhow!("版本号超出范围"))?;
                version.minor = 0;
                version.patch = 0;
            }
        }
        let candidate = format!("v{version}");
        if !tags.contains(&candidate) {
            return Ok(candidate);
        }
    }
}

fn validate_tag(git: &Path, workspace: &Path, tag: &str, cancel: &AtomicBool) -> Result<()> {
    if tag.is_empty() || tag.trim() != tag || tag.starts_with('-') {
        bail!("标签名称无效");
    }
    let reference = format!("refs/tags/{tag}");
    run_git(git, workspace, &["check-ref-format", &reference], cancel)
        .map_err(|_| anyhow!("标签名称无效：{tag}"))?;
    if run_git(
        git,
        workspace,
        &["show-ref", "--verify", "--quiet", &reference],
        cancel,
    )
    .is_ok()
    {
        bail!("标签 {tag} 已存在，请选择新的标签");
    }
    Ok(())
}

// Check only the actual Conduit release workflow in the staged tree, not arbitrary repositories.
fn validate_conduit_release(
    git: &Path,
    workspace: &Path,
    tag: &str,
    tree: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let workflow_path = format!("{tree}:.github/workflows/release.yml");
    let workflow = match run_git(git, workspace, &["show", &workflow_path], cancel) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    if !tag.starts_with('v')
        || !workflow.contains("Leo-Lee-j/conduit")
        || !workflow.contains("Tag must match Cargo.toml")
    {
        return Ok(());
    }
    let version =
        stable_tag(tag).ok_or_else(|| anyhow!("Conduit 发布标签必须采用稳定版 vX.Y.Z 格式"))?;
    let cargo = run_git(
        git,
        workspace,
        &["show", &format!("{tree}:Cargo.toml")],
        cancel,
    )?;
    let value: toml::Value = toml::from_str(&cargo).context("无法解析暂存区 Cargo.toml")?;
    let package = value
        .get("package")
        .and_then(|p| p.as_table())
        .ok_or_else(|| anyhow!("暂存区 Cargo.toml 缺少 package"))?;
    if package.get("name").and_then(|x| x.as_str()) != Some("conduit") {
        return Ok(());
    }
    let actual = package
        .get("version")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("暂存区 Cargo.toml 缺少版本号"))?;
    if version.to_string() != actual {
        bail!("发布标签 {tag} 与暂存区 Cargo.toml 版本 {actual} 不一致，请先更新并暂存版本号");
    }
    Ok(())
}

/// Read the patch for one changed file. Tracked files are compared against
/// HEAD, so the result includes both staged and unstaged edits. Untracked and
/// still-uncommitted added files are shown as a new file against an empty side.
pub fn load_file_diff(
    workspace: &Path,
    file: &ChangedFile,
    cancel: &AtomicBool,
) -> Result<FileDiff> {
    load_file_diff_scoped(workspace, file, FileDiffScope::All, cancel)
}

pub fn load_file_diff_scoped(
    workspace: &Path,
    file: &ChangedFile,
    scope: FileDiffScope,
    cancel: &AtomicBool,
) -> Result<FileDiff> {
    if !workspace.is_dir() {
        bail!("工作目录不存在：{}", workspace.display());
    }
    let git = resolve_git()
        .ok_or_else(|| anyhow!("未找到 git.exe；请安装 Git 或将其加入 PATH 后重试"))?;
    let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)
        .context("该工作区不是 Git 仓库")?;
    let root = PathBuf::from(root.trim());
    let mut notes = vec![];
    let patch = if file.untracked() && scope == FileDiffScope::Staged {
        String::new()
    } else if file.untracked() {
        let content = read_file_content(&root, &file.path)?;
        if content.truncated {
            notes.push(format!(
                "文件超过 {} MiB，仅显示前部分内容",
                MAX_FILE_DIFF_BYTES / (1024 * 1024)
            ));
        }
        added_file_patch(&file.path, content.text.as_deref())
    } else if scope == FileDiffScope::All
        && run_git(&git, &root, &["rev-parse", "--verify", "HEAD"], cancel).is_err()
    {
        if file.kind() != ChangeKind::Added {
            bail!("仓库尚无提交，无法对比该文件");
        }
        notes.push("仓库尚无提交，按新增文件展示".into());
        let content = read_file_content(&root, &file.path)?;
        if content.truncated {
            notes.push(format!(
                "文件超过 {} MiB，仅显示前部分内容",
                MAX_FILE_DIFF_BYTES / (1024 * 1024)
            ));
        }
        added_file_patch(&file.path, content.text.as_deref())
    } else {
        let context = format!("--unified={FULL_FILE_CONTEXT_LINES}");
        let mut args = vec![
            "--no-optional-locks",
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            &context,
        ];
        match scope {
            FileDiffScope::Staged => args.push("--cached"),
            FileDiffScope::Unstaged => {}
            FileDiffScope::All => args.push("HEAD"),
        }
        args.extend(["--", &file.path]);
        if let Some(original) = &file.original {
            args.push(original);
        }
        let output = run_git_bounded(
            &git,
            &root,
            &args,
            cancel,
            MAX_FILE_DIFF_BYTES,
            COMMAND_TIMEOUT,
            true,
        )?;
        if output.truncated {
            bail!(
                "文件差异超过 {} MiB，无法在此窗口完整显示",
                MAX_FILE_DIFF_BYTES / (1024 * 1024)
            );
        }
        output.text
    };
    Ok(FileDiff { patch, notes })
}

struct FileContent {
    text: Option<String>,
    truncated: bool,
}

fn read_file_content(root: &Path, path: &str) -> Result<FileContent> {
    if path.chars().any(char::is_control) {
        bail!("Git 文件路径包含不支持的控制字符");
    }
    let relative = Path::new(path);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("Git 文件路径超出仓库范围");
    }
    let canonical_root = fs::canonicalize(root)?;
    let absolute =
        fs::canonicalize(root.join(relative)).with_context(|| format!("无法读取文件：{path}"))?;
    if !absolute.starts_with(&canonical_root) {
        bail!("Git 文件路径超出仓库范围");
    }
    let mut file = File::open(&absolute)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_FILE_DIFF_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MAX_FILE_DIFF_BYTES;
    if truncated {
        bytes.truncate(MAX_FILE_DIFF_BYTES);
    }
    Ok(FileContent {
        text: decode_text(&bytes, truncated),
        truncated,
    })
}

fn decode_text(bytes: &[u8], truncated: bool) -> Option<String> {
    if bytes[..bytes.len().min(8 * 1024)].contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        // A read cut through the last glyph is still text; a genuinely invalid
        // sequence is treated as binary, matching Git's own preview behavior.
        Err(error) if truncated && error.error_len().is_none() => {
            Some(String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned())
        }
        Err(_) => None,
    }
}

fn added_file_patch(path: &str, text: Option<&str>) -> String {
    let mut patch = format!("--- /dev/null\n+++ b/{path}\n");
    let Some(text) = text else {
        patch.push_str(&format!("Binary files /dev/null and b/{path} differ\n"));
        return patch;
    };
    if text.is_empty() {
        return patch;
    }
    let final_newline = text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    if final_newline {
        lines.pop();
    }
    patch.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
    for (index, line) in lines.iter().enumerate() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
        if index + 1 == lines.len() && !final_newline {
            patch.push_str("\\ No newline at end of file\n");
        }
    }
    patch
}

fn commit_target(git: &Path, workspace: &Path, cancel: &AtomicBool) -> Result<CommitTarget> {
    let branch = run_git(
        git,
        workspace,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancel,
    )
    .ok()
    .map(|value| value.trim().to_string());
    let head = run_git(git, workspace, &["rev-parse", "--verify", "HEAD"], cancel)
        .ok()
        .map(|value| value.trim().to_string());
    let staged = run_git(
        git,
        workspace,
        &[
            "--no-optional-locks",
            "diff",
            "--cached",
            "--raw",
            "--no-abbrev",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
        ],
        cancel,
    )?;
    if staged.len() >= MAX_COMMAND_BYTES {
        bail!("暂存文件列表超过限制，请分批提交");
    }
    Ok(CommitTarget {
        branch,
        head,
        staged,
    })
}

#[derive(Clone, Copy)]
enum IndexAction {
    Stage,
    StageTracked,
    Unstage,
}

fn update_index(
    git: &Path,
    workspace: &Path,
    files: &[String],
    target: &CommitTarget,
    action: IndexAction,
    cancel: &AtomicBool,
) -> Result<()> {
    if files.is_empty() {
        bail!("请选择需要操作的文件");
    }
    if commit_target(git, workspace, cancel)? != *target {
        bail!("分支、HEAD 或暂存区已变化，请刷新后重试");
    }
    for path in files {
        if path.is_empty()
            || path.contains('\0')
            || path.contains(':')
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || path
                .split(['/', '\\'])
                .any(|c| c == ".." || c == "." || c.is_empty())
        {
            bail!("暂存路径必须是仓库内的文件：{path}");
        }
    }
    let root = run_git(git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let spec = std::env::temp_dir().join(format!(
        "conduit-index-{}-{}.paths",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&spec)?;
        for path in files {
            file.write_all(path.as_bytes())?;
            file.write_all(&[0])?;
        }
        drop(file);
        let from = format!("--pathspec-from-file={}", spec.display());
        let mut args = vec!["--literal-pathspecs"];
        match action {
            IndexAction::Stage => args.push("add"),
            IndexAction::StageTracked => args.extend(["add", "--update"]),
            IndexAction::Unstage => args.extend(["restore", "--staged"]),
        }
        args.extend([from.as_str(), "--pathspec-file-nul"]);
        run_git_with_timeout(
            git,
            Path::new(root.trim()),
            &args,
            cancel,
            Duration::from_secs(120),
        )?;
        Ok(())
    })();
    let _ = std::fs::remove_file(spec);
    result
}

fn operate(workspace: &Path, operation: Operation, cancel: &AtomicBool) -> OperationResult {
    if let Operation::Branch(request) = operation {
        return branches::operate_branch(workspace, *request, cancel);
    }
    let mut committed = false;
    let mut retry_tag = None;
    let mut retry_completed = false;
    let mut failure_stage = "提交已成功，推送失败";
    let result = (|| -> Result<String> {
        let git = resolve_git().ok_or_else(|| anyhow!("未找到 git.exe"))?;
        if !matches!(
            &operation,
            Operation::Stage { .. } | Operation::Unstage { .. }
        ) && let Some(state) = branches::integration_state(&git, workspace, cancel)?
        {
            bail!("仓库存在进行中的 {}，请先继续或中止", state.label());
        }
        match operation {
            Operation::Branch(_) => unreachable!(),
            Operation::Discard { file, target } => {
                discard::discard_file(&git, workspace, &file, &target, cancel)?;
                Ok(format!("已回滚 {} 的修改", file.path))
            }
            Operation::Stage { files, target } => {
                update_index(&git, workspace, &files, &target, IndexAction::Stage, cancel)?;
                Ok(format!("已暂存 {} 个文件", files.len()))
            }
            Operation::StageTracked { files, target } => {
                let conflicts = run_git(
                    &git,
                    workspace,
                    &["diff", "--name-only", "--diff-filter=U", "-z"],
                    cancel,
                )?;
                if !conflicts.is_empty() {
                    bail!("存在未解决的冲突，已暂停自动暂存");
                }
                update_index(
                    &git,
                    workspace,
                    &files,
                    &target,
                    IndexAction::StageTracked,
                    cancel,
                )?;
                Ok(format!("已自动暂存 {} 个已跟踪文件", files.len()))
            }
            Operation::Unstage { files, target } => {
                update_index(
                    &git,
                    workspace,
                    &files,
                    &target,
                    IndexAction::Unstage,
                    cancel,
                )?;
                Ok(format!("已将 {} 个文件移出暂存区", files.len()))
            }
            Operation::Commit {
                files,
                message,
                target,
                push,
                tag,
            } => {
                if message.trim().is_empty() {
                    bail!("请填写提交说明");
                }
                if message.len() > 16 * 1024 {
                    bail!("提交说明最多 16 KiB");
                }
                if target.branch.is_none() {
                    bail!("分离 HEAD 状态下不能从此面板提交，请先切回分支");
                }
                if target.staged.is_empty() {
                    bail!("没有已暂存变更，请先在 Git 中暂存文件再刷新");
                }
                if commit_target(&git, workspace, cancel)? != target {
                    bail!("分支、HEAD 或暂存区已变化，请刷新变更后重新提交");
                }
                if let Some(tag) = tag.as_deref() {
                    validate_tag(&git, workspace, tag, cancel)?;
                    if push && files.is_none() {
                        validate_conduit_release(&git, workspace, tag, "", cancel)?;
                    }
                }
                // An alternate index limits the commit without touching working files
                // or removing unchecked changes from the real index.
                if let Some(files) = files {
                    selected::commit(
                        &git,
                        workspace,
                        &target,
                        &files,
                        message.trim(),
                        if push { tag.as_deref() } else { None },
                        cancel,
                    )?;
                } else {
                    run_git_with_timeout(
                        &git,
                        workspace,
                        &["commit", "-m", message.trim()],
                        cancel,
                        Duration::from_secs(120),
                    )?;
                }
                committed = true;
                failure_stage = "提交已成功，无法确认提交对象";
                let oid = run_git(&git, workspace, &["rev-parse", "--verify", "HEAD"], cancel)?;
                if let Some(tag) = tag.as_deref() {
                    if push {
                        failure_stage = "提交已成功，发布版本校验失败";
                        validate_conduit_release(&git, workspace, tag, oid.trim(), cancel)?;
                    }
                    failure_stage = "提交已成功，创建标签失败";
                    run_git(
                        &git,
                        workspace,
                        &["-c", "tag.gpgSign=false", "tag", "--", tag, oid.trim()],
                        cancel,
                    )?;
                }
                failure_stage = if tag.is_some() {
                    "提交和本地标签已创建，推送失败"
                } else {
                    "提交已成功，推送失败"
                };
                if push {
                    let destination = resolve_push_destination(
                        &git,
                        workspace,
                        target.branch.as_deref().unwrap(),
                        cancel,
                    );
                    if let Some(tag) = tag.as_deref() {
                        let mut pending = TagRetry {
                            branch: target.branch.clone().unwrap(),
                            remote: String::new(),
                            remote_url: String::new(),
                            merge: String::new(),
                            tag: tag.into(),
                            oid: oid.trim().into(),
                        };
                        if let Ok(destination) = &destination
                            && let Ok(url) =
                                single_push_url(&git, workspace, &destination.remote, cancel)
                        {
                            pending.remote = destination.remote.clone();
                            pending.remote_url = url;
                            pending.merge = destination.merge.clone();
                        }
                        retry_tag = Some(pending);
                    }
                    push_branch(
                        &git,
                        workspace,
                        target.branch.as_deref().unwrap(),
                        &destination?,
                        tag.as_deref().map(|tag| (tag, oid.trim())),
                        cancel,
                    )?;
                    retry_tag = None;
                    Ok(if tag.is_some() {
                        "提交、标签及推送成功"
                    } else {
                        "提交并推送成功"
                    }
                    .into())
                } else {
                    Ok(if tag.is_some() {
                        "提交及本地标签创建成功"
                    } else {
                        "提交成功"
                    }
                    .into())
                }
            }
            Operation::RetryTag(retry) => {
                retry_tag = Some(retry.clone());
                retry_tag_push(&git, workspace, &retry, cancel)?;
                retry_tag = None;
                retry_completed = true;
                Ok("标签已推送（分支未在此操作中推送）".into())
            }
        }
    })();
    match result {
        Ok(message) => OperationResult {
            retry_completed,
            retry_tag: None,
            worktree_created: None,
            committed,
            message,
            error: None,
        },
        Err(error) => OperationResult {
            retry_completed,
            retry_tag,
            worktree_created: None,
            committed,
            message: if committed {
                if failure_stage.contains("推送失败")
                    && (format!("{error:#}").contains("超时")
                        || format!("{error:#}").contains("取消"))
                {
                    format!("{failure_stage}；远端状态可能未知，请检查远端后再重试")
                } else {
                    failure_stage.into()
                }
            } else {
                "Git 操作失败".into()
            },
            error: Some(format!("{error:#}")),
        },
    }
}

struct PushDestination {
    remote: String,
    merge: String,
    set_upstream: bool,
}

fn resolve_push_destination(
    git: &Path,
    workspace: &Path,
    branch: &str,
    cancel: &AtomicBool,
) -> Result<PushDestination> {
    let current = run_git(
        git,
        workspace,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancel,
    )?;
    if current.trim() != branch {
        bail!("当前分支已变化，请刷新后重试推送");
    }
    // A missing key has an empty default; command failures must still propagate.
    let config = |key: &str| -> Result<String> {
        run_git(
            git,
            workspace,
            &["config", "--default", "", "--get", key],
            cancel,
        )
        .map(|value| value.trim().to_owned())
    };
    let configured_remote = config(&format!("branch.{branch}.remote"))?;
    let merge = config(&format!("branch.{branch}.merge"))?;
    if !merge.is_empty() {
        if configured_remote.is_empty() || !merge.starts_with("refs/heads/") {
            bail!("上游配置不完整，请在分支菜单中选择 Push 并设置上游");
        }
        // The tracking ref may be missing locally; keep the configured destination.
        return Ok(PushDestination {
            remote: configured_remote,
            merge,
            set_upstream: false,
        });
    }
    let remotes = run_git(git, workspace, &["remote"], cancel)?;
    let remotes: Vec<_> = remotes
        .lines()
        .filter(|remote| !remote.is_empty())
        .collect();
    let mut remote = config(&format!("branch.{branch}.pushRemote"))?;
    if remote.is_empty() {
        remote = config("remote.pushDefault")?;
    }
    if remote.is_empty() {
        remote = configured_remote;
    }
    if remote.is_empty() {
        remote = if remotes.contains(&"origin") {
            "origin"
        } else if remotes.len() == 1 {
            remotes[0]
        } else if remotes.is_empty() {
            bail!("尚未配置远程仓库，无法推送新分支");
        } else {
            bail!("存在多个远程仓库且未指定默认推送目标，请在分支菜单中选择 Push 和目标远程");
        }
        .to_owned();
    }
    if remote != "." && !remotes.contains(&remote.as_str()) {
        bail!("推送远程 {remote} 不存在，请在分支菜单中选择 Push 和目标远程");
    }
    Ok(PushDestination {
        remote,
        merge: format!("refs/heads/{branch}"),
        set_upstream: true,
    })
}

fn resolve_upstream(
    git: &Path,
    workspace: &Path,
    branch: &str,
    expected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(String, String)> {
    let current = run_git(
        git,
        workspace,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancel,
    )?;
    if current.trim() != branch {
        bail!("当前分支已变化，请刷新后重试推送");
    }
    let upstream = run_git(
        git,
        workspace,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        cancel,
    )
    .map_err(|_| anyhow!("当前分支未设置有效上游，请先在分支菜单中选择 Push 并设置上游"))?;
    if expected.is_some_and(|expected| expected != upstream.trim()) {
        bail!("上游分支已变化，请刷新后重试推送");
    }
    // Resolve the configured upstream explicitly; never force or push unrelated refs.
    let remote = run_git(
        git,
        workspace,
        &["config", "--get", &format!("branch.{branch}.remote")],
        cancel,
    )?;
    let merge = run_git(
        git,
        workspace,
        &["config", "--get", &format!("branch.{branch}.merge")],
        cancel,
    )?;
    let remote = remote.trim().to_owned();
    let merge = merge.trim().to_owned();
    if remote.is_empty() || !merge.starts_with("refs/heads/") {
        bail!("上游配置不完整，请检查 Git 分支配置");
    }
    Ok((remote, merge))
}

fn single_push_url(
    git: &Path,
    workspace: &Path,
    remote: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let urls = run_git(
        git,
        workspace,
        &["remote", "get-url", "--push", "--all", remote],
        cancel,
    )?;
    let mut urls = urls.lines();
    let url = urls
        .next()
        .filter(|url| !url.is_empty())
        .ok_or_else(|| anyhow!("远程推送地址为空"))?;
    if urls.next().is_some() {
        bail!("远程配置了多个推送地址，无法保证标签和分支原子推送");
    }
    Ok(url.to_owned())
}

fn load_remote_tags(workspace: &Path, cancel: &AtomicBool) -> Result<RemoteTags> {
    let git = resolve_git().ok_or_else(|| anyhow!("未找到 git.exe"))?;
    let branch = run_git(
        &git,
        workspace,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        cancel,
    )
    .map(|branch| branch.trim().to_owned())
    .unwrap_or_else(|_| "HEAD".into());
    // Reading tags does not require a tracking branch (or even an attached HEAD).
    let configured = run_git(
        &git,
        workspace,
        &["config", "--get", &format!("branch.{branch}.remote")],
        cancel,
    )
    .ok();
    let remotes = run_git(&git, workspace, &["remote"], cancel)?;
    let remotes: Vec<_> = remotes.lines().collect();
    let remote = configured
        .as_deref()
        .map(str::trim)
        .filter(|remote| remotes.contains(remote))
        .or_else(|| remotes.iter().copied().find(|remote| *remote == "origin"))
        .or_else(|| (remotes.len() == 1).then(|| remotes[0]))
        .ok_or_else(|| anyhow!("未找到可查询的远程仓库，请配置 origin 或当前分支的远程"))?
        .to_owned();
    let push_url = single_push_url(&git, workspace, &remote, cancel)?;
    let output = run_git_with_timeout(
        &git,
        workspace,
        &[
            "-c",
            "credential.interactive=false",
            "ls-remote",
            "--refs",
            "--tags",
            "--",
            &push_url,
        ],
        cancel,
        Duration::from_secs(120),
    )?;
    if output.len() >= MAX_COMMAND_BYTES {
        bail!("远程标签列表过长，请缩小范围后重试");
    }
    let mut names = Vec::new();
    for line in output.lines() {
        let (_, reference) = line
            .split_once('\t')
            .ok_or_else(|| anyhow!("远程标签响应无效"))?;
        if let Some(name) = reference.strip_prefix("refs/tags/") {
            names.push(name.to_owned());
        }
    }
    names.sort();
    names.dedup();
    Ok(RemoteTags {
        branch,
        remote,
        names,
    })
}

fn retry_tag_push(
    git: &Path,
    workspace: &Path,
    retry: &TagRetry,
    cancel: &AtomicBool,
) -> Result<()> {
    let (remote, merge) = resolve_upstream(git, workspace, &retry.branch, None, cancel)?;
    let url = single_push_url(git, workspace, &remote, cancel)?;
    if !retry.remote.is_empty()
        && (remote != retry.remote || merge != retry.merge || url != retry.remote_url)
    {
        bail!("上游或远程地址已变化，请检查后重新操作");
    }
    let head = run_git(git, workspace, &["rev-parse", "--verify", "HEAD"], cancel)?;
    if head.trim() != retry.oid {
        bail!("HEAD 已变化，请检查后重新操作");
    }
    let reference = format!("refs/tags/{}", retry.tag);
    let local = run_git(
        git,
        workspace,
        &["rev-parse", "--verify", &reference],
        cancel,
    )?;
    if local.trim() != retry.oid {
        bail!("本地标签指向已变化，请检查后重新操作");
    }
    // Query the push endpoint: a remote may have a separate fetch URL.
    let output = run_git_with_timeout(
        git,
        workspace,
        &[
            "-c",
            "credential.interactive=false",
            "ls-remote",
            "--refs",
            "--",
            &url,
            &reference,
            &merge,
        ],
        cancel,
        Duration::from_secs(120),
    )?;
    if output.len() >= MAX_COMMAND_BYTES {
        bail!("远程引用结果过大");
    }
    let mut branch_oid = None;
    for line in output.lines() {
        let (oid, name) = line
            .split_once('\t')
            .ok_or_else(|| anyhow!("远程引用响应无效"))?;
        if name == reference {
            if oid == retry.oid {
                return Ok(());
            }
            bail!("远程已存在指向其他提交的同名标签");
        }
        if name == merge {
            branch_oid = Some(oid);
        }
    }
    let remote_head = branch_oid.ok_or_else(|| anyhow!("远程上游分支不存在，请先推送分支"))?;
    if remote_head != retry.oid
        && run_git(
            git,
            workspace,
            &["merge-base", "--is-ancestor", &retry.oid, remote_head],
            cancel,
        )
        .is_err()
    {
        bail!("远程上游分支尚未包含该提交，或本地缺少远程提交；请先推送或刷新分支");
    }
    let refspec = format!("{}:{}", retry.oid, reference);
    run_git_with_timeout(
        git,
        workspace,
        &[
            "-c",
            "credential.interactive=false",
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--",
            &remote,
            &refspec,
        ],
        cancel,
        Duration::from_secs(120),
    )?;
    Ok(())
}

fn push_branch(
    git: &Path,
    workspace: &Path,
    branch: &str,
    destination: &PushDestination,
    tag: Option<(&str, &str)>,
    cancel: &AtomicBool,
) -> Result<()> {
    let PushDestination {
        remote,
        merge,
        set_upstream,
    } = destination;
    if let Some((_, oid)) = tag {
        single_push_url(git, workspace, remote, cancel)?;
        let head = run_git(git, workspace, &["rev-parse", "--verify", "HEAD"], cancel)?;
        if head.trim() != oid {
            bail!("提交后 HEAD 已变化，请检查分支和标签再推送");
        }
    }
    let local_ref = format!("refs/heads/{branch}");
    let branch_ref = format!("{}:{merge}", tag.map_or(local_ref.as_str(), |(_, oid)| oid));
    let tag_ref = tag.map(|(name, oid)| format!("{oid}:refs/tags/{name}"));
    let mut args = vec![
        "-c",
        "credential.interactive=false",
        "push",
        "--porcelain",
        "--no-follow-tags",
    ];
    if tag_ref.is_some() {
        args.push("--atomic");
    } else if *set_upstream {
        args.push("--set-upstream");
    }
    args.extend(["--", remote.as_str(), branch_ref.as_str()]);
    if let Some(ref tag_ref) = tag_ref {
        args.push(tag_ref);
    }
    run_git_with_timeout(git, workspace, &args, cancel, Duration::from_secs(120))?;
    if *set_upstream && tag.is_some() {
        // Tagged pushes pin both refs to the commit OID. Git cannot infer a local
        // branch from that source, so record tracking only after atomic success.
        for (key, value) in [("remote", remote), ("merge", merge)] {
            run_git(
                git,
                workspace,
                &[
                    "config",
                    "--local",
                    &format!("branch.{branch}.{key}"),
                    value,
                ],
                cancel,
            )
            .context("分支和标签已推送，但保存上游配置失败")?;
        }
    }
    Ok(())
}

/// Parse `git status --porcelain=v1 -z` output (NUL-separated records).
pub fn parse_status(bytes: &[u8]) -> Vec<ChangedFile> {
    let mut fields = bytes.split(|byte| *byte == 0);
    let mut files = vec![];
    while let Some(field) = fields.next() {
        // `XY<space>path`; skip malformed/empty trailing records.
        if field.len() <= 3 || field[2] != b' ' {
            continue;
        }
        let index = field[0] as char;
        let worktree = field[1] as char;
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        let original = if matches!(index, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            fields
                .next()
                .map(|value| String::from_utf8_lossy(value).into_owned())
        } else {
            None
        };
        files.push(ChangedFile {
            index,
            worktree,
            path,
            original,
        });
    }
    files
}

fn truncate_diff(diff: String, notes: &mut Vec<String>) -> String {
    if diff.len() <= MAX_DIFF_BYTES {
        return diff;
    }
    // Keep the prefix on a UTF-8 boundary so prompts never cut a glyph.
    let mut end = MAX_DIFF_BYTES;
    while end > 0 && !diff.is_char_boundary(end) {
        end -= 1;
    }
    notes.push(format!(
        "暂存 diff 已截断，仅保留前 {} KiB",
        MAX_DIFF_BYTES / 1024
    ));
    let mut truncated = diff[..end].to_string();
    truncated.push_str("\n…（已截断）\n");
    truncated
}

fn read_available(
    file: &mut File,
    output: &mut Vec<u8>,
    truncated: &mut bool,
    limit: usize,
) -> Result<usize> {
    let Ok(available) = available(file) else {
        return Ok(0);
    };
    if available == 0 {
        return Ok(0);
    }
    let mut buffer = [0; READ_CHUNK];
    let count = file.read(&mut buffer[..(available as usize).min(READ_CHUNK)])?;
    let room = limit.saturating_sub(output.len());
    if room == 0 {
        *truncated = true;
    } else {
        let keep = count.min(room);
        output.extend_from_slice(&buffer[..keep]);
        if keep < count {
            *truncated = true;
        }
    }
    Ok(count)
}

/// Run one Git command in `workspace`, bounded in time and output size.
fn run_git(git: &Path, workspace: &Path, args: &[&str], cancel: &AtomicBool) -> Result<String> {
    run_git_with_timeout(git, workspace, args, cancel, COMMAND_TIMEOUT)
}

fn run_git_with_timeout(
    git: &Path,
    workspace: &Path,
    args: &[&str],
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<String> {
    run_git_bounded(
        git,
        workspace,
        args,
        cancel,
        MAX_COMMAND_BYTES,
        timeout,
        false,
    )
    .map(|output| output.text)
}

struct CommandOutput {
    text: String,
    truncated: bool,
}

fn run_git_bounded(
    git: &Path,
    workspace: &Path,
    args: &[&str],
    cancel: &AtomicBool,
    limit: usize,
    timeout: Duration,
    stop_when_full: bool,
) -> Result<CommandOutput> {
    run_git_with_environment(
        git,
        workspace,
        args,
        cancel,
        None,
        CommandLimits {
            limit,
            timeout,
            stop_when_full,
        },
    )
}

struct CommandLimits {
    limit: usize,
    timeout: Duration,
    stop_when_full: bool,
}

fn run_git_with_environment(
    git: &Path,
    workspace: &Path,
    args: &[&str],
    cancel: &AtomicBool,
    index: Option<&Path>,
    limits: CommandLimits,
) -> Result<CommandOutput> {
    let CommandLimits {
        limit,
        timeout,
        stop_when_full,
    } = limits;
    if cancel.load(Ordering::Acquire) {
        bail!("Git 读取已取消");
    }
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let mut environment = std::collections::BTreeMap::from([
        ("GIT_EDITOR".into(), "true".into()),
        ("GIT_SEQUENCE_EDITOR".into(), "true".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GCM_INTERACTIVE".into(), "Never".into()),
    ]);
    if let Some(index) = index {
        environment.insert(
            "GIT_INDEX_FILE".into(),
            index.to_string_lossy().into_owned(),
        );
    }
    let (child, mut pipes) = Child::spawn_with_env(git, &args, workspace, &environment)
        .with_context(|| format!("启动 {} 失败", git.display()))?;
    drop(pipes.stdin);
    let result = (|| {
        let started = Instant::now();
        let mut stdout = vec![];
        let mut stderr = vec![];
        let mut truncated = false;
        loop {
            if cancel.load(Ordering::Acquire) {
                bail!("Git 读取已取消");
            }
            if started.elapsed() > timeout {
                bail!("Git 命令超时，已停止进程树");
            }
            let mut progressed = false;
            progressed |=
                read_available(&mut pipes.stdout, &mut stdout, &mut truncated, limit)? > 0;
            progressed |=
                read_available(&mut pipes.stderr, &mut stderr, &mut truncated, limit)? > 0;
            if stop_when_full && truncated {
                return Ok(CommandOutput {
                    text: String::from_utf8_lossy(&stdout).into_owned(),
                    truncated,
                });
            }
            if let Some(code) = child.exit_code()? {
                while read_available(&mut pipes.stdout, &mut stdout, &mut truncated, limit)? > 0 {}
                while read_available(&mut pipes.stderr, &mut stderr, &mut truncated, limit)? > 0 {}
                // --no-index uses exit code 1 to report a successful comparison
                // with differences, like diff(1).
                if code != 0
                    && !(code == 1
                        && args
                            .windows(2)
                            .any(|pair| pair[0] == "diff" && pair[1] == "--no-index"))
                {
                    let stderr = String::from_utf8_lossy(&stderr);
                    let stdout = String::from_utf8_lossy(&stdout);
                    bail!(
                        "Git 命令失败（退出码 {code}）：{}\n{}",
                        stderr.trim(),
                        stdout.trim()
                    );
                }
                return Ok(CommandOutput {
                    text: String::from_utf8_lossy(&stdout).into_owned(),
                    truncated,
                });
            }
            if !progressed {
                thread::sleep(Duration::from_millis(10));
            }
        }
    })();
    child.terminate()?;
    while child.active_processes()? > 0 || child.exit_code()?.is_none() {
        thread::sleep(Duration::from_millis(10));
    }
    result
}

#[cfg(test)]
mod tests {
    include!("git/branch_tests.rs");
    include!("git/push_tests.rs");
    use super::*;

    #[test]
    fn worktree_scope_groups_subdirectories_and_separates_linked_worktrees() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("conduit-worktree-scope-{unique}"));
        let repo = root.join("repository");
        let child = repo.join("src/nested");
        let linked = repo.join("linked");
        let separate = root.join("separate");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&child).unwrap();
        std::fs::create_dir_all(linked.join("src")).unwrap();
        std::fs::write(linked.join(".git"), "gitdir: ../.git/worktrees/linked\n").unwrap();
        std::fs::create_dir_all(&separate).unwrap();
        assert_eq!(worktree_root(&repo), repo);
        assert_eq!(worktree_root(&child), repo);
        assert_eq!(worktree_root(&linked.join("src")), linked);
        assert_eq!(worktree_root(&separate), separate);
        assert_ne!(worktree_root(&linked), worktree_root(&child));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn index_operations_treat_selected_paths_literally() {
        let repo = TestRepository::new().expect("Git required for index regression");
        for name in ["file[1].txt", "file1.txt"] {
            std::fs::write(repo.path.join(name), "before\n").unwrap();
        }
        repo.run(&["add", "--all"]);
        repo.run(&["commit", "-qm", "fixture"]);
        for name in ["file[1].txt", "file1.txt"] {
            std::fs::write(repo.path.join(name), "after\n").unwrap();
        }
        let cancel = AtomicBool::new(false);
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let staged = operate(
            &repo.path,
            Operation::Stage {
                files: vec!["file[1].txt".into()],
                target,
            },
            &cancel,
        );
        assert!(staged.error.is_none(), "{:?}", staged.error);
        assert_eq!(
            repo.run(&["diff", "--cached", "--name-only"]).trim(),
            "file[1].txt"
        );
        repo.run(&["add", "--all"]);
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let unstaged = operate(
            &repo.path,
            Operation::Unstage {
                files: vec!["file[1].txt".into()],
                target,
            },
            &cancel,
        );
        assert!(unstaged.error.is_none(), "{:?}", unstaged.error);
        assert_eq!(
            repo.run(&["diff", "--cached", "--name-only"]).trim(),
            "file1.txt"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
            "after\n"
        );
    }

    fn commit_selection(repo: &TestRepository, files: &[&str]) -> OperationResult {
        let cancel = AtomicBool::new(false);
        operate(
            &repo.path,
            Operation::Commit {
                files: Some(files.iter().map(|s| s.to_string()).collect()),
                message: "selected changes".into(),
                target: commit_target(&repo.git, &repo.path, &cancel).unwrap(),
                push: false,
                tag: None,
            },
            &cancel,
        )
    }

    #[test]
    fn selected_commit_preserves_unchecked_index_and_unstaged_content() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        assert!(repo.commit("initial", false).committed);
        repo.stage("staged version\n");
        std::fs::write(repo.path.join("tracked.txt"), "unstaged version\n").unwrap();
        std::fs::write(repo.path.join("other.txt"), "unchecked\n").unwrap();
        repo.run(&["add", "other.txt"]);
        let index = std::fs::read(repo.path.join(".git/index")).unwrap();
        let result = commit_selection(&repo, &["tracked.txt"]);
        assert!(result.committed, "{:?}", result.error);
        assert_eq!(repo.run(&["show", "HEAD:tracked.txt"]), "staged version\n");
        assert_eq!(
            repo.run(&["diff", "--cached", "--name-only"]).trim(),
            "other.txt"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "unstaged version\n"
        );
        assert_eq!(std::fs::read(repo.path.join(".git/index")).unwrap(), index);
        assert!(!repo.path.join(".git/index.lock").exists());
    }

    #[test]
    fn selected_release_validates_the_selected_tree_before_creating_commit() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        std::fs::create_dir_all(repo.path.join(".github/workflows")).unwrap();
        std::fs::write(
            repo.path.join(".github/workflows/release.yml"),
            "Leo-Lee-j/conduit\nTag must match Cargo.toml\n",
        )
        .unwrap();
        std::fs::write(
            repo.path.join("Cargo.toml"),
            "[package]\nname = \"conduit\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        repo.stage("initial\n");
        repo.run(&["add", "."]);
        assert!(repo.commit("initial", false).committed);
        let head = repo.run(&["rev-parse", "HEAD"]);
        std::fs::write(
            repo.path.join("Cargo.toml"),
            "[package]\nname = \"conduit\"\nversion = \"2.0.0\"\n",
        )
        .unwrap();
        repo.stage("selected\n");
        repo.run(&["add", "Cargo.toml"]);
        let cancel = AtomicBool::new(false);
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: Some(vec!["tracked.txt".into()]),
                message: "release".into(),
                target: commit_target(&repo.git, &repo.path, &cancel).unwrap(),
                push: true,
                tag: Some("v2.0.0".into()),
            },
            &cancel,
        );
        assert!(!result.committed);
        assert!(result.error.unwrap().contains("不一致"));
        assert_eq!(repo.run(&["rev-parse", "HEAD"]), head);
        assert!(!repo.path.join(".git/index.lock").exists());
    }

    #[test]
    fn selected_initial_commit_and_literal_paths() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        for name in ["file[1].txt", "file1.txt", "中文 空格.txt"] {
            std::fs::write(repo.path.join(name), name).unwrap();
        }
        repo.run(&["add", "."]);
        let result = commit_selection(&repo, &["file[1].txt", "中文 空格.txt"]);
        assert!(result.committed, "{:?}", result.error);
        assert_eq!(
            repo.run(&["diff", "--cached", "--name-only"]).trim(),
            "file1.txt"
        );
        assert_eq!(repo.run(&["show", "HEAD:file[1].txt"]), "file[1].txt");
        assert_eq!(repo.run(&["show", "HEAD:中文 空格.txt"]), "中文 空格.txt");
    }

    #[test]
    fn selected_commit_renames_deletes_and_failure_keep_index() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("rename me\n");
        std::fs::write(repo.path.join("delete.txt"), "delete me\n").unwrap();
        repo.run(&["add", "."]);
        assert!(repo.commit("initial", false).committed);
        repo.run(&["mv", "tracked.txt", "renamed.txt"]);
        repo.run(&["rm", "delete.txt"]);
        std::fs::write(repo.path.join("other.txt"), "unchecked\n").unwrap();
        repo.run(&["add", "other.txt"]);
        let index = std::fs::read(repo.path.join(".git/index")).unwrap();
        assert!(!commit_selection(&repo, &[]).committed);
        assert!(!commit_selection(&repo, &["missing.txt"]).committed);
        std::fs::write(
            repo.path.join("empty-hooks/pre-commit"),
            "#!/bin/sh\nexit 1\n",
        )
        .unwrap();
        let failed = commit_selection(&repo, &["tracked.txt", "renamed.txt", "delete.txt"]);
        assert!(!failed.committed);
        assert_eq!(std::fs::read(repo.path.join(".git/index")).unwrap(), index);
        assert!(!repo.path.join(".git/index.lock").exists());
        std::fs::remove_file(repo.path.join("empty-hooks/pre-commit")).unwrap();
        let result = commit_selection(&repo, &["tracked.txt", "renamed.txt", "delete.txt"]);
        assert!(result.committed, "{:?}", result.error);
        assert_eq!(
            repo.run(&["ls-tree", "--name-only", "HEAD"]).trim(),
            "renamed.txt"
        );
        assert_eq!(
            repo.run(&["diff", "--cached", "--name-only"]).trim(),
            "other.txt"
        );
    }

    #[test]
    fn selected_generation_patch_uses_exact_names_and_handles_git_quoting() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        for name in ["file[1].txt", "file1.txt", "中文 空格.txt"] {
            std::fs::write(repo.path.join(name), format!("{name} content\n")).unwrap();
        }
        repo.run(&["add", "."]);
        for quote in ["true", "false"] {
            repo.run(&["config", "core.quotePath", quote]);
            let snapshot = load(&repo.path, &AtomicBool::new(false)).unwrap();
            let patch =
                selected_staged_patch(&snapshot, &["file[1].txt".into(), "中文 空格.txt".into()]);
            assert!(patch.contains("+file[1].txt content"), "{patch}");
            assert!(patch.contains("+中文 空格.txt content"), "{patch}");
            assert!(!patch.contains("+file1.txt content"), "{patch}");
            assert!(selected_staged_patch(&snapshot, &[]).is_empty());
        }
    }

    struct TestRepository {
        path: PathBuf,
        git: PathBuf,
    }
    impl TestRepository {
        fn new() -> Option<Self> {
            let git = resolve_git()?;
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("conduit-commit-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(path.join("empty-hooks")).unwrap();
            let repository = Self { path, git };
            repository.run(&["init", "-q", "--initial-branch=main"]);
            repository.run(&["config", "user.name", "Conduit Test"]);
            repository.run(&["config", "user.email", "conduit@example.invalid"]);
            repository.run(&["config", "commit.gpgSign", "false"]);
            repository.run(&[
                "config",
                "core.hooksPath",
                repository.path.join("empty-hooks").to_str().unwrap(),
            ]);
            Some(repository)
        }
        fn run(&self, args: &[&str]) -> String {
            let output = std::process::Command::new(&self.git)
                .args(args)
                .current_dir(&self.path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        }
        fn stage(&self, content: &str) {
            std::fs::write(self.path.join("tracked.txt"), content).unwrap();
            self.run(&["add", "tracked.txt"]);
        }
        fn commit(&self, message: &str, push: bool) -> OperationResult {
            let target = commit_target(&self.git, &self.path, &AtomicBool::new(false)).unwrap();
            operate(
                &self.path,
                Operation::Commit {
                    files: None,
                    message: message.into(),
                    target,
                    push,
                    tag: None,
                },
                &AtomicBool::new(false),
            )
        }
        fn metadata(&self) -> Repository {
            load_repository(&self.path, &AtomicBool::new(false)).unwrap()
        }
        fn seed_remote(&self) -> PathBuf {
            let remote = self.path.join("remote.git");
            self.run(&[
                "init",
                "--bare",
                "-q",
                "--initial-branch=main",
                remote.to_str().unwrap(),
            ]);
            self.run(&["remote", "add", "origin", remote.to_str().unwrap()]);
            self.run(&["push", "-u", "origin", "main"]);
            remote
        }
    }
    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    use std::time::{SystemTime, UNIX_EPOCH};

    fn discard_test_file(repo: &TestRepository, path: &str) -> OperationResult {
        let snapshot = load(&repo.path, &AtomicBool::new(false)).unwrap();
        let file = snapshot
            .files
            .iter()
            .find(|file| file.path == path)
            .unwrap()
            .clone();
        operate(
            &repo.path,
            Operation::Discard {
                file,
                target: snapshot.target,
            },
            &AtomicBool::new(false),
        )
    }

    #[test]
    fn discard_restores_only_the_selected_file_and_handles_renames_and_deletions() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.run(&["config", "core.autocrlf", "false"]);
        repo.stage("original\n");
        fs::write(repo.path.join("file[1].txt"), "literal original\n").unwrap();
        fs::write(repo.path.join("file1.txt"), "other original\n").unwrap();
        repo.run(&["add", "."]);
        assert!(repo.commit("seed", false).committed);
        repo.stage("staged edit\n");
        fs::write(repo.path.join("tracked.txt"), "unstaged edit\n").unwrap();
        fs::write(repo.path.join("file[1].txt"), "literal change\n").unwrap();
        fs::write(repo.path.join("file1.txt"), "keep this edit\n").unwrap();
        repo.run(&["add", "file1.txt"]);
        let result = discard_test_file(&repo, "file[1].txt");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(
            fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
            "literal original\n"
        );
        let result = discard_test_file(&repo, "tracked.txt");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(
            fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "original\n"
        );
        assert_eq!(repo.run(&["show", ":file1.txt"]), "keep this edit\n");
        assert_eq!(
            fs::read_to_string(repo.path.join("file1.txt")).unwrap(),
            "keep this edit\n"
        );
        repo.run(&["mv", "tracked.txt", "renamed.txt"]);
        fs::write(repo.path.join("tracked.txt"), "new file at the old path\n").unwrap();
        let result = discard_test_file(&repo, "renamed.txt");
        assert!(result.error.is_some());
        assert_eq!(
            fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "new file at the old path\n"
        );
        assert!(repo.path.join("renamed.txt").exists());
        fs::remove_file(repo.path.join("tracked.txt")).unwrap();
        let result = discard_test_file(&repo, "renamed.txt");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(repo.path.join("tracked.txt").exists());
        assert!(!repo.path.join("renamed.txt").exists());
        repo.run(&["rm", "tracked.txt"]);
        let result = discard_test_file(&repo, "tracked.txt");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(
            fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "original\n"
        );
        assert_eq!(
            repo.run(&["diff", "--cached", "--name-only"]).trim(),
            "file1.txt"
        );
    }

    #[test]
    fn discard_removes_only_selected_new_files_even_before_the_first_commit() {
        for initial_commit in [false, true] {
            let Some(repo) = TestRepository::new() else {
                return;
            };
            repo.stage("keep\n");
            if initial_commit {
                assert!(repo.commit("seed", false).committed);
            }
            for path in ["new [1].txt", "untracked.txt", "keep.txt"] {
                fs::write(repo.path.join(path), "new\n").unwrap();
            }
            repo.run(&["add", "--", "new [1].txt"]);
            for path in ["new [1].txt", "untracked.txt"] {
                let result = discard_test_file(&repo, path);
                assert!(result.error.is_none(), "{:?}", result.error);
                assert!(!repo.path.join(path).exists());
            }
            assert_eq!(
                fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
                "keep\n"
            );
            assert!(repo.path.join("keep.txt").exists());
            if !initial_commit {
                assert_eq!(
                    repo.run(&["diff", "--cached", "--name-only"]).trim(),
                    "tracked.txt"
                );
            }
        }
    }

    #[test]
    fn discard_rejects_stale_state_unsafe_paths_and_directories() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("original\n");
        assert!(repo.commit("seed", false).committed);
        fs::write(repo.path.join("tracked.txt"), "keep\n").unwrap();
        let snapshot = load(&repo.path, &AtomicBool::new(false)).unwrap();
        let file = snapshot.files[0].clone();
        repo.run(&["add", "tracked.txt"]);
        let result = operate(
            &repo.path,
            Operation::Discard {
                file: file.clone(),
                target: snapshot.target,
            },
            &AtomicBool::new(false),
        );
        assert!(result.error.is_some());
        let target = commit_target(&repo.git, &repo.path, &AtomicBool::new(false)).unwrap();
        // A fresh target must not make an outdated worktree status acceptable.
        let result = operate(
            &repo.path,
            Operation::Discard {
                file: file.clone(),
                target: target.clone(),
            },
            &AtomicBool::new(false),
        );
        assert!(result.error.is_some());
        fs::create_dir(repo.path.join("directory")).unwrap();
        fs::write(repo.path.join("directory/keep.txt"), "keep\n").unwrap();
        for path in ["../outside.txt", ":(glob)*", "directory", "missing.txt"] {
            let file = ChangedFile {
                path: path.into(),
                ..file.clone()
            };
            let result = operate(
                &repo.path,
                Operation::Discard {
                    file,
                    target: target.clone(),
                },
                &AtomicBool::new(false),
            );
            assert!(result.error.is_some(), "accepted {path}");
        }
        assert_eq!(
            fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "keep\n"
        );
        assert!(repo.path.join("directory/keep.txt").exists());
    }

    #[test]
    fn cached_tags_survive_status_refresh_until_explicitly_reloaded() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("seed\n");
        assert!(repo.commit("seed", false).committed);
        repo.run(&["tag", "v1.0.0"]);
        let cancel = AtomicBool::new(false);
        let snapshot = load(&repo.path, &cancel).unwrap();
        repo.run(&["tag", "v1.0.1"]);
        fs::write(repo.path.join("tracked.txt"), "modified\n").unwrap();
        let cached = load_with_tags(&repo.path, &cancel, Some(snapshot.tags)).unwrap();
        assert_eq!(cached.tags, vec!["v1.0.0"]);
        assert!(
            cached
                .files
                .iter()
                .any(|file| file.path == "tracked.txt" && file.unstaged())
        );
        assert_eq!(
            load(&repo.path, &cancel).unwrap().tags,
            vec!["v1.0.1", "v1.0.0"]
        );
    }

    #[test]
    fn auto_stage_tracked_updates_modifications_deletions_and_manually_added_files() {
        let repo = TestRepository::new().expect("Git required for automatic staging");
        repo.stage("initial\n");
        for name in [
            "deleted.txt",
            "literal[1].txt",
            "literal1.txt",
            "中文 文件.txt",
        ] {
            fs::write(repo.path.join(name), "before\n").unwrap();
        }
        repo.run(&["add", "."]);
        repo.run(&["commit", "-qm", "initial"]);
        repo.stage("partially staged\n");
        fs::write(repo.path.join("tracked.txt"), "latest\n").unwrap();
        fs::remove_file(repo.path.join("deleted.txt")).unwrap();
        for name in ["literal[1].txt", "literal1.txt", "中文 文件.txt"] {
            fs::write(repo.path.join(name), "after\n").unwrap();
        }
        fs::write(repo.path.join("added.txt"), "added\n").unwrap();
        repo.run(&["add", "added.txt"]);
        fs::write(repo.path.join("added.txt"), "edited after adding\n").unwrap();
        fs::write(repo.path.join("untracked.txt"), "manual only\n").unwrap();
        fs::create_dir(repo.path.join("nested")).unwrap();
        let cancel = AtomicBool::new(false);
        let result = operate(
            &repo.path.join("nested"),
            Operation::StageTracked {
                files: [
                    "tracked.txt",
                    "deleted.txt",
                    "literal[1].txt",
                    "中文 文件.txt",
                    "added.txt",
                ]
                .map(str::to_string)
                .to_vec(),
                target: load(&repo.path, &cancel).unwrap().target,
            },
            &cancel,
        );
        assert_eq!(result.error, None, "{result:?}");
        let snapshot = load(&repo.path, &cancel).unwrap();
        assert_eq!(snapshot.staged_count(), 5);
        assert_eq!(snapshot.untracked_count(), 1);
        assert_eq!(snapshot.unstaged_count(), 1);
        assert!(
            snapshot
                .files
                .iter()
                .any(|f| f.path == "literal1.txt" && f.unstaged())
        );
        assert_eq!(repo.run(&["show", ":tracked.txt"]), "latest\n");
        assert_eq!(repo.run(&["show", ":added.txt"]), "edited after adding\n");
        assert!(repo.run(&["ls-files", "--", "untracked.txt"]).is_empty());
    }

    #[test]
    fn auto_stage_tracked_rejects_stale_index_and_preserves_unresolved_conflicts() {
        let repo = TestRepository::new().expect("Git required for automatic staging");
        repo.stage("initial\n");
        repo.run(&["commit", "-qm", "initial"]);
        let cancel = AtomicBool::new(false);
        let target = load(&repo.path, &cancel).unwrap().target;
        repo.stage("changed index\n");
        let result = operate(
            &repo.path,
            Operation::StageTracked {
                files: vec!["tracked.txt".into()],
                target,
            },
            &cancel,
        );
        assert!(result.error.unwrap().contains("暂存区已变化"));
        repo.run(&["commit", "-qm", "main edit"]);
        repo.run(&["checkout", "-qb", "other", "HEAD~"]);
        repo.stage("other branch\n");
        repo.run(&["commit", "-qm", "other edit"]);
        let merge = std::process::Command::new(&repo.git)
            .args(["merge", "main", "--no-edit"])
            .current_dir(&repo.path)
            .output()
            .unwrap();
        assert!(!merge.status.success());
        let conflicts = repo.run(&["ls-files", "--unmerged"]);
        assert!(!conflicts.is_empty());
        let target = load(&repo.path, &cancel).unwrap().target;
        let operation = Operation::StageTracked {
            files: vec!["tracked.txt".into()],
            target,
        };
        assert!(
            operate(&repo.path, operation.clone(), &cancel)
                .error
                .is_some()
        );
        // Conflicts still require manual resolution without an integration marker.
        fs::remove_file(repo.path.join(".git/MERGE_HEAD")).unwrap();
        assert!(
            operate(&repo.path, operation, &cancel)
                .error
                .unwrap()
                .contains("未解决的冲突")
        );
        assert_eq!(repo.run(&["ls-files", "--unmerged"]), conflicts);
    }

    #[test]
    fn stage_selected_files_is_literal_supports_batches_and_preserves_other_changes() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        repo.run(&["commit", "-qm", "initial"]);
        for name in [
            "literal[1].txt",
            "literal1.txt",
            "中文 文件.txt",
            "outside.txt",
        ] {
            std::fs::write(repo.path.join(name), name).unwrap();
        }
        std::fs::create_dir(repo.path.join("nested")).unwrap();
        let target = load(&repo.path, &AtomicBool::new(false)).unwrap().target;
        let result = operate(
            &repo.path.join("nested"),
            Operation::Stage {
                files: vec!["literal[1].txt".into(), "中文 文件.txt".into()],
                target,
            },
            &AtomicBool::new(false),
        );
        assert_eq!(result.error, None, "{result:?}");
        let staged = repo.run(&[
            "-c",
            "core.quotepath=false",
            "diff",
            "--cached",
            "--name-only",
        ]);
        assert!(staged.contains("literal[1].txt"));
        assert!(staged.contains("中文 文件.txt"));
        assert!(
            !staged
                .lines()
                .any(|name| name == "literal1.txt" || name == "outside.txt")
        );
        assert!(
            repo.run(&["status", "--porcelain"])
                .contains("?? outside.txt")
        );
        assert!(!result.committed);
    }

    #[test]
    fn unstage_selected_files_preserves_worktree_and_other_staged_files() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        repo.run(&["commit", "-qm", "initial"]);
        let tracked = repo.run(&["ls-files"]).lines().next().unwrap().to_string();
        std::fs::write(repo.path.join(&tracked), "staged change\n").unwrap();
        std::fs::write(repo.path.join("second.txt"), "second\n").unwrap();
        repo.run(&["add", &tracked, "second.txt"]);
        std::fs::write(repo.path.join(&tracked), "worktree edit\n").unwrap();
        let target = load(&repo.path, &AtomicBool::new(false)).unwrap().target;
        let result = operate(
            &repo.path,
            Operation::Unstage {
                files: vec![tracked.clone()],
                target,
            },
            &AtomicBool::new(false),
        );
        assert_eq!(result.error, None, "{result:?}");
        let status = repo.run(&["status", "--porcelain"]);
        assert!(status.contains("A  second.txt"), "{status}");
        assert!(status.contains(&format!(" M {tracked}")), "{status}");
        assert_eq!(
            std::fs::read_to_string(repo.path.join(&tracked)).unwrap(),
            "worktree edit\n"
        );
    }
    #[test]
    fn staging_rejects_stale_index_and_paths_outside_the_repository() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        repo.run(&["commit", "-qm", "initial"]);
        std::fs::write(repo.path.join("chosen.txt"), "chosen").unwrap();
        let target = load(&repo.path, &AtomicBool::new(false)).unwrap().target;
        for name in ["../outside", "C:/outside", ":(glob)*", "./chosen.txt"] {
            let result = operate(
                &repo.path,
                Operation::Stage {
                    files: vec![name.into()],
                    target: target.clone(),
                },
                &AtomicBool::new(false),
            );
            assert!(result.error.is_some(), "unsafe path {name}");
        }
        std::fs::write(repo.path.join("other.txt"), "other").unwrap();
        repo.run(&["add", "other.txt"]);
        let result = operate(
            &repo.path,
            Operation::Stage {
                files: vec!["chosen.txt".into()],
                target,
            },
            &AtomicBool::new(false),
        );
        assert!(result.error.as_deref().unwrap().contains("已变化"));
        assert!(
            repo.run(&["status", "--porcelain"])
                .contains("?? chosen.txt")
        );
    }

    #[test]
    fn tag_generation_uses_stable_numeric_versions() {
        use crate::store::TagIncrement::*;
        let tags = ["v0.1.9", "v0.1.10", "v9.0.0-rc.1", "other"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(next_tag(&tags, Patch, 1).unwrap(), "v0.1.11");
        assert_eq!(next_tag(&tags, Minor, 1).unwrap(), "v0.2.0");
        assert_eq!(next_tag(&tags, Major, 1).unwrap(), "v1.0.0");
        assert_eq!(next_tag(&[], Patch, 1).unwrap(), "v0.1.0");
    }

    #[test]
    fn tagged_commit_pushes_only_requested_tag_and_rejects_conflicts() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        let remote = repo.seed_remote();
        repo.run(&["tag", "-a", "local-only", "-m", "local only"]);
        repo.run(&["config", "push.followTags", "true"]);
        repo.stage("next\n");
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "release".into(),
                target,
                push: true,
                tag: Some("v0.1.2".into()),
            },
            &cancel,
        );
        assert!(result.committed && result.error.is_none(), "{result:?}");
        let head = repo.run(&["rev-parse", "HEAD"]);
        assert_eq!(head, repo.run(&["rev-parse", "refs/tags/v0.1.2"]));
        assert_eq!(
            head,
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/tags/v0.1.2"
            ])
        );
        assert!(
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "tag",
                "-l",
                "local-only"
            ])
            .trim()
            .is_empty()
        );
        repo.stage("duplicate\n");
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let duplicate = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "duplicate".into(),
                target,
                push: false,
                tag: Some("v0.1.2".into()),
            },
            &cancel,
        );
        assert!(!duplicate.committed && duplicate.error.unwrap().contains("已存在"));
        assert_eq!(head, repo.run(&["rev-parse", "HEAD"]));
    }

    #[test]
    fn invalid_tag_is_rejected_and_locked_tag_reports_partial_commit() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        repo.stage("next\n");
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "invalid".into(),
                target: target.clone(),
                push: false,
                tag: Some("bad tag".into()),
            },
            &cancel,
        );
        assert!(!result.committed && result.error.is_some());
        std::fs::create_dir_all(repo.path.join(".git/refs/tags")).unwrap();
        std::fs::write(repo.path.join(".git/refs/tags/v0.1.4.lock"), "locked").unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "valid commit".into(),
                target,
                push: false,
                tag: Some("v0.1.4".into()),
            },
            &cancel,
        );
        assert!(result.committed && result.error.is_some(), "{result:?}");
        assert_eq!(result.message, "提交已成功，创建标签失败");
        assert_eq!(
            repo.run(&["log", "-1", "--format=%s"]).trim(),
            "valid commit"
        );
    }

    #[test]
    fn conduit_release_checks_staged_cargo_version_before_commit() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        std::fs::create_dir_all(repo.path.join(".github/workflows")).unwrap();
        std::fs::write(
            repo.path.join(".github/workflows/release.yml"),
            "Leo-Lee-j/conduit\nTag must match Cargo.toml\n",
        )
        .unwrap();
        std::fs::write(
            repo.path.join("Cargo.toml"),
            "[package]\nname = \"conduit\"\nversion = \"0.1.1\"\n",
        )
        .unwrap();
        repo.run(&["add", ".github/workflows/release.yml", "Cargo.toml"]);
        // An unstaged version edit must not affect the version that is actually committed.
        std::fs::write(
            repo.path.join("Cargo.toml"),
            "[package]\nname = \"conduit\"\nversion = \"0.1.2\"\n",
        )
        .unwrap();
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "release".into(),
                target,
                push: true,
                tag: Some("v0.1.2".into()),
            },
            &cancel,
        );
        assert!(!result.committed && result.error.unwrap().contains("0.1.1"));
        assert!(validate_conduit_release(&repo.git, &repo.path, "v0.1.1", "", &cancel).is_ok());
        assert!(
            validate_conduit_release(&repo.git, &repo.path, "v0.1.1-rc.1", "", &cancel).is_err()
        );
    }

    #[test]
    fn custom_tag_steps_and_bounds() {
        use crate::store::TagIncrement::*;
        let tags = vec!["v0.1.1".into()];
        assert_eq!(next_tag(&tags, Patch, 10).unwrap(), "v0.1.11");
        assert_eq!(next_tag(&tags, Minor, 2).unwrap(), "v0.3.0");
        assert_eq!(next_tag(&tags, Major, 2).unwrap(), "v2.0.0");
        assert!(next_tag(&tags, Patch, 0).is_err());
        assert!(next_tag(&["v0.0.18446744073709551615".into()], Patch, 1).is_err());
    }

    #[test]
    fn remote_tag_refresh_does_not_modify_local_refs_and_observes_deletion() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        repo.stage("seed\n");
        assert!(repo.commit("seed", false).committed);
        let remote = repo.seed_remote();
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "tag",
            "v1.0.0",
            "refs/heads/main",
        ]);
        repo.run(&["tag", "local-only"]);
        let result = load_remote_tags(&repo.path, &cancel).unwrap();
        assert_eq!(result.remote, "origin");
        assert_eq!(result.names, vec!["v1.0.0"]);
        assert_eq!(repo.run(&["tag", "-l"]).trim(), "local-only");
        repo.run(&["branch", "--unset-upstream"]);
        assert_eq!(
            load_remote_tags(&repo.path, &cancel).unwrap().names,
            vec!["v1.0.0"]
        );
        repo.run(&["checkout", "--detach", "HEAD"]);
        assert_eq!(
            load_remote_tags(&repo.path, &cancel).unwrap().names,
            vec!["v1.0.0"]
        );
        repo.run(&["--git-dir", remote.to_str().unwrap(), "tag", "-d", "v1.0.0"]);
        assert!(
            load_remote_tags(&repo.path, &cancel)
                .unwrap()
                .names
                .is_empty()
        );
        assert_eq!(repo.run(&["tag", "-l"]).trim(), "local-only");
    }

    #[test]
    fn tag_retry_can_use_an_upstream_configured_after_commit() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        repo.stage("seed\n");
        assert!(repo.commit("seed", false).committed);
        repo.stage("release\n");
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "release".into(),
                target,
                push: true,
                tag: Some("v0.1.2".into()),
            },
            &cancel,
        );
        let retry = result.retry_tag.expect("missing upstream retains retry");
        assert!(retry.remote.is_empty());
        let remote = repo.seed_remote();
        let done = operate(&repo.path, Operation::RetryTag(retry.clone()), &cancel);
        assert!(done.retry_completed && done.error.is_none(), "{done:?}");
        assert_eq!(
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/tags/v0.1.2"
            ])
            .trim(),
            retry.oid
        );
    }

    #[test]
    fn tag_retry_requires_branch_push_and_refuses_changed_refs() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        repo.stage("seed\n");
        assert!(repo.commit("seed", false).committed);
        let remote = repo.seed_remote();
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "config",
            "receive.advertiseAtomic",
            "false",
        ]);
        repo.stage("release\n");
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "release".into(),
                target,
                push: true,
                tag: Some("v0.1.2".into()),
            },
            &cancel,
        );
        assert!(result.committed && result.error.is_some(), "{result:?}");
        let retry = result.retry_tag.expect("retry metadata");
        let rejected = operate(&repo.path, Operation::RetryTag(retry.clone()), &cancel);
        assert!(rejected.error.as_deref().unwrap().contains("尚未包含"));
        repo.run(&["push", "origin", "main"]);
        let done = operate(&repo.path, Operation::RetryTag(retry.clone()), &cancel);
        assert!(done.retry_completed && done.error.is_none(), "{done:?}");
        assert_eq!(
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/tags/v0.1.2"
            ])
            .trim(),
            retry.oid
        );
        assert!(
            operate(&repo.path, Operation::RetryTag(retry.clone()), &cancel)
                .error
                .is_none()
        );
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "tag",
            "-f",
            "v0.1.2",
            "HEAD~1",
        ]);
        let conflict = operate(&repo.path, Operation::RetryTag(retry.clone()), &cancel);
        assert!(conflict.error.unwrap().contains("远程已存在"));
        repo.run(&["tag", "-f", "v0.1.2", "HEAD~1"]);
        assert!(
            operate(&repo.path, Operation::RetryTag(retry), &cancel)
                .error
                .unwrap()
                .contains("本地标签")
        );
    }

    #[test]
    fn atomic_tag_conflict_leaves_remote_branch_unchanged() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let cancel = AtomicBool::new(false);
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        let remote = repo.seed_remote();
        repo.run(&["tag", "v0.1.3"]);
        repo.run(&["push", "origin", "refs/tags/v0.1.3"]);
        repo.run(&["tag", "-d", "v0.1.3"]);
        let old = repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "rev-parse",
            "refs/heads/main",
        ]);
        repo.stage("next\n");
        let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "release".into(),
                target,
                push: true,
                tag: Some("v0.1.3".into()),
            },
            &cancel,
        );
        assert!(result.committed && result.error.is_some(), "{result:?}");
        assert!(result.message.contains("本地标签"));
        assert_eq!(
            old,
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/main"
            ])
        );
        assert_ne!(old, repo.run(&["rev-parse", "refs/tags/v0.1.3"]));
    }

    #[test]
    fn commit_only_includes_index_and_preserves_unstaged_and_untracked_files() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        repo.stage("staged\n");
        std::fs::write(repo.path.join("tracked.txt"), "unstaged\n").unwrap();
        std::fs::write(repo.path.join("private.txt"), "untracked\n").unwrap();
        let message = "feat: 提交说明 `literal` $(literal)\n\n仅提交暂存区。";
        let outcome = repo.commit(message, false);
        assert!(outcome.committed && outcome.error.is_none(), "{outcome:?}");
        assert_eq!(repo.run(&["show", "HEAD:tracked.txt"]).trim(), "staged");
        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "unstaged\n"
        );
        assert!(
            repo.run(&["status", "--porcelain"])
                .contains("?? private.txt")
        );
        assert_eq!(repo.run(&["log", "-1", "--format=%B"]).trim(), message);
        assert_eq!(repo.metadata().ahead, 2); // No upstream: local unpublished history.
    }

    #[test]
    fn commit_rejects_changed_index_branch_and_invalid_requests() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        repo.stage("first\n");
        let target = commit_target(&repo.git, &repo.path, &AtomicBool::new(false)).unwrap();
        let before = repo.run(&["rev-parse", "HEAD"]);
        repo.stage("second\n");
        let changed = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "stale".into(),
                target,
                push: false,
                tag: None,
            },
            &AtomicBool::new(false),
        );
        assert!(!changed.committed && changed.error.unwrap().contains("变化"));
        let target = commit_target(&repo.git, &repo.path, &AtomicBool::new(false)).unwrap();
        repo.run(&["checkout", "-q", "-b", "other"]);
        let changed = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "wrong branch".into(),
                target,
                push: false,
                tag: None,
            },
            &AtomicBool::new(false),
        );
        assert!(!changed.committed && changed.error.unwrap().contains("变化"));
        assert!(!repo.commit(" \n", false).committed);
        assert_eq!(repo.run(&["rev-parse", "HEAD"]), before);
        let target = commit_target(&repo.git, &repo.path, &AtomicBool::new(false)).unwrap();
        let cancelled = operate(
            &repo.path,
            Operation::Commit {
                files: None,
                message: "cancelled".into(),
                target,
                push: false,
                tag: None,
            },
            &AtomicBool::new(true),
        );
        assert!(!cancelled.committed);
        assert_eq!(repo.run(&["rev-parse", "HEAD"]), before);
        repo.run(&["checkout", "--detach", "-q"]);
        assert!(
            repo.commit("detached", false)
                .error
                .unwrap()
                .contains("分离")
        );
    }

    #[test]
    fn push_counts_follow_upstream_and_commit_and_push_clears_pending() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        let remote = repo.seed_remote();
        assert_eq!(repo.metadata().ahead, 0);
        repo.stage("second\n");
        assert!(repo.commit("second", false).committed);
        assert_eq!(repo.metadata().ahead, 1);
        repo.stage("third\n");
        assert!(repo.commit("third", false).committed);
        assert_eq!(repo.metadata().ahead, 2);
        let repository = repo.metadata();
        let selected = repository
            .branch_info
            .into_iter()
            .find(|branch| branch.full_ref == "refs/heads/main")
            .unwrap();
        let pushed = operate(
            &repo.path,
            Operation::Branch(Box::new(BranchRequest {
                expected: repository.context,
                selected: Some(selected),
                action: BranchAction::Push {
                    remote: "origin".into(),
                    destination: "main".into(),
                    set_upstream: false,
                },
            })),
            &AtomicBool::new(false),
        );
        assert!(pushed.error.is_none(), "{pushed:?}");
        assert_eq!(repo.metadata().ahead, 0);
        repo.stage("fourth\n");
        let outcome = repo.commit("fourth", true);
        assert!(outcome.committed && outcome.error.is_none(), "{outcome:?}");
        assert_eq!(repo.metadata().ahead, 0);
        assert_eq!(
            repo.run(&["rev-parse", "HEAD"]),
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/main"
            ])
        );
    }

    #[test]
    fn failed_push_keeps_successful_commit_and_pending_count() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        let no_remote = repo.commit("first", true);
        assert!(no_remote.committed && no_remote.error.unwrap().contains("远程仓库"));
        assert_eq!(repo.metadata().ahead, 1);
        repo.seed_remote();
        // Keep the tracking ref but point the remote at a missing local directory.
        repo.run(&[
            "remote",
            "set-url",
            "origin",
            repo.path.join("missing.git").to_str().unwrap(),
        ]);
        repo.stage("second\n");
        let failed = repo.commit("second", true);
        assert!(failed.committed && failed.error.is_some());
        assert_eq!(failed.message, "提交已成功，推送失败");
        assert_eq!(repo.metadata().ahead, 1);
        assert_eq!(repo.run(&["log", "-1", "--format=%s"]).trim(), "second");
        assert!(
            repo.commit("no staged changes", false)
                .error
                .unwrap()
                .contains("暂存")
        );
        assert_eq!(repo.run(&["rev-list", "--count", "HEAD"]).trim(), "2");
    }

    #[test]
    fn branches_preserve_slashes_unicode_and_skip_remote_head_aliases() {
        let branches = parse_branches(
            "refs/heads/feature/界面\0\nrefs/heads/main\0\nrefs/remotes/origin/HEAD\0refs/remotes/origin/main\nrefs/remotes/origin/main\0\n",
        );
        assert_eq!(
            branches,
            vec![
                Branch {
                    name: "feature/界面".into(),
                    remote: false
                },
                Branch {
                    name: "main".into(),
                    remote: false
                },
                Branch {
                    name: "origin/main".into(),
                    remote: true
                },
            ]
        );
    }

    #[test]
    fn status_parser_handles_tracked_untracked_renames_and_kinds() {
        let files = parse_status(
            b" M src/main.rs\0?? notes.md\0R  src/new.rs\0src/old.rs\0A  staged.txt\0 D gone.txt\0",
        );
        assert_eq!(files.len(), 5);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].kind(), ChangeKind::Modified);
        assert!(files[0].unstaged() && !files[0].staged());
        assert_eq!(files[1].kind(), ChangeKind::Added);
        assert!(files[1].untracked());
        assert_eq!(files[2].kind(), ChangeKind::Modified);
        assert_eq!(files[2].original.as_deref(), Some("src/old.rs"));
        assert_eq!(files[3].kind(), ChangeKind::Added);
        assert!(files[3].staged());
        assert_eq!(files[4].kind(), ChangeKind::Deleted);
        assert!(parse_status(b"").is_empty());
        assert!(parse_status(b"broken\0").is_empty());
    }

    #[test]
    fn diff_truncation_keeps_utf8_and_notes_the_cut() {
        let mut notes = vec![];
        let diff = "汉".repeat(MAX_DIFF_BYTES);
        let truncated = truncate_diff(diff, &mut notes);
        assert!(truncated.len() < MAX_DIFF_BYTES + 64);
        assert!(truncated.ends_with("…（已截断）\n"));
        assert!(notes.iter().any(|note| note.contains("截断")));
        let small = "tiny".to_string();
        let mut notes = vec![];
        assert_eq!(truncate_diff(small.clone(), &mut notes), small);
        assert!(notes.is_empty());
    }

    #[test]
    fn snapshot_counts_distinguish_staged_and_worktree_changes() {
        let snapshot = Snapshot {
            branch: "main".into(),
            tags: vec![],
            files: parse_status(b"M  staged\0 M worktree\0MM both\0?? new\0"),
            staged_diff: String::new(),
            notes: vec![],
            target: CommitTarget::default(),
        };
        assert_eq!(snapshot.files.len(), 4);
        // M ,  M and MM carry one status each: staged 2, unstaged 2.
        assert_eq!(snapshot.staged_count(), 2);
        assert_eq!(snapshot.unstaged_count(), 2);
        assert_eq!(snapshot.untracked_count(), 1);
        assert!(!snapshot.is_clean());
        assert!(
            Snapshot {
                files: vec![],
                ..snapshot
            }
            .is_clean()
        );
    }

    #[test]
    fn real_snapshot_is_read_only_and_sees_worktree_changes() {
        let Some(git) = resolve_git() else {
            return;
        };
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("conduit-git-panel-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let run = |args: &[&str]| {
            let output = std::process::Command::new(&git)
                .args(args)
                .current_dir(&directory)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        assert!(load_repository(&directory, &AtomicBool::new(false)).is_err());
        run(&["init", "-q"]);
        let unborn = load_repository(&directory, &AtomicBool::new(false)).unwrap();
        assert!(!unborn.detached);
        assert_eq!(
            unborn.branches,
            vec![Branch {
                name: unborn.current.clone(),
                remote: false
            }]
        );
        std::fs::write(directory.join("tracked.txt"), "one\n").unwrap();
        run(&["add", "tracked.txt"]);
        run(&[
            "-c",
            "user.name=Conduit Test",
            "-c",
            "user.email=conduit@example.invalid",
            "commit",
            "-q",
            "-m",
            "seed",
        ]);
        run(&["branch", "feature/ui"]);
        run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        run(&[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ]);
        std::fs::write(directory.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(directory.join("new.txt"), "untracked\n").unwrap();

        let before = read_repository_bytes(&directory);
        let repository = load_repository(&directory, &AtomicBool::new(false)).unwrap();
        assert_eq!(repository.current, unborn.current);
        assert!(repository.branches.contains(&Branch {
            name: "feature/ui".into(),
            remote: false
        }));
        assert!(repository.branches.contains(&Branch {
            name: "origin/main".into(),
            remote: true
        }));
        assert!(
            !repository
                .branches
                .iter()
                .any(|branch| branch.name == "origin/HEAD")
        );
        let snapshot = load(&directory, &AtomicBool::new(false)).unwrap();
        assert!(!snapshot.branch.is_empty());
        assert!(snapshot.files.iter().any(|file| file.path == "tracked.txt"
            && file.kind() == ChangeKind::Modified
            && file.unstaged()));
        assert!(
            snapshot
                .files
                .iter()
                .any(|file| file.path == "new.txt" && file.untracked())
        );
        assert!(snapshot.staged_diff.is_empty());
        // Loading the panel must not touch the repository (no index lock, no
        // staging, no commits).
        assert_eq!(read_repository_bytes(&directory), before);
        run(&["checkout", "--detach", "-q"]);
        let detached_before = read_repository_bytes(&directory);
        let detached = load_repository(&directory, &AtomicBool::new(false)).unwrap();
        assert!(detached.detached);
        assert_eq!(
            detached.current,
            run(&["rev-parse", "--short", "HEAD"]).trim()
        );
        assert_eq!(read_repository_bytes(&directory), detached_before);
        let plain_directory = directory.join("not-a-repo");
        std::fs::create_dir(&plain_directory).unwrap();
        // A child directory belongs to its parent repository.
        assert!(load_repository(&plain_directory, &AtomicBool::new(false)).is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Byte fingerprint of the files and index under `.git`, for the read-only
    /// assertion above.
    fn read_repository_bytes(directory: &Path) -> Vec<u8> {
        let mut entries: Vec<_> = std::fs::read_dir(directory.join(".git"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .collect();
        entries.sort();
        let mut bytes = vec![];
        for path in entries {
            bytes.extend(std::fs::read(path).unwrap());
        }
        bytes
    }

    fn changed_file(path: &str, index: char, worktree: char) -> ChangedFile {
        ChangedFile {
            index,
            worktree,
            path: path.into(),
            original: None,
        }
    }

    fn parse_file_diff(patch: &str) -> Vec<gpui_kit::component::diff::DiffFile> {
        gpui_kit::component::diff::DiffFile::parse(patch)
            .expect("generated patch should parse as a Git diff")
    }

    #[test]
    fn file_diff_compares_working_tree_with_head() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("top\nalpha\nbeta\ngamma\ndelta\n");
        assert!(repo.commit("seed", false).committed);
        std::fs::write(
            repo.path.join("tracked.txt"),
            "top\nalpha\nBETA\ngamma\ndelta\n",
        )
        .unwrap();

        let diff = load_file_diff(
            &repo.path,
            &changed_file("tracked.txt", ' ', 'M'),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(diff.patch.contains("-beta"));
        assert!(diff.patch.contains("+BETA"));
        // Full-file context keeps untouched lines in the patch so the viewer
        // can expand unchanged regions instead of dropping them.
        assert!(diff.patch.contains(" top"));
        assert!(diff.patch.contains(" delta"));
        let files = parse_file_diff(&diff.patch);
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].status(),
            gpui_kit::component::diff::DiffFileStatus::Modified
        );
        assert_eq!(files[0].path().as_str(), "tracked.txt");
        assert_eq!(files[0].additions(), 1);
        assert_eq!(files[0].deletions(), 1);
    }

    #[test]
    fn file_diff_scopes_keep_index_separate_from_worktree_and_other_files() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.run(&["config", "core.autocrlf", "false"]);
        repo.stage("base\n");
        assert!(repo.commit("seed", false).committed);
        repo.stage("staged A\n");
        fs::write(repo.path.join("tracked.txt"), "unstaged B\n").unwrap();
        fs::write(repo.path.join("unchecked.txt"), "unchecked C\n").unwrap();
        repo.run(&["add", "--", "unchecked.txt"]);
        let file = changed_file("tracked.txt", 'M', 'M');
        let cancel = AtomicBool::new(false);
        let staged =
            load_file_diff_scoped(&repo.path, &file, FileDiffScope::Staged, &cancel).unwrap();
        let unstaged =
            load_file_diff_scoped(&repo.path, &file, FileDiffScope::Unstaged, &cancel).unwrap();
        let all = load_file_diff_scoped(&repo.path, &file, FileDiffScope::All, &cancel).unwrap();
        assert!(staged.patch.contains("-base\n") && staged.patch.contains("+staged A\n"));
        assert!(!staged.patch.contains("unstaged B"));
        assert!(unstaged.patch.contains("-staged A\n") && unstaged.patch.contains("+unstaged B\n"));
        assert!(all.patch.contains("-base\n") && all.patch.contains("+unstaged B\n"));
        for diff in [&staged, &unstaged, &all] {
            assert!(!diff.patch.contains("unchecked C"));
        }
        assert_eq!(repo.run(&["show", ":tracked.txt"]), "staged A\n");
        assert_eq!(repo.run(&["show", ":unchecked.txt"]), "unchecked C\n");
    }

    #[test]
    fn file_diff_scopes_handle_unborn_index_deleted_and_untracked_files() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.run(&["config", "core.autocrlf", "false"]);
        repo.stage("first index\n");
        fs::write(repo.path.join("tracked.txt"), "first working\n").unwrap();
        let file = changed_file("tracked.txt", 'A', 'M');
        let cancel = AtomicBool::new(false);
        let staged =
            load_file_diff_scoped(&repo.path, &file, FileDiffScope::Staged, &cancel).unwrap();
        assert!(staged.patch.contains("+first index") && !staged.patch.contains("first working"));
        let unstaged =
            load_file_diff_scoped(&repo.path, &file, FileDiffScope::Unstaged, &cancel).unwrap();
        assert!(
            unstaged.patch.contains("-first index") && unstaged.patch.contains("+first working")
        );
        assert!(repo.commit("seed", false).committed);
        repo.run(&["rm", "-f", "--", "tracked.txt"]);
        let deleted = load_file_diff_scoped(
            &repo.path,
            &changed_file("tracked.txt", 'D', ' '),
            FileDiffScope::Staged,
            &cancel,
        )
        .unwrap();
        assert!(deleted.patch.contains("-first index"));
        fs::write(repo.path.join("new.txt"), "untracked\n").unwrap();
        let new = changed_file("new.txt", '?', '?');
        assert!(
            load_file_diff_scoped(&repo.path, &new, FileDiffScope::Staged, &cancel)
                .unwrap()
                .patch
                .is_empty()
        );
        assert!(
            load_file_diff_scoped(&repo.path, &new, FileDiffScope::Unstaged, &cancel)
                .unwrap()
                .patch
                .contains("+untracked")
        );
    }

    #[test]
    fn file_diff_treats_untracked_and_added_files_as_new() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        std::fs::write(repo.path.join("notes.md"), "alpha\nbeta\n").unwrap();
        let untracked = load_file_diff(
            &repo.path,
            &changed_file("notes.md", '?', '?'),
            &AtomicBool::new(false),
        )
        .unwrap();
        let files = parse_file_diff(&untracked.patch);
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].status(),
            gpui_kit::component::diff::DiffFileStatus::Added
        );
        assert_eq!(files[0].additions(), 2);
        assert_eq!(files[0].deletions(), 0);

        // An added file in a repository without HEAD still gets a readable
        // empty-to-working-tree preview.
        repo.run(&["add", "notes.md"]);
        let added = load_file_diff(
            &repo.path,
            &changed_file("notes.md", 'A', ' '),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(added.notes.iter().any(|note| note.contains("尚无提交")));
        let files = parse_file_diff(&added.patch);
        assert_eq!(
            files[0].status(),
            gpui_kit::component::diff::DiffFileStatus::Added
        );
        assert_eq!(files[0].additions(), 2);
    }

    #[test]
    fn file_diff_shows_deletions_and_binary_markers() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        std::fs::remove_file(repo.path.join("tracked.txt")).unwrap();
        let deleted = load_file_diff(
            &repo.path,
            &changed_file("tracked.txt", ' ', 'D'),
            &AtomicBool::new(false),
        )
        .unwrap();
        let files = parse_file_diff(&deleted.patch);
        assert_eq!(
            files[0].status(),
            gpui_kit::component::diff::DiffFileStatus::Deleted
        );
        assert_eq!(files[0].deletions(), 1);

        std::fs::write(repo.path.join("blob.bin"), b"abc\0def").unwrap();
        let binary = load_file_diff(
            &repo.path,
            &changed_file("blob.bin", '?', '?'),
            &AtomicBool::new(false),
        )
        .unwrap();
        let files = parse_file_diff(&binary.patch);
        assert!(files[0].is_binary());
        assert_eq!(
            files[0].status(),
            gpui_kit::component::diff::DiffFileStatus::Added
        );
    }

    #[test]
    fn file_diff_rejects_oversized_patches() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let line = "0123456789abcdef\n";
        let body = line.repeat(5 * 1024 * 1024 / line.len());
        repo.stage(&body);
        assert!(repo.commit("seed large file", false).committed);
        std::fs::write(
            repo.path.join("tracked.txt"),
            format!("modified\n{}", &body[line.len()..]),
        )
        .unwrap();

        let error = load_file_diff(
            &repo.path,
            &changed_file("tracked.txt", ' ', 'M'),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("超过 4 MiB"));
    }

    #[test]
    fn file_diff_resolves_paths_from_a_workspace_subdirectory() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        repo.stage("initial\n");
        assert!(repo.commit("seed", false).committed);
        std::fs::create_dir(repo.path.join("nested")).unwrap();
        std::fs::write(repo.path.join("tracked.txt"), "nested edit\n").unwrap();
        let diff = load_file_diff(
            &repo.path.join("nested"),
            &changed_file("tracked.txt", ' ', 'M'),
            &AtomicBool::new(false),
        )
        .unwrap();
        let files = parse_file_diff(&diff.patch);
        assert_eq!(files[0].path().as_str(), "tracked.txt");
        assert_eq!(files[0].additions(), 1);
    }

    #[test]
    fn file_diff_rejects_paths_outside_the_repository() {
        let Some(repo) = TestRepository::new() else {
            return;
        };
        let error = load_file_diff(
            &repo.path,
            &changed_file("../outside.txt", '?', '?'),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("仓库范围"));
    }
}
