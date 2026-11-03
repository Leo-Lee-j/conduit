//! Branch mutations use explicit refs and optimistic checks against the menu snapshot.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchContext {
    pub current_ref: Option<String>,
    pub head: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub full_ref: String,
    pub oid: String,
    pub remote: bool,
    pub upstream: Option<String>,
    pub upstream_remote: Option<String>,
    pub upstream_ref: Option<String>,
    pub occupied: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegrationState {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl IntegrationState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
            Self::CherryPick => "Cherry-pick",
            Self::Revert => "Revert",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BranchRequest {
    pub expected: BranchContext,
    pub selected: Option<BranchInfo>,
    pub action: BranchAction,
}

#[derive(Clone, Debug)]
pub enum BranchAction {
    Checkout,
    Create {
        name: String,
        checkout: bool,
    },
    Rename(String),
    Delete,
    Track(Option<String>),
    Push {
        remote: String,
        destination: String,
        set_upstream: bool,
    },
    Worktree {
        path: PathBuf,
        new_branch: Option<String>,
    },
    Merge,
    Rebase,
    CheckoutRebase,
    Update {
        rebase: bool,
    },
    CheckoutUpdate {
        rebase: bool,
    },
    Continue(IntegrationState),
    Abort(IntegrationState),
}

/// Common repository scope, including linked worktrees. No Git on the UI thread.
pub fn repository_scope(path: &Path) -> PathBuf {
    let root = worktree_root(path);
    let marker = root.join(".git");
    let git_dir = if marker.is_file() {
        fs::read_to_string(&marker)
            .ok()
            .and_then(|text| {
                text.trim()
                    .strip_prefix("gitdir: ")
                    .map(|dir| root.join(dir))
            })
            .unwrap_or(marker)
    } else {
        marker
    };
    let common = fs::read_to_string(git_dir.join("commondir"))
        .map(|dir| git_dir.join(dir.trim()))
        .unwrap_or(git_dir);
    fs::canonicalize(&common).unwrap_or(common)
}

type Metadata = (
    BranchContext,
    Vec<BranchInfo>,
    Option<IntegrationState>,
    Vec<String>,
    Vec<String>,
);

pub(super) fn metadata(git: &Path, workspace: &Path, cancel: &AtomicBool) -> Result<Metadata> {
    let context = context(git, workspace, cancel)?;
    let refs = run_git(
        git,
        workspace,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)%00%(objectname)%00%(symref)%00%(upstream)%00%(upstream:remotename)%00%(upstream:remoteref)%00%(worktreepath)",
            "refs/heads",
            "refs/remotes",
        ],
        cancel,
    )?;
    let mut info = Vec::new();
    for line in refs.lines() {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 7 || !fields[2].is_empty() {
            continue;
        }
        let (name, remote) = if let Some(name) = fields[0].strip_prefix("refs/heads/") {
            (name, false)
        } else if let Some(name) = fields[0].strip_prefix("refs/remotes/") {
            (name, true)
        } else {
            continue;
        };
        let optional = |value: &str| (!value.is_empty()).then(|| value.to_string());
        info.push(BranchInfo {
            name: name.into(),
            full_ref: fields[0].into(),
            oid: fields[1].into(),
            remote,
            upstream: optional(fields[3]),
            upstream_remote: optional(fields[4]),
            upstream_ref: optional(fields[5]),
            occupied: optional(fields[6]).map(PathBuf::from),
        });
    }
    info.sort_by_key(|branch| context.current_ref.as_ref() != Some(&branch.full_ref));
    info.truncate(MAX_BRANCHES);
    let integration = integration_state(git, workspace, cancel)?;
    let conflicts = if integration.is_some() {
        run_git(
            git,
            workspace,
            &["diff", "--name-only", "--diff-filter=U", "-z"],
            cancel,
        )?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
    } else {
        vec![]
    };
    let remotes = run_git(git, workspace, &["remote"], cancel)?
        .lines()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if self::context(git, workspace, cancel)? != context {
        bail!("读取期间分支或 HEAD 已变化，请刷新后重试");
    }
    Ok((context, info, integration, conflicts, remotes))
}

