//! Commit a selection from the index while preserving the real index byte for byte.
use super::*;
use std::{collections::HashSet, io::Write};

struct CommitIndex {
    index: PathBuf,
    paths: PathBuf,
    lock: PathBuf,
}

impl Drop for CommitIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.index);
        let _ = fs::remove_file(self.index.with_extension("index.lock"));
        let _ = fs::remove_file(&self.paths);
        let _ = fs::remove_file(&self.lock);
    }
}

pub(super) fn commit(
    git: &Path,
    workspace: &Path,
    target: &CommitTarget,
    files: &[String],
    message: &str,
    release_tag: Option<&str>,
    cancel: &AtomicBool,
) -> Result<()> {
    if files.is_empty() {
        bail!(conduit::i18n::text(
            "git_ui.select_staged_files_to_commit_dbd187"
        ));
    }
    let root = run_git(git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let root = Path::new(root.trim());
    let index = run_git(
        git,
        root,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
        cancel,
    )?;
    let index = PathBuf::from(index.trim());
    let unique = format!(
        "conduit-commit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let lock = index.with_file_name("index.lock");
    // Hold the ordinary index lock for the entire operation. External index
    // writers must retry; the commit itself uses a separate index and lock.
    let lock_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .context(conduit::i18n::text_static(
            "git.selected.the_staging_area_is_in_use_by",
        ))?;
    drop(lock_file);
    let temporary = CommitIndex {
        index: index.with_file_name(format!("{unique}.index")),
        paths: index.with_file_name(format!("{unique}.paths")),
        lock,
    };
    if commit_target(git, root, cancel)? != *target {
        bail!(conduit::i18n::text(
            "git.selected.branch_head_or_staging_area_changed_refresh"
        ));
    }
    let names = run_git(
        git,
        root,
        &[
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
        ],
        cancel,
    )?;
    if names.len() >= MAX_COMMAND_BYTES {
        bail!(conduit::i18n::text(
            "git.selected.staged_file_list_exceeds_the_limit_commit"
        ));
    }
    let staged: HashSet<&str> = names.split('\0').filter(|name| !name.is_empty()).collect();
    let selected: HashSet<&str> = files.iter().map(String::as_str).collect();
    if selected.iter().any(|path| !staged.contains(path)) {
        bail!(conduit::i18n::text(
            "git.selected.selected_files_staged_state_changed_refresh_and"
        ));
    }
    fs::copy(&index, &temporary.index)?;
    let excluded: Vec<_> = staged.difference(&selected).copied().collect();
    if !excluded.is_empty() {
        let mut paths = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary.paths)?;
        for path in excluded {
            paths.write_all(path.as_bytes())?;
            paths.write_all(&[0])?;
        }
        drop(paths);
        let from = format!("--pathspec-from-file={}", temporary.paths.display());
        let mut args = vec!["--literal-pathspecs"];
        if target.head.is_some() {
            args.extend(["restore", "--staged", "--source=HEAD"]);
        } else {
            args.extend(["rm", "--cached", "-r", "--ignore-unmatch"]);
        }
        args.extend([&from, "--pathspec-file-nul"]);
        run(git, root, &args, &temporary.index, cancel)?;
    }
    if let Some(tag) = release_tag {
        let tree = run(git, root, &["write-tree"], &temporary.index, cancel)?;
        validate_conduit_release(git, root, tag, tree.trim(), cancel)?;
    }
    // Revalidate HEAD/index after preparing the selection, before creating a commit.
    if commit_target(git, root, cancel)? != *target {
        bail!(conduit::i18n::text(
            "git.selected.branch_head_or_staging_area_changed_refresh"
        ));
    }
    // A worktree edit can arrive without changing HEAD or the index. Never
    // commit an older staged portion of a selected file as a complete file.
    let unstaged = run_git(
        git,
        root,
        &[
            "--no-optional-locks",
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
        ],
        cancel,
    )?;
    if unstaged.len() >= MAX_COMMAND_BYTES
        || unstaged.split('\0').any(|path| selected.contains(path))
    {
        bail!(conduit::i18n::text(
            "git.selected.files_have_unstaged_changes"
        ));
    }
    run(
        git,
        root,
        &["commit", "-m", message],
        &temporary.index,
        cancel,
    )?;
    Ok(())
}

fn run(
    git: &Path,
    root: &Path,
    args: &[&str],
    index: &Path,
    cancel: &AtomicBool,
) -> Result<String> {
    run_git_with_environment(
        git,
        root,
        args,
        cancel,
        Some(index),
        CommandLimits {
            limit: MAX_COMMAND_BYTES,
            timeout: Duration::from_secs(120),
            stop_when_full: false,
        },
    )
    .map(|output| output.text)
}
