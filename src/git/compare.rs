//! Bounded, read-only branch comparisons. Refs are resolved once to commit IDs.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonKind {
    Branches,
    WorkingTree,
}

#[derive(Clone, Debug)]
pub struct ComparisonRequest {
    pub selected: BranchInfo,
    pub expected: BranchContext,
    pub kind: ComparisonKind,
}

#[derive(Clone, Debug)]
pub struct Comparison {
    pub request: ComparisonRequest,
    pub files: Vec<ChangedFile>,
    pub commits: String,
    pub notes: Vec<String>,
}

pub fn load_comparison(
    workspace: &Path,
    request: ComparisonRequest,
    cancel: &AtomicBool,
) -> Result<Comparison> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
    let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let workspace = Path::new(root.trim());
    if branches::context(&git, workspace, cancel)? != request.expected {
        bail!(conduit::i18n::text(
            "git.compare.head_changed_close_and_reopen_the_comparison"
        ));
    }
    let head = request.expected.head.as_deref().ok_or_else(|| {
        anyhow!(conduit::i18n::text(
            "git.branches.current_branch_has_no_commits_yet"
        ))
    })?;
    let mut args = vec![
        "--no-optional-locks",
        "diff",
        "--name-status",
        "--no-ext-diff",
        "--no-textconv",
        "-z",
        "--no-renames",
        &request.selected.oid,
    ];
    if request.kind == ComparisonKind::Branches {
        args.push(head);
    }
    args.push("--");
    let output = run_git_bounded(
        &git,
        workspace,
        &args,
        cancel,
        MAX_COMMAND_BYTES,
        COMMAND_TIMEOUT,
        false,
    )?;
    if output.truncated {
        bail!(conduit::i18n::text(
            "git.compare.comparison_file_list_is_too_large_narrow"
        ));
    }
    let mut fields = output.text.split('\0');
    let mut files = Vec::new();
    while let Some(status) = fields.next().filter(|s| !s.is_empty()) {
        let Some(path) = fields.next() else {
            bail!(conduit::i18n::text(
                "git.compare.incomplete_git_comparison_output"
            ));
        };
        files.push(ChangedFile {
            path: path.into(),
            original: None,
            index: status.chars().next().unwrap_or('M'),
            worktree: ' ',
        });
    }
    let mut notes = Vec::new();
    if request.kind == ComparisonKind::WorkingTree {
        let untracked = run_git_bounded(
            &git,
            workspace,
            &["ls-files", "--others", "--exclude-standard", "-z"],
            cancel,
            MAX_COMMAND_BYTES,
            COMMAND_TIMEOUT,
            false,
        )?;
        if untracked.truncated {
            bail!(conduit::i18n::text(
                "git.compare.untracked_file_list_is_too_large_view"
            ));
        }
        for path in untracked.text.split('\0').filter(|s| !s.is_empty()) {
            if let Some(file) = files.iter_mut().find(|file| file.path == path) {
                // Git reports deletion when the selected branch tracks this path
                // but the current index does not. Compare its actual disk content.
                file.index = 'M';
                file.worktree = '?';
            } else {
                files.push(ChangedFile {
                    path: path.into(),
                    original: None,
                    index: '?',
                    worktree: '?',
                });
            }
        }
        notes.push(
            conduit::i18n::text_static(
                "git.compare.working_tree_includes_staged_unstaged_and_untracked",
            )
            .into(),
        );
    }
    if files.len() > MAX_FILES {
        bail!(conduit::i18n::format(
            "git.compare.comparison_exceeds_files_narrow_the_scope_in",
            &[("MAX_FILES", format!("{}", MAX_FILES))]
        ));
    }
    let commits = if request.kind == ComparisonKind::Branches {
        let range = format!("{}...{head}", request.selected.oid);
        let output = run_git_bounded(
            &git,
            workspace,
            &[
                "--no-pager",
                "log",
                "--left-right",
                "--no-decorate",
                "--format=%m %h %s",
                "-n",
                "200",
                &range,
                "--",
            ],
            cancel,
            MAX_COMMAND_BYTES,
            COMMAND_TIMEOUT,
            false,
        )?;
        notes.push(
            conduit::i18n::text_static("git.compare.only_on_selected_branch_only_on_current")
                .into(),
        );
        output.text
    } else {
        String::new()
    };
    if branches::context(&git, workspace, cancel)? != request.expected {
        bail!(conduit::i18n::text(
            "git.compare.head_changed_while_reading_reopen_the_comparison"
        ));
    }
    Ok(Comparison {
        request,
        files,
        commits,
        notes,
    })
}

