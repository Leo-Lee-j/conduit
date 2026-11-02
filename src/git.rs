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
pub use branches::{
    BranchAction, BranchContext, BranchInfo, BranchRequest, IntegrationState, repository_scope,
};
pub use compare::{
    Comparison, ComparisonKind, ComparisonRequest, load_comparison, load_comparison_diff,
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
    pub const ALL: [Self; 3] = [Self::Added, Self::Modified, Self::Deleted];

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
    pub files: Vec<ChangedFile>,
    /// Staged patch used only for commit-message generation.
    pub staged_diff: String,
    pub notes: Vec<String>,
    pub target: CommitTarget,
}

/// Patch for one file, compared against HEAD in the diff window.
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
    Unstage {
        files: Vec<String>,
        target: CommitTarget,
    },
    Branch(Box<BranchRequest>),
    Commit {
        message: String,
        target: CommitTarget,
        push: bool,
    },
    Push {
        branch: String,
        upstream: String,
    },
}

#[derive(Clone, Debug)]
pub struct OperationResult {
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
    pub fn start(path: PathBuf) -> Self {
        Self::start_with(path, load)
    }
}

impl BranchProbe {
    pub fn start(path: PathBuf) -> Self {
        Self::start_with(path, load_repository)
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
    Ok(Snapshot {
        branch,
        files,
        staged_diff,
        notes,
        target,
    })
}

/// Read the patch for one changed file. Tracked files are compared against
/// HEAD, so the result includes both staged and unstaged edits. Untracked and
/// still-uncommitted added files are shown as a new file against an empty side.
pub fn load_file_diff(
    workspace: &Path,
    file: &ChangedFile,
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
    let patch = if file.untracked() {
        let content = read_file_content(&root, &file.path)?;
        if content.truncated {
            notes.push(format!(
                "文件超过 {} MiB，仅显示前部分内容",
                MAX_FILE_DIFF_BYTES / (1024 * 1024)
            ));
        }
        added_file_patch(&file.path, content.text.as_deref())
    } else if run_git(&git, &root, &["rev-parse", "--verify", "HEAD"], cancel).is_err() {
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
        let output = run_git_bounded(
            &git,
            &root,
            &[
                "--no-optional-locks",
                "--no-pager",
                "-c",
                "core.quotepath=false",
                "diff",
                "HEAD",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                &context,
                "--",
                &file.path,
            ],
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

fn operate(workspace: &Path, operation: Operation, cancel: &AtomicBool) -> OperationResult {
    if let Operation::Branch(request) = operation {
        return branches::operate_branch(workspace, *request, cancel);
    }
    let mut committed = false;
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
            Operation::Stage { files, target } => {
                if files.is_empty() {
                    bail!("请选择需要暂存的文件");
                }
                if commit_target(&git, workspace, cancel)? != target {
                    bail!("分支、HEAD 或暂存区已变化，请刷新后重新暂存");
                }
                // Porcelain paths are repository-relative, even from a nested workspace.
                // Literal NUL-delimited pathspecs avoid wildcard expansion and Windows'
                // command-line length limit, and never stage unselected files.
                for file in &files {
                    if file.is_empty()
                        || file.contains('\0')
                        || Path::new(file)
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                        || file
                            .split(['/', '\\'])
                            .any(|c| c == ".." || c == "." || c.is_empty())
                        || file.contains(':')
                    {
                        bail!("暂存路径必须是仓库内的文件：{file}");
                    }
                }
                let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
                let root = PathBuf::from(root.trim());
                let spec = std::env::temp_dir().join(format!(
                    "conduit-stage-{}-{}.paths",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_nanos()
                ));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&spec)?;
                let result = (|| -> Result<String> {
                    use std::io::Write as _;
                    for path in &files {
                        file.write_all(path.as_bytes())?;
                        file.write_all(&[0])?;
                    }
                    drop(file);
                    run_git_with_timeout(
                        &git,
                        &root,
                        &[
                            "--literal-pathspecs",
                            "add",
                            &format!("--pathspec-from-file={}", spec.display()),
                            "--pathspec-file-nul",
                        ],
                        cancel,
                        Duration::from_secs(120),
                    )
                })();
                let _ = std::fs::remove_file(&spec);
                result?;
                Ok(format!("已暂存 {} 个文件", files.len()))
            }
            Operation::Unstage { files, target } => {
                if files.is_empty() {
                    bail!("请选择需要移出暂存区的文件");
                }
                if commit_target(&git, workspace, cancel)? != target {
                    bail!("分支、HEAD 或暂存区已变化，请刷新后重新移出暂存");
                }
                for file in &files {
                    if file.is_empty()
                        || file.contains('\0')
                        || Path::new(file)
                            .components()
                            .any(|c| !matches!(c, std::path::Component::Normal(_)))
                        || file
                            .split(['/', '\\'])
                            .any(|c| c == ".." || c == "." || c.is_empty())
                        || file.contains(':')
                    {
                        bail!("暂存路径必须是仓库内的文件：{file}");
                    }
                }
                let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
                let root = PathBuf::from(root.trim());
                let spec = std::env::temp_dir().join(format!(
                    "conduit-unstage-{}-{}.paths",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_nanos()
                ));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&spec)?;
                let result = (|| -> Result<String> {
                    use std::io::Write as _;
                    for path in &files {
                        file.write_all(path.as_bytes())?;
                        file.write_all(&[0])?;
                    }
                    drop(file);
                    run_git_with_timeout(
                        &git,
                        &root,
                        &[
                            "restore",
                            "--staged",
                            &format!("--pathspec-from-file={}", spec.display()),
                            "--pathspec-file-nul",
                        ],
                        cancel,
                        Duration::from_secs(120),
                    )
                })();
                let _ = std::fs::remove_file(&spec);
                result?;
                Ok(format!("已将 {} 个文件移出暂存区", files.len()))
            }
            Operation::Commit {
                message,
                target,
                push,
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
                // No add/-a: Git commits exactly its index and preserves unstaged files.
                run_git_with_timeout(
                    &git,
                    workspace,
                    &["commit", "-m", message.trim()],
                    cancel,
                    Duration::from_secs(120),
                )?;
                committed = true;
                if push {
                    push_upstream(
                        &git,
                        workspace,
                        target.branch.as_deref().unwrap(),
                        None,
                        cancel,
                    )?;
                    Ok("提交并推送成功".into())
                } else {
                    Ok("提交成功".into())
                }
            }
            Operation::Push { branch, upstream } => {
                push_upstream(&git, workspace, &branch, Some(&upstream), cancel)?;
                Ok("推送成功".into())
            }
        }
    })();
    match result {
        Ok(message) => OperationResult {
            worktree_created: None,
            committed,
            message,
            error: None,
        },
        Err(error) => OperationResult {
            worktree_created: None,
            committed,
            message: if committed {
                "提交已成功，推送失败".into()
            } else {
                "Git 操作失败".into()
            },
            error: Some(format!("{error:#}")),
        },
    }
}