pub(super) fn context(git: &Path, workspace: &Path, cancel: &AtomicBool) -> Result<BranchContext> {
    // Verify this is a repository before treating a missing HEAD as unborn.
    run_git(git, workspace, &["rev-parse", "--git-dir"], cancel)?;
    Ok(BranchContext {
        current_ref: run_git(git, workspace, &["symbolic-ref", "--quiet", "HEAD"], cancel)
            .ok()
            .map(|s| s.trim().into()),
        head: run_git(git, workspace, &["rev-parse", "--verify", "HEAD"], cancel)
            .ok()
            .map(|s| s.trim().into()),
    })
}

pub(super) fn integration_state(
    git: &Path,
    workspace: &Path,
    cancel: &AtomicBool,
) -> Result<Option<IntegrationState>> {
    let git_dir = run_git(git, workspace, &["rev-parse", "--absolute-git-dir"], cancel)?;
    let dir = Path::new(git_dir.trim());
    Ok(
        if dir.join("rebase-merge").exists() || dir.join("rebase-apply").exists() {
            Some(IntegrationState::Rebase)
        } else if dir.join("MERGE_HEAD").exists() {
            Some(IntegrationState::Merge)
        } else if dir.join("CHERRY_PICK_HEAD").exists() {
            Some(IntegrationState::CherryPick)
        } else if dir.join("REVERT_HEAD").exists() {
            Some(IntegrationState::Revert)
        } else {
            None
        },
    )
}

fn validate_name(git: &Path, workspace: &Path, name: &str, cancel: &AtomicBool) -> Result<()> {
    if name.is_empty() || name.starts_with('-') || name.trim() != name {
        bail!("请填写有效的分支名");
    }
    run_git(
        git,
        workspace,
        &["check-ref-format", &format!("refs/heads/{name}")],
        cancel,
    )
    .context("分支名无效")?;
    Ok(())
}

fn ensure_clean(git: &Path, workspace: &Path, cancel: &AtomicBool) -> Result<()> {
    run_git(git, workspace, &["diff", "--quiet", "--"], cancel)
        .and_then(|_| {
            run_git(
                git,
                workspace,
                &["diff", "--cached", "--quiet", "--"],
                cancel,
            )
        })
        .context("存在未提交的已跟踪文件修改，请先提交或自行暂存到 stash 后重试")?;
    Ok(())
}

fn local(branch: &BranchInfo) -> Result<()> {
    if branch.remote {
        bail!("此操作只适用于本地分支");
    }
    Ok(())
}

fn ensure_unoccupied(branch: &BranchInfo, workspace: &Path, allow_current: bool) -> Result<()> {
    if let Some(path) = &branch.occupied {
        let current = fs::canonicalize(worktree_root(workspace)).ok();
        let occupied = fs::canonicalize(path).ok();
        if !allow_current || current.is_none() || current != occupied {
            bail!("分支正在工作区 {} 中使用", path.display());
        }
    }
    Ok(())
}

fn checkout(
    git: &Path,
    workspace: &Path,
    branch: &BranchInfo,
    cancel: &AtomicBool,
) -> Result<String> {
    ensure_unoccupied(branch, workspace, true)?;
    if branch.remote {
        let remote = remote_for_ref(git, workspace, &branch.full_ref, cancel)?;
        let name = remote.1.strip_prefix("refs/heads/").unwrap_or(&remote.1);
        validate_name(git, workspace, name, cancel)?;
        run_git_with_timeout(
            git,
            workspace,
            &["switch", "--track", "-c", name, &branch.full_ref],
            cancel,
            Duration::from_secs(120),
        )?;
        Ok(name.into())
    } else {
        run_git_with_timeout(
            git,
            workspace,
            &["switch", "--", &branch.name],
            cancel,
            Duration::from_secs(120),
        )?;
        Ok(branch.name.clone())
    }
}

