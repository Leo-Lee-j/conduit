//! Read-only Git status and diff snapshots for the selected workspace.
//!
//! Only non-mutating plumbing is executed: `rev-parse`, `status` and `diff`.
//! `--no-optional-locks` keeps `status`/`diff` from taking the index lock, so
//! viewing the panel never competes with the CLI's own Git commands. Output is
//! bounded and every probe runs on its own thread with a timeout and a killed
//! process tree, mirroring the CLI version probe.
use crate::child::{Child, available};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
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
/// Bytes of each diff section kept for rendering.
pub const MAX_DIFF_BYTES: usize = 200 * 1024;
/// Hard cap on the bytes read from one Git command (the rest is drained).
const MAX_COMMAND_BYTES: usize = 512 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const READ_CHUNK: usize = 64 * 1024;

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
    /// User-facing change kind, preferring the worktree status for readability.
    pub fn label(&self) -> &'static str {
        let code = if self.worktree != ' ' && self.worktree != '?' {
            self.worktree
        } else {
            self.index
        };
        match code {
            '?' => "未跟踪",
            'A' => "新增",
            'D' => "删除",
            'R' => "重命名",
            'C' => "复制",
            'M' => "修改",
            'T' => "类型变更",
            'U' => "冲突",
            _ => "变更",
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub branch: String,
    pub files: Vec<ChangedFile>,
    pub staged_diff: String,
    pub unstaged_diff: String,
    pub notes: Vec<String>,
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
pub struct GitProbe {
    pub path: PathBuf,
    pub events: Receiver<Result<Snapshot, String>>,
    cancelled: Arc<AtomicBool>,
}

impl GitProbe {
    pub fn start(path: PathBuf) -> Self {
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

impl Drop for GitProbe {
    fn drop(&mut self) {
        self.cancel();
    }
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
        &["rev-parse", "--abbrev-ref", "HEAD"],
        cancel,
    )
    .map(|output| output.trim().to_string())
    .ok()
    .filter(|branch| !branch.is_empty())
    .unwrap_or_else(|| "（尚无提交）".into());

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
        ],
        cancel,
    )?;
    let unstaged_diff = run_git(
        &git,
        workspace,
        &[
            "--no-optional-locks",
            "--no-pager",
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-color",
            "--no-ext-diff",
        ],
        cancel,
    )?;
    let staged_diff = truncate_diff(staged_diff, &mut notes);
    let unstaged_diff = truncate_diff(unstaged_diff, &mut notes);
    if files.iter().any(|file| file.untracked()) {
        notes.push("未跟踪文件只列出路径，不读取内容".into());
    }
    Ok(Snapshot {
        branch,
        files,
        staged_diff,
        unstaged_diff,
        notes,
    })
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Header,
    Hunk,
    Added,
    Removed,
}

/// Classify diff lines so the UI can colour them without per-line elements.
/// Returns byte ranges into `diff` and only lines that carry a style.
pub fn diff_highlights(diff: &str) -> Vec<(std::ops::Range<usize>, DiffLine)> {
    let mut ranges = vec![];
    let mut offset = 0;
    let mut in_hunk = false;
    for line in diff.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        // Inside a hunk every line is content/context, so a payload line that
        // happens to start with "+++" or "---" must stay Added/Removed. Only
        // the file headers before the first "@@" are metadata.
        let kind = if text.starts_with("diff --git") {
            in_hunk = false;
            Some(DiffLine::Header)
        } else if text.starts_with("@@") {
            in_hunk = true;
            Some(DiffLine::Hunk)
        } else if text.starts_with("\\ No newline") || text.starts_with("Binary files") {
            Some(DiffLine::Header)
        } else if in_hunk {
            if text.starts_with('+') {
                Some(DiffLine::Added)
            } else if text.starts_with('-') {
                Some(DiffLine::Removed)
            } else {
                None
            }
        } else if text.starts_with("index ")
            || text.starts_with("+++")
            || text.starts_with("---")
            || text.starts_with("new file")
            || text.starts_with("deleted file")
            || text.starts_with("old mode")
            || text.starts_with("new mode")
            || text.starts_with("similarity ")
            || text.starts_with("rename ")
            || text.starts_with("copy ")
        {
            Some(DiffLine::Header)
        } else {
            None
        };
        if let Some(kind) = kind {
            ranges.push((offset..offset + line.len(), kind));
        }
        offset += line.len();
    }
    ranges
}

fn truncate_diff(diff: String, notes: &mut Vec<String>) -> String {
    if diff.len() <= MAX_DIFF_BYTES {
        return diff;
    }
    // Keep the prefix on a UTF-8 boundary so StyledText never splits a glyph.
    let mut end = MAX_DIFF_BYTES;
    while end > 0 && !diff.is_char_boundary(end) {
        end -= 1;
    }
    notes.push(format!(
        "diff 已截断，仅显示前 {} KiB",
        MAX_DIFF_BYTES / 1024
    ));
    let mut truncated = diff[..end].to_string();
    truncated.push_str("\n…（已截断）\n");
    truncated
}

