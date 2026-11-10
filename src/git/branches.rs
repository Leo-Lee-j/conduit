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
    Fetch,
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
    RemoveWorktree {
        parent: PathBuf,
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
    let conflicts = {
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
    };
    let remotes = run_git(git, workspace, &["remote"], cancel)?
        .lines()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if self::context(git, workspace, cancel)? != context {
        bail!(conduit::i18n::text(
            "git.branches.branch_or_head_changed_while_reading_refresh"
        ));
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
        bail!(conduit::i18n::text(
            "git.branches.enter_a_valid_branch_name"
        ));
    }
    run_git(
        git,
        workspace,
        &["check-ref-format", &format!("refs/heads/{name}")],
        cancel,
    )
    .context(conduit::i18n::text_static(
        "git.branches.invalid_branch_name",
    ))?;
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
        .context(conduit::i18n::text_static(
            "git.branches.tracked_files_have_uncommitted_changes_commit_or",
        ))?;
    Ok(())
}

fn local(branch: &BranchInfo) -> Result<()> {
    if branch.remote {
        bail!(conduit::i18n::text(
            "git.branches.this_operation_applies_to_local_branches_only"
        ));
    }
    Ok(())
}

fn ensure_unoccupied(branch: &BranchInfo, workspace: &Path, allow_current: bool) -> Result<()> {
    if let Some(path) = &branch.occupied {
        let current = fs::canonicalize(worktree_root(workspace)).ok();
        let occupied = fs::canonicalize(path).ok();
        if !allow_current || current.is_none() || current != occupied {
            bail!(conduit::i18n::format(
                "git.branches.branch_is_in_use_by_workspace",
                &[("arg0", format!("{}", path.display()))]
            ));
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
    bail!(conduit::i18n::text(
        "git.branches.no_fetch_configuration_found_for_this_remote"
    ))
}

pub(super) fn operate_branch(
    workspace: &Path,
    request: BranchRequest,
    cancel: &AtomicBool,
) -> OperationResult {
    let mut progress = String::new();
    let mut created = None;
    let result = (|| -> Result<String> {
        let git = resolve_git()
            .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
        let (actual, branches, state, _, remotes) = metadata(&git, workspace, cancel)?;
        if actual != request.expected {
            bail!(conduit::i18n::text(
                "git.branches.branch_or_head_changed_refresh_and_retry"
            ));
        }
        let recovery = matches!(
            request.action,
            BranchAction::Continue(_) | BranchAction::Abort(_)
        );
        if let Some(state) = state
            && !recovery
        {
            bail!(conduit::i18n::format(
                "git.branches.repository_has_an_ongoing_continue_or_abort",
                &[("arg0", state.label().to_string())]
            ));
        }
        let selected = request.selected.as_ref();
        if let Some(expected) = selected {
            let actual = branches
                .iter()
                .find(|b| b.full_ref == expected.full_ref)
                .ok_or_else(|| {
                    anyhow!(conduit::i18n::text(
                        "git.branches.selected_branch_no_longer_exists_refresh_to"
                    ))
                })?;
            if actual != expected {
                bail!(conduit::i18n::text(
                    "git.branches.selected_branch_upstream_or_worktree_usage_changed"
                ));
            }
        }
        let selected = || {
            selected.ok_or_else(|| {
                anyhow!(conduit::i18n::text(
                    "git.branches.select_a_branch_that_has_commits"
                ))
            })
        };
        let run = |args: &[&str]| {
            run_git_with_timeout(&git, workspace, args, cancel, Duration::from_secs(120))
        };
        match request.action {
            BranchAction::Fetch => {
                run(&["fetch", "--all", "--prune"])?;
                Ok(conduit::i18n::text_static(
                    "git.branches.fetched_all_remotes_and_pruned_stale_remote",
                )
                .into())
            }
            BranchAction::Checkout => {
                let name = checkout(&git, workspace, selected()?, cancel)?;
                Ok(conduit::i18n::format(
                    "git.branches.switched_to",
                    &[("name", name.to_string())],
                ))
            }
            BranchAction::Create {
                name,
                checkout: switch,
            } => {
                validate_name(&git, workspace, &name, cancel)?;
                let source = request
                    .selected
                    .as_ref()
                    .map(|b| b.oid.as_str())
                    .or(actual.head.as_deref())
                    .ok_or_else(|| {
                        anyhow!(conduit::i18n::text(
                            "git.branches.this_repository_has_no_commits_yet"
                        ))
                    })?;
                if switch {
                    run(&["switch", "-c", &name, source])?;
                } else {
                    run(&["branch", "--no-track", &name, source])?;
                }
                Ok(conduit::i18n::format(
                    "git.branches.created_branch",
                    &[
                        ("name", name.to_string()),
                        (
                            "arg0",
                            (if switch {
                                conduit::i18n::text_static("git.branches.and_switched_to_it")
                            } else {
                                ""
                            })
                            .to_string(),
                        ),
                    ],
                ))
            }
            BranchAction::Rename(name) => {
                let branch = selected()?;
                local(branch)?;
                ensure_unoccupied(branch, workspace, true)?;
                validate_name(&git, workspace, &name, cancel)?;
                run(&["branch", "-m", &branch.name, &name])?;
                Ok(conduit::i18n::format(
                    "git.branches.renamed_to_remote_branch_unchanged",
                    &[
                        ("arg0", branch.name.to_string()),
                        ("name", name.to_string()),
                    ],
                ))
            }
            BranchAction::Delete => {
                let branch = selected()?;
                local(branch)?;
                ensure_unoccupied(branch, workspace, false)?;
                if actual.current_ref.as_ref() == Some(&branch.full_ref) {
                    bail!(conduit::i18n::text(
                        "git.branches.cannot_delete_the_current_branch"
                    ));
                }
                run(&["branch", "-d", "--", &branch.name])?;
                Ok(conduit::i18n::format(
                    "git.branches.deleted_local_branch",
                    &[("arg0", branch.name.to_string())],
                ))
            }
            BranchAction::Track(upstream) => {
                let branch = selected()?;
                local(branch)?;
                ensure_unoccupied(branch, workspace, true)?;
                if let Some(upstream) = upstream {
                    if !branches.iter().any(|b| b.remote && b.full_ref == upstream) {
                        bail!(conduit::i18n::text(
                            "git.branches.upstream_reference_no_longer_exists_refresh_to"
                        ));
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
                Ok(conduit::i18n::format(
                    "git.branches.updated_upstream_configuration_for",
                    &[("arg0", branch.name.to_string())],
                ))
            }
            BranchAction::Push {
                remote,
                destination,
                set_upstream,
            } => {
                let branch = selected()?;
                local(branch)?;
                if remote != "." && !remotes.contains(&remote) {
                    bail!(conduit::i18n::text(
                        "git.branches.remote_no_longer_exists_refresh_to_continue"
                    ));
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
                Ok(conduit::i18n::format(
                    "git.branches.pushed",
                    &[
                        ("arg0", branch.name.to_string()),
                        ("remote", remote.to_string()),
                        ("destination", destination.to_string()),
                    ],
                ))
            }
            BranchAction::Worktree { path, new_branch } => {
                let path = crate::child::normalized_dir(&path);
                let branch = selected()?;
                if !path.is_absolute() {
                    bail!(conduit::i18n::text(
                        "git.branches.select_an_absolute_path_for_the_worktree"
                    ));
                }
                if path.exists() && (!path.is_dir() || fs::read_dir(&path)?.next().is_some()) {
                    bail!(conduit::i18n::text(
                        "git.branches.worktree_target_must_be_a_nonexistent_or"
                    ));
                }
                let path_text = path.to_str().ok_or_else(|| {
                    anyhow!(conduit::i18n::text(
                        "git.branches.unable_to_encode_directory_path"
                    ))
                })?;
                if let Some(name) = &new_branch {
                    validate_name(&git, workspace, name, cancel)?;
                    run(&["worktree", "add", "-b", name, "--", path_text, &branch.oid])?;
                } else {
                    local(branch)?;
                    ensure_unoccupied(branch, workspace, false)?;
                    run(&["worktree", "add", "--", path_text, &branch.name])?;
                }
                created = Some(fs::canonicalize(&path).unwrap_or(path));
                Ok(conduit::i18n::text_static(
                    "git.branches.worktree_created_and_added_to_the_workspace",
                )
                .into())
            }
            BranchAction::RemoveWorktree { parent } => {
                let target = fs::canonicalize(worktree_root(workspace))?;
                let parent = fs::canonicalize(parent)?;
                if target == parent
                    || !target.join(".git").is_file()
                    || repository_scope(&target) != repository_scope(&parent)
                {
                    bail!(conduit::i18n::text("workflow.cleanup_invalid"));
                }
                // Git rejects dirty, locked and unregistered worktrees. Never use --force.
                run_git_with_timeout(
                    &git,
                    &parent,
                    &[
                        "worktree",
                        "remove",
                        "--",
                        target.to_str().context("Invalid path")?,
                    ],
                    cancel,
                    Duration::from_secs(120),
                )?;
                Ok(conduit::i18n::text("workflow.cleaned"))
            }
            BranchAction::Merge | BranchAction::Rebase => {
                if actual.current_ref.is_none() {
                    bail!(conduit::i18n::text(
                        "git.branches.switch_back_to_a_local_branch_first"
                    ));
                }
                ensure_clean(&git, workspace, cancel)?;
                let branch = selected()?;
                if matches!(request.action, BranchAction::Merge) {
                    run(&["merge", "--no-edit", &branch.oid])?;
                    Ok(conduit::i18n::format(
                        "git.branches.merged_into_the_current_branch",
                        &[("arg0", branch.name.to_string())],
                    ))
                } else {
                    run(&["rebase", &branch.oid])?;
                    Ok(conduit::i18n::format(
                        "git.branches.rebased_the_current_branch_onto",
                        &[("arg0", branch.name.to_string())],
                    ))
                }
            }
            BranchAction::CheckoutRebase => {
                ensure_clean(&git, workspace, cancel)?;
                let head = actual.head.as_deref().ok_or_else(|| {
                    anyhow!(conduit::i18n::text(
                        "git.branches.current_branch_has_no_commits_yet"
                    ))
                })?;
                let name = checkout(&git, workspace, selected()?, cancel)?;
                progress = conduit::i18n::format(
                    "git.branches.switched_to_subsequent_rebase_incomplete",
                    &[("name", name.to_string())],
                );
                run(&["rebase", head])?;
                Ok(conduit::i18n::format(
                    "git.branches.switched_to_and_rebased_onto_the_original",
                    &[("name", name.to_string())],
                ))
            }
            BranchAction::Update { rebase } | BranchAction::CheckoutUpdate { rebase } => {
                let branch = selected()?;
                let name = if matches!(request.action, BranchAction::CheckoutUpdate { .. }) {
                    ensure_clean(&git, workspace, cancel)?;
                    let name = checkout(&git, workspace, branch, cancel)?;
                    progress = conduit::i18n::format(
                        "git.branches.switched_to_subsequent_update_incomplete",
                        &[("name", name.to_string())],
                    );
                    name
                } else {
                    local(branch)?;
                    branch.name.clone()
                };
                update(&git, workspace, &name, rebase, cancel)?;
                Ok(conduit::i18n::format(
                    "git.branches.updated_branch",
                    &[("name", name.to_string())],
                ))
            }
            BranchAction::Continue(expected) | BranchAction::Abort(expected) => {
                if state != Some(expected) {
                    bail!(conduit::i18n::text(
                        "git.branches.the_ongoing_git_operation_changed_refresh_to"
                    ));
                }
                if !matches!(expected, IntegrationState::Merge | IntegrationState::Rebase) {
                    bail!(conduit::i18n::text(
                        "git.branches.handle_this_operation_in_the_terminal"
                    ));
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
                Ok(conduit::i18n::format(
                    "git.branches.label",
                    &[
                        ("arg0", expected.label().to_string()),
                        (
                            "arg1",
                            (if mode == "--continue" {
                                conduit::i18n::text_static("git.branches.continued_to_completion")
                            } else {
                                conduit::i18n::text_static("git.branches.aborted")
                            })
                            .to_string(),
                        ),
                    ],
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
                conduit::i18n::text_static(
                    "git.branches.git_operation_incomplete_check_repository_status",
                )
                .into()
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
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.branch_no_longer_exists")))?;
    let remote = branch.upstream_remote.as_deref().ok_or_else(|| {
        anyhow!(conduit::i18n::text(
            "git.branches.branch_has_no_valid_upstream"
        ))
    })?;
    let upstream_ref = branch.upstream_ref.as_deref().ok_or_else(|| {
        anyhow!(conduit::i18n::text(
            "git.branches.incomplete_upstream_configuration"
        ))
    })?;
    let upstream = branch.upstream.as_deref().ok_or_else(|| {
        anyhow!(conduit::i18n::text(
            "git.branches.incomplete_upstream_configuration"
        ))
    })?;
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
        bail!(conduit::i18n::text(
            "git.branches.fetch_completed_but_head_changed_refresh_and"
        ));
    }
    let after_branch = after_branches
        .iter()
        .find(|candidate| candidate.full_ref == branch.full_ref);
    if after_branch != Some(branch) {
        bail!(conduit::i18n::text(
            "git.branches.fetch_completed_but_the_branch_upstream_or"
        ));
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
        bail!(conduit::i18n::text(
            "git.branches.fetch_completed_but_the_selected_branch_changed"
        ));
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
        .context(conduit::i18n::text_static(
            "git.branches.this_branch_has_diverged_from_its_upstream",
        ))?;
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