pub fn load_comparison_diff(
    workspace: &Path,
    request: &ComparisonRequest,
    file: &ChangedFile,
    cancel: &AtomicBool,
) -> Result<FileDiff> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
    let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let workspace = Path::new(root.trim());
    if branches::context(&git, workspace, cancel)? != request.expected {
        bail!(conduit::i18n::text(
            "git.compare.head_changed_reopen_the_comparison"
        ));
    }
    if request.kind == ComparisonKind::WorkingTree && file.worktree == '?' {
        return untracked_diff(&git, workspace, request, file, cancel);
    }
    let context = format!("--unified={FULL_FILE_CONTEXT_LINES}");
    let mut args = vec![
        "--no-optional-locks",
        "--no-pager",
        "-c",
        "core.quotepath=false",
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        &context,
        &request.selected.oid,
    ];
    if request.kind == ComparisonKind::Branches {
        args.push(request.expected.head.as_deref().ok_or_else(|| {
            anyhow!(conduit::i18n::text(
                "git.branches.current_branch_has_no_commits_yet"
            ))
        })?);
    }
    args.extend(["--", &file.path]);
    let output = run_git_bounded(
        &git,
        workspace,
        &args,
        cancel,
        MAX_FILE_DIFF_BYTES,
        COMMAND_TIMEOUT,
        true,
    )?;
    if output.truncated {
        bail!(conduit::i18n::text(
            "git.compare.file_diff_exceeds_4_mib_and_cannot"
        ));
    }
    Ok(FileDiff {
        patch: output.text,
        notes: vec![],
    })
}

fn untracked_diff(
    git: &Path,
    workspace: &Path,
    request: &ComparisonRequest,
    file: &ChangedFile,
    cancel: &AtomicBool,
) -> Result<FileDiff> {
    let content = read_file_content(workspace, &file.path)?;
    if content.truncated {
        bail!(conduit::i18n::text(
            "git.compare.untracked_file_exceeds_4_mib_and_cannot"
        ));
    }
    if file.untracked() {
        return Ok(FileDiff {
            patch: added_file_patch(&file.path, content.text.as_deref()),
            notes: vec![],
        });
    }
    let object = format!("{}:{}", request.selected.oid, file.path);
    let source = run_git_bounded(
        git,
        workspace,
        &["show", &object],
        cancel,
        MAX_FILE_DIFF_BYTES,
        COMMAND_TIMEOUT,
        true,
    )?;
    if source.truncated {
        bail!(conduit::i18n::text(
            "git.compare.file_in_selected_branch_exceeds_4_mib"
        ));
    }
    if content.text.is_none() || source.text.contains(['\0', '\u{fffd}']) {
        return Ok(FileDiff {
            patch: format!(
                "diff --git a/{0} b/{0}\nBinary files a/{0} and b/{0} differ\n",
                file.path
            ),
            notes: vec![],
        });
    }
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let scratch_path =
        std::env::temp_dir().join(format!("conduit-compare-{}-{nonce}", std::process::id()));
    use std::io::Write as _;
    let mut scratch_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&scratch_path)?;
    let scratch = Scratch(scratch_path);
    scratch_file.write_all(source.text.as_bytes())?;
    drop(scratch_file);
    let actual = crate::child::normalized_dir(&fs::canonicalize(workspace.join(&file.path))?);
    let context = format!("--unified={FULL_FILE_CONTEXT_LINES}");
    let output = run_git_bounded(
        git,
        workspace,
        &[
            "--no-pager",
            "diff",
            "--no-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            &context,
            "--",
            scratch.0.to_str().ok_or_else(|| {
                anyhow!(conduit::i18n::text(
                    "git.compare.unable_to_encode_temporary_path"
                ))
            })?,
            actual.to_str().ok_or_else(|| {
                anyhow!(conduit::i18n::text(
                    "git.compare.unable_to_encode_file_path"
                ))
            })?,
        ],
        cancel,
        MAX_FILE_DIFF_BYTES,
        COMMAND_TIMEOUT,
        true,
    )?;
    if output.truncated {
        bail!(conduit::i18n::text(
            "git.compare.file_diff_exceeds_4_mib_and_cannot"
        ));
    }
    let patch = output
        .text
        .lines()
        .map(|line| {
            if line.starts_with("diff --git ") {
                format!("diff --git a/{0} b/{0}", file.path)
            } else if line.starts_with("--- ") {
                format!("--- a/{}", file.path)
            } else if line.starts_with("+++ ") {
                format!("+++ b/{}", file.path)
            } else {
                line.into()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(FileDiff {
        patch,
        notes: vec![],
    })
}