fn read_available(file: &mut File, output: &mut Vec<u8>, truncated: &mut bool) -> Result<()> {
    if let Ok(count) = available(file)
        && count > 0
    {
        let mut buffer = [0; READ_CHUNK];
        let count = file.read(&mut buffer[..(count as usize).min(READ_CHUNK)])?;
        let room = MAX_COMMAND_BYTES.saturating_sub(output.len());
        if room == 0 {
            *truncated = true;
        } else {
            let keep = count.min(room);
            output.extend_from_slice(&buffer[..keep]);
            if keep < count {
                *truncated = true;
            }
        }
    }
    Ok(())
}

/// Run one Git command in `workspace`, bounded in time and output size.
fn run_git(git: &Path, workspace: &Path, args: &[&str], cancel: &AtomicBool) -> Result<String> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let (child, mut pipes) = Child::spawn(git, &args, workspace)
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
            if started.elapsed() > COMMAND_TIMEOUT {
                bail!("Git 命令超时，已停止进程树");
            }
            read_available(&mut pipes.stdout, &mut stdout, &mut truncated)?;
            read_available(&mut pipes.stderr, &mut stderr, &mut truncated)?;
            if let Some(code) = child.exit_code()? {
                while available(&pipes.stdout).unwrap_or(0) > 0 {
                    read_available(&mut pipes.stdout, &mut stdout, &mut truncated)?;
                }
                while available(&pipes.stderr).unwrap_or(0) > 0 {
                    read_available(&mut pipes.stderr, &mut stderr, &mut truncated)?;
                }
                if code != 0 {
                    let stderr = String::from_utf8_lossy(&stderr);
                    bail!("Git 命令失败（退出码 {code}）：{}", stderr.trim());
                }
                return Ok(String::from_utf8_lossy(&stdout).into_owned());
            }
            thread::sleep(Duration::from_millis(10));
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
    use super::*;

    #[test]
    fn status_parser_handles_tracked_untracked_and_renames() {
        let files = parse_status(
            b" M src/main.rs\0?? notes.md\0R  src/new.rs\0src/old.rs\0A  staged.txt\0",
        );
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].label(), "修改");
        assert!(files[0].unstaged() && !files[0].staged());
        assert_eq!(files[1].label(), "未跟踪");
        assert!(files[1].untracked());
        assert_eq!(files[2].label(), "重命名");
        assert_eq!(files[2].original.as_deref(), Some("src/old.rs"));
        assert_eq!(files[3].label(), "新增");
        assert!(files[3].staged());
        assert!(parse_status(b"").is_empty());
        assert!(parse_status(b"broken\0").is_empty());
    }

    #[test]
    fn diff_highlights_classify_git_patch_lines() {
        let diff = "diff --git a/a b/a\nindex 1..2 100644\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n context\n\\ No newline at end of file\ndiff --git a/b b/b\nindex 3..4 100644\n--- a/b\n+++ b/b\n@@ -1 +1 @@\n+++payload\n---payload\n";
        let kinds: Vec<_> = diff_highlights(diff)
            .into_iter()
            .map(|(range, kind)| (&diff[range], kind))
            .collect();
        assert!(kinds.contains(&("diff --git a/a b/a\n", DiffLine::Header)));
        assert!(kinds.contains(&("+++ b/a\n", DiffLine::Header)));
        assert!(kinds.contains(&("@@ -1 +1 @@\n", DiffLine::Hunk)));
        assert!(kinds.contains(&("-old\n", DiffLine::Removed)));
        assert!(kinds.contains(&("+new\n", DiffLine::Added)));
        assert!(!kinds.iter().any(|(text, _)| *text == " context\n"));
        // A second file resets hunk state, and payload lines that look like
        // headers stay coloured as additions/deletions.
        assert!(kinds.contains(&("+++ b/b\n", DiffLine::Header)));
        assert!(kinds.contains(&("+++payload\n", DiffLine::Added)));
        assert!(kinds.contains(&("---payload\n", DiffLine::Removed)));
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
            unstaged_diff: String::new(),
            notes: vec![],
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
        run(&["init", "-q"]);
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
        std::fs::write(directory.join("tracked.txt"), "one\ntwo\n").unwrap();
        std::fs::write(directory.join("new.txt"), "untracked\n").unwrap();

        let before = read_repository_bytes(&directory);
        let snapshot = load(&directory, &AtomicBool::new(false)).unwrap();
        assert!(!snapshot.branch.is_empty());
        assert!(
            snapshot.files.iter().any(|file| file.path == "tracked.txt"
                && file.label() == "修改"
                && file.unstaged())
        );
        assert!(
            snapshot
                .files
                .iter()
                .any(|file| file.path == "new.txt" && file.untracked())
        );
        assert!(snapshot.unstaged_diff.contains("+two"));
        assert!(snapshot.staged_diff.is_empty());
        // Loading the panel must not touch the repository (no index lock, no
        // staging, no commits).
        assert_eq!(read_repository_bytes(&directory), before);
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
}