fn push_upstream(
    git: &Path,
    workspace: &Path,
    branch: &str,
    expected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<()> {
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
    .map_err(|_| anyhow!("当前分支未设置有效上游，请先在终端配置上游后重试推送"))?;
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
    let remote = remote.trim();
    let merge = merge.trim();
    if remote.is_empty() || !merge.starts_with("refs/heads/") {
        bail!("上游配置不完整，请检查 Git 分支配置");
    }
    run_git_with_timeout(
        git,
        workspace,
        &[
            "-c",
            "credential.interactive=false",
            "push",
            "--porcelain",
            "--",
            remote,
            &format!("HEAD:{merge}"),
        ],
        cancel,
        Duration::from_secs(120),
    )?;
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
    if cancel.load(Ordering::Acquire) {
        bail!("Git 读取已取消");
    }
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let environment = std::collections::BTreeMap::from([
        ("GIT_EDITOR".into(), "true".into()),
        ("GIT_SEQUENCE_EDITOR".into(), "true".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GCM_INTERACTIVE".into(), "Never".into()),
    ]);
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
                    message: message.into(),
                    target,
                    push,
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
                message: "stale".into(),
                target,
                push: false,
            },
            &AtomicBool::new(false),
        );
        assert!(!changed.committed && changed.error.unwrap().contains("变化"));
        let target = commit_target(&repo.git, &repo.path, &AtomicBool::new(false)).unwrap();
        repo.run(&["checkout", "-q", "-b", "other"]);
        let changed = operate(
            &repo.path,
            Operation::Commit {
                message: "wrong branch".into(),
                target,
                push: false,
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
                message: "cancelled".into(),
                target,
                push: false,
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
        let pushed = operate(
            &repo.path,
            Operation::Push {
                branch: "main".into(),
                upstream: "origin/main".into(),
            },
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
        let no_upstream = repo.commit("first", true);
        assert!(no_upstream.committed && no_upstream.error.unwrap().contains("上游"));
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