/// Resolve remote refs via configured fetch mappings (remote names may contain '/').
fn remote_for_ref(
    git: &Path,
    workspace: &Path,
    full_ref: &str,
    cancel: &AtomicBool,
) -> Result<(String, String)> {
    let remotes = run_git(git, workspace, &["remote"], cancel)?;
    for remote in remotes.lines() {
        let specs = match run_git(
            git,
            workspace,
            &["config", "--get-all", &format!("remote.{remote}.fetch")],
            cancel,
        ) {
            Ok(specs) => specs,
            Err(_) => continue,
        };
        for spec in specs.lines() {
            let Some((source, destination)) = spec.trim_start_matches('+').split_once(':') else {
                continue;
            };
            if let Some((prefix, suffix)) = destination.split_once('*') {
                if let Some(middle) = full_ref
                    .strip_prefix(prefix)
                    .and_then(|s| s.strip_suffix(suffix))
                {
                    return Ok((remote.into(), source.replace('*', middle)));
                }
            } else if destination == full_ref {
                return Ok((remote.into(), source.into()));
            }
        }
    }
    bail!("找不到该远程引用对应的 fetch 配置，请先检查 remote 配置")
}

pub(super) fn operate_branch(
    workspace: &Path,
    request: BranchRequest,
    cancel: &AtomicBool,
) -> OperationResult {
    let mut progress = String::new();
    let mut created = None;
    let result = (|| -> Result<String> {
        let git = resolve_git().ok_or_else(|| anyhow!("未找到 git.exe"))?;
        let (actual, branches, state, _, remotes) = metadata(&git, workspace, cancel)?;
        if actual != request.expected {
            bail!("分支或 HEAD 已变化，请刷新后重试");
        }
        let recovery = matches!(
            request.action,
            BranchAction::Continue(_) | BranchAction::Abort(_)
        );
        if let Some(state) = state
            && !recovery
        {
            bail!("仓库存在进行中的 {}，请先继续或中止", state.label());
        }
        let selected = request.selected.as_ref();
        if let Some(expected) = selected {
            let actual = branches
                .iter()
                .find(|b| b.full_ref == expected.full_ref)
                .ok_or_else(|| anyhow!("所选分支已不存在，请刷新"))?;
            if actual != expected {
                bail!("所选分支、上游或 worktree 占用已变化，请刷新后重试");
            }
        }
        let selected = || selected.ok_or_else(|| anyhow!("请选择一个已有提交的分支"));
        let run = |args: &[&str]| {
            run_git_with_timeout(&git, workspace, args, cancel, Duration::from_secs(120))
        };
        match request.action {
            BranchAction::Checkout => {
                let name = checkout(&git, workspace, selected()?, cancel)?;
                Ok(format!("已切换到 {name}"))
            }
            BranchAction::Create {
                name,
                checkout: switch,
            } => {
                validate_name(&git, workspace, &name, cancel)?;
                let source = selected()?.oid.as_str();
                if switch {
                    run(&["switch", "-c", &name, source])?;
                } else {
                    run(&["branch", "--no-track", &name, source])?;
                }
                Ok(format!(
                    "已创建分支 {name}{}",
                    if switch { " 并切换" } else { "" }
                ))
            }
            BranchAction::Rename(name) => {
                let branch = selected()?;
                local(branch)?;
                ensure_unoccupied(branch, workspace, true)?;
                validate_name(&git, workspace, &name, cancel)?;
                run(&["branch", "-m", &branch.name, &name])?;
                Ok(format!(
                    "已将 {} 重命名为 {name}（远程分支名不变）",
                    branch.name
                ))
            }
            BranchAction::Delete => {
                let branch = selected()?;
                local(branch)?;
                ensure_unoccupied(branch, workspace, false)?;
                if actual.current_ref.as_ref() == Some(&branch.full_ref) {
                    bail!("不能删除当前分支");
                }
                run(&["branch", "-d", "--", &branch.name])?;
                Ok(format!("已删除本地分支 {}", branch.name))
            }
            BranchAction::Track(upstream) => {
                let branch = selected()?;
                local(branch)?;
                ensure_unoccupied(branch, workspace, true)?;
                if let Some(upstream) = upstream {
                    if !branches.iter().any(|b| b.remote && b.full_ref == upstream) {
                        bail!("上游引用已不存在，请刷新");
                    }
                    run(&[
                        "branch",
                        &format!("--set-upstream-to={upstream}"),
                        "--",
                        &branch.name,
                    ])?;
                } else if branch.upstream.is_some() {
                    run(&["branch", "--unset-upstream", "--", &branch.name])?;
                }
                Ok(format!("已更新 {} 的上游配置", branch.name))
            }
            BranchAction::Push {
                remote,
                destination,
                set_upstream,
            } => {
                let branch = selected()?;
                local(branch)?;
                if remote != "." && !remotes.contains(&remote) {
                    bail!("远程仓库不存在，请刷新");
                }
                validate_name(&git, workspace, &destination, cancel)?;
                let refspec = format!("{}:refs/heads/{destination}", branch.full_ref);
                let mut args = vec![
                    "-c",
                    "credential.interactive=false",
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                ];
                if set_upstream {
                    args.push("--set-upstream");
                }
                args.extend(["--", &remote, &refspec]);
                run(&args)?;
                Ok(format!("已推送 {} → {remote}/{destination}", branch.name))
            }
            BranchAction::Worktree { path, new_branch } => {
                let path = crate::child::normalized_dir(&path);
                let branch = selected()?;
                if !path.is_absolute() {
                    bail!("请选择 worktree 的绝对路径");
                }
                if path.exists() && (!path.is_dir() || fs::read_dir(&path)?.next().is_some()) {
                    bail!("Worktree 目标必须是不存在的目录或空目录");
                }
                let path_text = path.to_str().ok_or_else(|| anyhow!("目录路径无法编码"))?;
                if let Some(name) = &new_branch {
                    validate_name(&git, workspace, name, cancel)?;
                    run(&["worktree", "add", "-b", name, "--", path_text, &branch.oid])?;
                } else {
                    local(branch)?;
                    ensure_unoccupied(branch, workspace, false)?;
                    run(&["worktree", "add", "--", path_text, &branch.name])?;
                }
                created = Some(fs::canonicalize(&path).unwrap_or(path));
                Ok("Worktree 已创建，已加入工作区列表".into())
            }
            BranchAction::Merge | BranchAction::Rebase => {
                if actual.current_ref.is_none() {
                    bail!("请先切回本地分支");
                }
                ensure_clean(&git, workspace, cancel)?;
                let branch = selected()?;
                if matches!(request.action, BranchAction::Merge) {
                    run(&["merge", "--no-edit", &branch.oid])?;
                    Ok(format!("已将 {} 合并到当前分支", branch.name))
                } else {
                    run(&["rebase", &branch.oid])?;
                    Ok(format!("已将当前分支 Rebase 到 {}", branch.name))
                }
            }
            BranchAction::CheckoutRebase => {
                ensure_clean(&git, workspace, cancel)?;
                let head = actual
                    .head
                    .as_deref()
                    .ok_or_else(|| anyhow!("当前分支尚无提交"))?;
                let name = checkout(&git, workspace, selected()?, cancel)?;
                progress = format!("已切换到 {name}；后续 Rebase 未完成");
                run(&["rebase", head])?;
                Ok(format!("已切换到 {name} 并 Rebase 到原分支"))
            }
            BranchAction::Update { rebase } | BranchAction::CheckoutUpdate { rebase } => {
                let branch = selected()?;
                let name = if matches!(request.action, BranchAction::CheckoutUpdate { .. }) {
                    ensure_clean(&git, workspace, cancel)?;
                    let name = checkout(&git, workspace, branch, cancel)?;
                    progress = format!("已切换到 {name}；后续 Update 未完成");
                    name
                } else {
                    local(branch)?;
                    branch.name.clone()
                };
                update(&git, workspace, &name, rebase, cancel)?;
                Ok(format!("已更新分支 {name}"))
            }
            BranchAction::Continue(expected) | BranchAction::Abort(expected) => {
                if state != Some(expected) {
                    bail!("进行中的 Git 操作已变化，请刷新");
                }
                if !matches!(expected, IntegrationState::Merge | IntegrationState::Rebase) {
                    bail!("请在终端处理该操作");
                }
                let command = if expected == IntegrationState::Merge {
                    "merge"
                } else {
                    "rebase"
                };
                let mode = if matches!(request.action, BranchAction::Continue(_)) {
                    "--continue"
                } else {
                    "--abort"
                };
                run(&[command, mode])?;
                Ok(format!(
                    "{} 已{}",
                    expected.label(),
                    if mode == "--continue" {
                        "继续完成"
                    } else {
                        "中止"
                    }
                ))
            }
        }
    })();
    match result {
        Ok(message) => OperationResult {
            retry_completed: false,
            retry_tag: None,
            committed: false,
            message,
            error: None,
            worktree_created: created,
        },
        Err(error) => OperationResult {
            retry_completed: false,
            retry_tag: None,
            committed: false,
            message: if progress.is_empty() {
                "Git 操作未完成，请检查仓库状态".into()
            } else {
                progress
            },
            error: Some(format!("{error:#}")),
            worktree_created: created,
        },
    }
}

