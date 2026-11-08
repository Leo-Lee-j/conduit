//! Restore exactly the reviewed file, including its index entry and rename source.
use super::*;

pub(super) fn discard_file(
    git: &Path,
    workspace: &Path,
    file: &ChangedFile,
    target: &CommitTarget,
    cancel: &AtomicBool,
) -> Result<()> {
    if commit_target(git, workspace, cancel)? != *target {
        bail!(conduit::i18n::text(
            "git.discard.branch_head_or_staging_area_changed_refresh"
        ));
    }
    let root = run_git(git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let root = Path::new(root.trim());
    let mut paths = vec![file.path.as_str()];
    if (file.index == 'R' || file.worktree == 'R')
        && let Some(original) = file.original.as_deref()
    {
        paths.push(original);
    }
    for path in &paths {
        if path.is_empty()
            || path.contains(['\0', ':'])
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || path
                .split(['/', '\\'])
                .any(|c| c == ".." || c == "." || c.is_empty())
        {
            bail!(conduit::i18n::format(
                "git.discard.discard_path_must_be_a_file_inside",
                &[("path", path.to_string())]
            ));
        }
        // Never recurse into untracked directories or submodule working trees.
        if fs::symlink_metadata(root.join(path)).is_ok_and(|metadata| metadata.is_dir()) {
            bail!(conduit::i18n::format(
                "git.discard.cannot_discard_directories_or_submodules_from_the",
                &[("path", path.to_string())]
            ));
        }
    }
    let status = run_git(
        git,
        root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ],
        cancel,
    )?;
    let current = parse_status(status.as_bytes());
    if !current.contains(file) {
        bail!(conduit::i18n::text(
            "git.discard.file_status_changed_refresh_before_discarding_again"
        ));
    }
    if current
        .iter()
        .any(|other| other != file && paths.contains(&other.path.as_str()))
    {
        bail!(conduit::i18n::text(
            "git.discard.the_original_path_of_the_renamed_file"
        ));
    }
    if file.untracked() {
        run_git(
            git,
            root,
            &["--literal-pathspecs", "clean", "-f", "--", &file.path],
            cancel,
        )?;
    } else if target.head.is_some() {
        let mut args = vec![
            "--literal-pathspecs",
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "--",
        ];
        args.extend(paths);
        run_git(git, root, &args, cancel)?;
    } else {
        // Before the first commit every index entry is an addition.
        run_git(
            git,
            root,
            &[
                "--literal-pathspecs",
                "rm",
                "--cached",
                "-f",
                "--",
                &file.path,
            ],
            cancel,
        )?;
        run_git(
            git,
            root,
            &["--literal-pathspecs", "clean", "-f", "--", &file.path],
            cancel,
        )?;
    }
    Ok(())
}
