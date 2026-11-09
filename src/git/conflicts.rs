//! Review immutable index stages, then resolve exactly that reviewed conflict.
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

mod content;
mod plan;
pub use plan::{Decision, MergeBlock, MergePlan};
const TEXT_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stage {
    pub mode: String,
    pub oid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictSnapshot {
    pub root: PathBuf,
    pub path: String,
    pub context: BranchContext,
    pub integration: Option<IntegrationState>,
    pub incoming: String,
    pub stages: [Option<Stage>; 3],
    worktree: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct ConflictFile {
    pub snapshot: ConflictSnapshot,
    pub plan: Option<MergePlan>,
    pub note: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ResolutionContent {
    Ours,
    Theirs,
    Merged(String),
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub expected: ConflictSnapshot,
    pub content: ResolutionContent,
}

fn checked_path(root: &Path, path: &str) -> Result<PathBuf> {
    if path.is_empty()
        || path.contains(['\0', ':'])
        || path.split(['/', '\\']).any(|part| {
            part.is_empty() || part == "." || part == ".." || part.eq_ignore_ascii_case(".git")
        })
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!(conduit::i18n::text("git_conflicts.invalid_path"));
    }
    let mut full = root.to_path_buf();
    for part in Path::new(path).components() {
        full.push(part);
        match fs::symlink_metadata(&full) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        bail!(conduit::i18n::text("git_conflicts.special_file"));
                    }
                }
                if meta.file_type().is_symlink() {
                    bail!(conduit::i18n::text("git_conflicts.special_file"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if full.is_dir() {
        bail!(conduit::i18n::text("git_conflicts.special_file"));
    }
    Ok(full)
}

fn fingerprint(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut hash = Sha256::new();
    let mut bytes = [0; READ_CHUNK];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(Some(hash.finalize().to_vec()))
}

fn snapshot(
    git: &Path,
    workspace: &Path,
    path: &str,
    cancel: &AtomicBool,
) -> Result<ConflictSnapshot> {
    let root = run_git(git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let root = PathBuf::from(root.trim());
    let full = checked_path(&root, path)?;
    let listing = run_git_bounded(
        git,
        &root,
        &[
            "--literal-pathspecs",
            "ls-files",
            "--unmerged",
            "-z",
            "--",
            path,
        ],
        cancel,
        MAX_COMMAND_BYTES,
        COMMAND_TIMEOUT,
        false,
    )?;
    if listing.truncated {
        bail!(conduit::i18n::text("git_conflicts.too_large"));
    }
    let mut stages: [Option<Stage>; 3] = [None, None, None];
    for entry in listing.text.split('\0').filter(|s| !s.is_empty()) {
        let (header, name) = entry
            .split_once('\t')
            .ok_or_else(|| anyhow!("Invalid index entry"))?;
        if name != path {
            bail!(conduit::i18n::text("git_conflicts.stale"));
        }
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 {
            bail!("Invalid index stage");
        }
        let index: usize = fields[2].parse()?;
        if !(1..=3).contains(&index) {
            bail!("Invalid index stage");
        }
        stages[index - 1] = Some(Stage {
            mode: fields[0].into(),
            oid: fields[1].into(),
        });
    }
    if stages.iter().all(Option::is_none) {
        bail!(conduit::i18n::text("git_conflicts.no_longer_conflicted"));
    }
    let integration = branches::integration_state(git, &root, cancel)?;
    let reference = match integration {
        Some(IntegrationState::Merge) => "MERGE_HEAD",
        Some(IntegrationState::Rebase) => "REBASE_HEAD",
        Some(IntegrationState::CherryPick) => "CHERRY_PICK_HEAD",
        Some(IntegrationState::Revert) => "REVERT_HEAD",
        None => "HEAD",
    };
    let incoming = run_git(git, &root, &["rev-parse", "--verify", reference], cancel)?;
    Ok(ConflictSnapshot {
        context: branches::context(git, &root, cancel)?,
        root,
        path: path.into(),
        integration,
        incoming: incoming.trim().into(),
        stages,
        worktree: fingerprint(&full)?,
    })
}

fn text_blob(
    git: &Path,
    root: &Path,
    stage: &Option<Stage>,
    cancel: &AtomicBool,
) -> Result<String> {
    let Some(stage) = stage else {
        return Ok(String::new());
    };
    if !matches!(stage.mode.as_str(), "100644" | "100755") {
        bail!(conduit::i18n::text("git_conflicts.special_file"));
    }
    content::read_text_object(git, root, &stage.oid, cancel)
}

pub fn load(workspace: &Path, path: &str, cancel: &AtomicBool) -> Result<ConflictFile> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
    let expected = snapshot(&git, workspace, path, cancel)?;
    let text = (|| {
        let base = text_blob(&git, &expected.root, &expected.stages[0], cancel)?;
        let ours = text_blob(&git, &expected.root, &expected.stages[1], cancel)?;
        let theirs = text_blob(&git, &expected.root, &expected.stages[2], cancel)?;
        // Existence conflicts require an explicit keep/delete decision.
        if expected.stages[1].is_none() || expected.stages[2].is_none() {
            bail!(conduit::i18n::text("git_conflicts.deleted_side"));
        }
        let plan = MergePlan::build(&git, &expected.root, &base, &ours, &theirs, cancel)?;
        let current = content::read_worktree(&git, &expected, cancel)?;
        if content::is_original_merge(&git, &expected, [&base, &ours, &theirs], &current, cancel)? {
            Ok(plan)
        } else {
            MergePlan::preserving_worktree(base, ours, theirs, current)
        }
    })();
    let (plan, note) = match text {
        Ok(plan) => (Some(plan), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    if snapshot(&git, workspace, path, cancel)? != expected {
        bail!(conduit::i18n::text("git_conflicts.stale"));
    }
    Ok(ConflictFile {
        snapshot: expected,
        plan,
        note,
    })
}

/// Private index plus the ordinary lock prevents other Git writers from staging
/// a partially resolved file. On failure the original worktree is restored only
/// if nobody edited it since our write.
pub(super) fn resolve(
    git: &Path,
    workspace: &Path,
    resolution: Resolution,
    cancel: &AtomicBool,
) -> Result<()> {
    let expected = &resolution.expected;
    if snapshot(git, workspace, &expected.path, cancel)? != *expected {
        bail!(conduit::i18n::text("git_conflicts.stale"));
    }
    if expected
        .stages
        .iter()
        .flatten()
        .any(|stage| !matches!(stage.mode.as_str(), "100644" | "100755"))
    {
        bail!(conduit::i18n::text("git_conflicts.special_file"));
    }
    let index = run_git(
        git,
        &expected.root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        cancel,
    )?;
    let index = PathBuf::from(index.trim());
    let lock = index.with_file_name("index.lock");
    let lock_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .context(conduit::i18n::text_static(
            "git.selected.the_staging_area_is_in_use_by",
        ))?;
    drop(lock_file);
    let mut guard = IndexLock {
        path: lock,
        owned: true,
    };
    let scratch = Scratch::new_in(index.parent().unwrap(), "resolve")?;
    let alternate = scratch.0.join("index");
    fs::copy(&index, &alternate)?;
    if snapshot(git, workspace, &expected.path, cancel)? != *expected {
        bail!(conduit::i18n::text("git_conflicts.stale"));
    }
    let full = checked_path(&expected.root, &expected.path)?;
    let backup = scratch.0.join("original");
    if expected.worktree.is_some() {
        fs::copy(&full, &backup)?;
    }
    let run = |args: &[&str]| {
        run_git_with_environment(
            git,
            &expected.root,
            args,
            cancel,
            Some(&alternate),
            CommandLimits {
                limit: MAX_COMMAND_BYTES,
                timeout: COMMAND_TIMEOUT,
                stop_when_full: false,
            },
        )
    };
    let desired_mode = match &resolution.content {
        ResolutionContent::Ours => expected.stages[1].as_ref().map(|stage| stage.mode.as_str()),
        ResolutionContent::Theirs => expected.stages[2].as_ref().map(|stage| stage.mode.as_str()),
        ResolutionContent::Merged(_) => Some(merged_mode(&expected.stages)?),
    };
    let mut written = None;
    let result = (|| -> Result<()> {
        let mut attempted_write = false;
        let write_result = (|| -> Result<()> {
            match resolution.content {
                ResolutionContent::Merged(text) => {
                    if text.len() > TEXT_LIMIT || text.contains('\0') {
                        bail!(conduit::i18n::text("git_conflicts.too_large"));
                    }
                    if has_conflict_markers(&text, content::marker_size(git, expected, cancel)?) {
                        bail!(conduit::i18n::text("git_conflicts.markers"));
                    }
                    let bytes = content::to_worktree(git, expected, &scratch.0, &text, cancel)?;
                    if fingerprint(&full)? != expected.worktree {
                        bail!(conduit::i18n::text("git_conflicts.stale"));
                    }
                    attempted_write = true;
                    fs::write(&full, bytes)?;
                }
                ref side => {
                    let ours = matches!(side, ResolutionContent::Ours);
                    if expected.stages[if ours { 1 } else { 2 }].is_some() {
                        attempted_write = true;
                        run(&[
                            "--literal-pathspecs",
                            "checkout",
                            if ours { "--ours" } else { "--theirs" },
                            "--",
                            &expected.path,
                        ])?;
                    } else if full.exists() {
                        attempted_write = true;
                        fs::remove_file(&full)?;
                    }
                }
            }
            Ok(())
        })();
        if attempted_write {
            written = Some(fingerprint(&full)?);
        }
        write_result?;
        run(&["--literal-pathspecs", "add", "--", &expected.path])?;
        if let Some(mode) = desired_mode {
            run(&[
                "--literal-pathspecs",
                "update-index",
                if mode == "100755" {
                    "--chmod=+x"
                } else {
                    "--chmod=-x"
                },
                "--",
                &expected.path,
            ])?;
        }
        if fingerprint(&full)? != written.clone().unwrap_or_else(|| expected.worktree.clone()) {
            bail!(conduit::i18n::text("git_conflicts.stale"));
        }
        let mut current = snapshot(git, workspace, &expected.path, cancel)?;
        current.worktree = expected.worktree.clone();
        if current != *expected {
            bail!(conduit::i18n::text("git_conflicts.stale"));
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&guard.path)?;
        std::io::copy(&mut File::open(&alternate)?, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        drop(output);
        fs::rename(&guard.path, &index)?;
        guard.owned = false;
        Ok(())
    })();
    if result.is_err()
        && let Some(written) = written
        && fingerprint(&full)? == written
    {
        if backup.exists() {
            fs::copy(backup, &full)?;
        } else if full.exists() {
            fs::remove_file(&full)?;
        }
    }
    result
}

fn merged_mode(stages: &[Option<Stage>; 3]) -> Result<&str> {
    let [base, ours, theirs] = stages
        .each_ref()
        .map(|stage| stage.as_ref().map(|stage| stage.mode.as_str()));
    let (Some(ours), Some(theirs)) = (ours, theirs) else {
        bail!(conduit::i18n::text("git_conflicts.deleted_side"));
    };
    if ours == theirs || Some(theirs) == base {
        Ok(ours)
    } else if Some(ours) == base {
        Ok(theirs)
    } else {
        // Add/add can disagree on mode with no ancestor to disambiguate it.
        bail!(conduit::i18n::text("git_conflicts.mode_conflict"))
    }
}

pub fn has_conflict_markers(text: &str, marker_size: usize) -> bool {
    // A separator alone is valid content (e.g. a Markdown Setext heading).
    // Opening, ancestor and closing tokens also catch partially removed
    // conflicts. Require a token boundary so `<<<<<<<operator` is not one.
    // Accept the default width too, if attributes changed after the merge.
    let minimum = marker_size.clamp(1, 7);
    text.lines().any(|line| {
        let bytes = line.as_bytes();
        let Some(symbol @ (b'<' | b'|' | b'>')) = bytes.first() else {
            return false;
        };
        let width = bytes.iter().take_while(|byte| *byte == symbol).count();
        width >= minimum && (width == bytes.len() || bytes[width] == b' ')
    })
}

struct IndexLock {
    path: PathBuf,
    owned: bool,
}
impl Drop for IndexLock {
    fn drop(&mut self) {
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}
struct Scratch(PathBuf);
impl Scratch {
    fn new_in(parent: &Path, name: &str) -> Result<Self> {
        let path = parent.join(format!(
            "conduit-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
pub(crate) mod tests;