fn update(
    git: &Path,
    workspace: &Path,
    name: &str,
    rebase: bool,
    cancel: &AtomicBool,
) -> Result<()> {
    let (context, branches, _, _, _) = metadata(git, workspace, cancel)?;
    let branch = branches
        .iter()
        .find(|b| !b.remote && b.name == name)
        .ok_or_else(|| anyhow!("分支已不存在"))?;
    let remote = branch
        .upstream_remote
        .as_deref()
        .ok_or_else(|| anyhow!("分支未设置有效上游"))?;
    let upstream_ref = branch
        .upstream_ref
        .as_deref()
        .ok_or_else(|| anyhow!("上游配置不完整"))?;
    let upstream = branch
        .upstream
        .as_deref()
        .ok_or_else(|| anyhow!("上游配置不完整"))?;
    let current = context.current_ref.as_ref() == Some(&branch.full_ref);
    ensure_unoccupied(branch, workspace, current)?;
    if current {
        ensure_clean(git, workspace, cancel)?;
    }
    let run = |args: &[&str]| {
        run_git_with_timeout(git, workspace, args, cancel, Duration::from_secs(120))
    };
    if remote != "." {
        run(&[
            "-c",
            "credential.interactive=false",
            "fetch",
            "--no-tags",
            "--",
            remote,
            &format!("{upstream_ref}:{upstream}"),
        ])?;
    }
    let destination = run_git(
        git,
        workspace,
        &["rev-parse", "--verify", &format!("{upstream}^{{commit}}")],
        cancel,
    )?;
    let destination = destination.trim();
    // Fetch may have run hooks or overlapped an external command. Check before mutation.
    let (after_context, after_branches, after_state, _, _) = metadata(git, workspace, cancel)?;
    if after_context != context || after_state.is_some() {
        bail!("Fetch 已完成，但 HEAD 已变化；请刷新后重试");
    }
    let after_branch = after_branches
        .iter()
        .find(|candidate| candidate.full_ref == branch.full_ref);
    if after_branch != Some(branch) {
        bail!("Fetch 已完成，但分支、上游或 Worktree 占用已变化；请刷新后重试");
    }
    if current {
        ensure_clean(git, workspace, cancel)?;
    }
    let actual_oid = run_git(
        git,
        workspace,
        &["rev-parse", "--verify", &branch.full_ref],
        cancel,
    )?;
    if actual_oid.trim() != branch.oid {
        bail!("Fetch 已完成，但所选分支已变化；请刷新后重试");
    }
    if current {
        if rebase {
            run(&["rebase", destination])?;
        } else {
            run(&["merge", "--no-edit", destination])?;
        }
    } else {
        // Updating another branch never checks it out behind the user's back.
        if run_git(
            git,
            workspace,
            &["merge-base", "--is-ancestor", destination, &branch.oid],
            cancel,
        )
        .is_ok()
        {
            return Ok(());
        }
        run_git(
            git,
            workspace,
            &["merge-base", "--is-ancestor", &branch.oid, destination],
            cancel,
        )
        .context("该分支与上游已分叉。请先切换到该分支，再选择 Merge 或 Rebase 更新")?;
        run(&[
            "update-ref",
            "-m",
            "conduit: fast-forward branch",
            &branch.full_ref,
            destination,
            &branch.oid,
        ])?;
    }
    Ok(())
}
