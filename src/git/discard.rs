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
        bail!("分支、HEAD 或暂存区已变化，请刷新后重新回滚");
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
            bail!("回滚路径必须是仓库内的文件：{path}");
        }
        // Never recurse into untracked directories or submodule working trees.
        if fs::symlink_metadata(root.join(path)).is_ok_and(|metadata| metadata.is_dir()) {
            bail!("不能从文件菜单回滚目录或子模块：{path}");
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
        bail!("文件状态已变化，请刷新后重新回滚");
    }
    if current
        .iter()
        .any(|other| other != file && paths.contains(&other.path.as_str()))
    {
        bail!("重命名前的路径存在其他修改，请先处理该文件后再回滚");
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
