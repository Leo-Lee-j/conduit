//! Read-only, bounded commit history. Resolve names before reading immutable objects.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    pub reference: Option<String>,
    pub text: String,
    pub author: String,
    pub since: Option<String>,
    pub path: String,
    pub limit: usize,
}
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            reference: None,
            text: String::new(),
            author: String::new(),
            since: None,
            path: String::new(),
            limit: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogCommit {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct History {
    pub commits: Vec<LogCommit>,
    pub tags: Vec<String>,
    pub has_more: bool,
    pub tags_truncated: bool,
}
#[derive(Clone, Debug)]
pub struct CommitDetails {
    pub oid: String,
    pub parent: Option<String>,
    pub message: String,
    pub files: Vec<ChangedFile>,
    pub merge: bool,
}
#[derive(Clone, Debug)]
pub struct CommitDiffRequest {
    pub oid: String,
    pub parent: Option<String>,
}

fn resolve_commit(
    git: &Path,
    workspace: &Path,
    reference: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let expression = format!("{reference}^{{commit}}");
    let output = run_git(
        git,
        workspace,
        &["rev-parse", "--verify", "--end-of-options", &expression],
        cancel,
    )?;
    let oid = output.trim();
    if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(conduit::i18n::text(
            "git.log.git_returned_an_invalid_commit_id"
        ));
    }
    Ok(oid.to_string())
}

pub fn load_history(
    workspace: &Path,
    query: &HistoryQuery,
    cancel: &AtomicBool,
) -> Result<History> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
    let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let workspace = Path::new(root.trim());
    let context = branches::context(&git, workspace, cancel)?;
    let tags = run_git(
        &git,
        workspace,
        &[
            "for-each-ref",
            "--count=1001",
            "--sort=refname",
            "--format=%(refname)",
            "refs/tags",
        ],
        cancel,
    )?;
    let mut tags: Vec<String> = tags.lines().map(str::to_string).collect();
    let tags_truncated = tags.len() > 1000;
    tags.truncate(1000);
    if query.reference.as_deref() == Some("HEAD") && context.head.is_none() {
        return Ok(History {
            commits: vec![],
            tags,
            has_more: false,
            tags_truncated,
        });
    }
    let reference = query
        .reference
        .as_deref()
        .map(|r| resolve_commit(&git, workspace, r, cancel))
        .transpose()?;
    let text = query.text.trim();
    let hash = if (4..=64).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        resolve_commit(&git, workspace, text, cancel).ok()
    } else {
        None
    };
    if let (Some(hash), Some(reference)) = (&hash, &reference)
        && run_git(
            &git,
            workspace,
            &["merge-base", "--is-ancestor", hash, reference],
            cancel,
        )
        .is_err()
    {
        return Ok(History {
            commits: vec![],
            tags,
            has_more: false,
            tags_truncated,
        });
    }
    let count = (query.limit.clamp(1, 5000) + 1).to_string();
    let mut args = vec![
        "--no-optional-locks",
        "--literal-pathspecs",
        "log",
        "--topo-order",
        "--date-order",
        "--decorate=full",
        "-z",
        "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x00%D",
        "--max-count",
        &count,
        "--regexp-ignore-case",
        "--fixed-strings",
    ];
    let author = query.author.trim();
    let grep = format!("--grep={text}");
    let author_arg = format!("--author={author}");
    let since = query.since.as_ref().map(|s| format!("--since={s}"));
    if !text.is_empty() && hash.is_none() {
        args.push(&grep);
    }
    if !author.is_empty() {
        args.push(&author_arg);
    }
    if let Some(since) = &since {
        args.push(since);
    }
    if let Some(hash) = &hash {
        args.extend(["--no-walk", hash]);
    } else if let Some(reference) = &reference {
        args.push(reference);
    } else {
        args.push("--all");
        if context.head.is_some() {
            args.push("HEAD");
        }
    }
    args.push("--");
    if !query.path.trim().is_empty() {
        args.push(query.path.trim());
    }
    let result = run_git_bounded(
        &git,
        workspace,
        &args,
        cancel,
        4 * 1024 * 1024,
        COMMAND_TIMEOUT,
        false,
    )?;
    let fields: Vec<_> = result.text.split_terminator('\0').collect();
    let (rows, remainder) = fields.as_chunks::<7>();
    let mut commits = Vec::new();
    for row in rows {
        commits.push(LogCommit {
            oid: row[0].into(),
            parents: row[1].split_whitespace().map(str::to_string).collect(),
            author: row[2].into(),
            email: row[3].into(),
            timestamp: row[4].parse().unwrap_or_default(),
            subject: row[5].into(),
            refs: row[6]
                .split(", ")
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        });
    }
    if !result.truncated && !remainder.is_empty() {
        bail!(conduit::i18n::text(
            "git.log.incomplete_git_log_output_refresh_and_retry"
        ));
    }
    let has_more = commits.len() > query.limit || result.truncated;
    commits.truncate(query.limit.clamp(1, 5000));
    Ok(History {
        commits,
        tags,
        has_more,
        tags_truncated,
    })
}

pub fn load_commit_details(
    workspace: &Path,
    oid: &str,
    cancel: &AtomicBool,
) -> Result<CommitDetails> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
    let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let workspace = Path::new(root.trim());
    let oid = resolve_commit(&git, workspace, oid, cancel)?;
    let message = run_git(
        &git,
        workspace,
        &["show", "-s", "--format=%P%x00%B", &oid],
        cancel,
    )?;
    let (parents, message) = message
        .split_once('\0')
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.log.incomplete_git_commit_output")))?;
    let parents: Vec<_> = parents.split_whitespace().collect();
    let parent = parents.first().map(|s| s.to_string());
    let mut args = vec![
        "--no-optional-locks",
        "diff-tree",
        "--no-commit-id",
        "--root",
        "-r",
        "--name-status",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "-z",
    ];
    if let Some(parent) = &parent {
        args.push(parent);
    }
    args.extend([&oid, "--"]);
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
            "git.log.this_commit_s_file_list_is_too"
        ));
    }
    let mut fields = output.text.split_terminator('\0');
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let path = fields
            .next()
            .ok_or_else(|| anyhow!(conduit::i18n::text("git.log.incomplete_git_file_list")))?;
        files.push(ChangedFile {
            path: path.into(),
            original: None,
            index: status.chars().next().unwrap_or('M'),
            worktree: ' ',
        });
    }
    Ok(CommitDetails {
        oid,
        parent,
        message: message.trim_end().into(),
        files,
        merge: parents.len() > 1,
    })
}

pub fn load_commit_diff(
    workspace: &Path,
    request: &CommitDiffRequest,
    file: &ChangedFile,
    cancel: &AtomicBool,
) -> Result<FileDiff> {
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.branches.git_exe_not_found")))?;
    let root = run_git(&git, workspace, &["rev-parse", "--show-toplevel"], cancel)?;
    let workspace = Path::new(root.trim());
    let oid = resolve_commit(&git, workspace, &request.oid, cancel)?;
    let parent = request
        .parent
        .as_deref()
        .map(|p| resolve_commit(&git, workspace, p, cancel))
        .transpose()?;
    let mut args = vec!["--no-optional-locks", "--literal-pathspecs"];
    if parent.is_some() {
        args.push("diff");
    } else {
        args.extend(["show", "--format=", "--root"]);
    }
    args.extend([
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-color",
        "--unified=3",
    ]);
    if let Some(parent) = &parent {
        args.push(parent);
    }
    args.extend([&oid, "--", &file.path]);
    let output = run_git_bounded(
        &git,
        workspace,
        &args,
        cancel,
        MAX_DIFF_BYTES,
        COMMAND_TIMEOUT,
        false,
    )?;
    let notes = if output.truncated {
        vec![
            conduit::i18n::text_static("git.log.diff_exceeds_the_display_limit_showing_the").into(),
        ]
    } else {
        vec![]
    };
    Ok(FileDiff {
        patch: output.text,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "conduit-history-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            let repo = Self(root);
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "user.name", "History Tester"]);
            repo.git(&["config", "user.email", "history@example.invalid"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo
        }
        fn git(&self, args: &[&str]) -> String {
            let result = std::process::Command::new(resolve_git().unwrap())
                .current_dir(&self.0)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            String::from_utf8_lossy(&result.stdout).trim().to_string()
        }
        fn write_commit(&self, path: &str, contents: &str, message: &str) -> String {
            fs::write(self.0.join(path), contents).unwrap();
            self.git(&["add", "--", path]);
            self.git(&["commit", "-qm", message]);
            self.git(&["rev-parse", "HEAD"])
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn history_filters_merges_tags_and_immutable_commit_diffs() {
        let _locale = crate::test_support::chinese();
        let repo = Repo::new();
        let cancel = AtomicBool::new(false);
        let root = repo.write_commit("中文 [1].txt", "original\n", "initial message\n\nfull body");
        repo.git(&["tag", "-a", "v1", "-m", "version one"]);
        repo.git(&["switch", "-qc", "feature"]);
        let feature = repo.write_commit("feature.txt", "feature\n", "feature message");
        repo.git(&["switch", "-q", "main"]);
        repo.write_commit("main.txt", "main\n", "main message");
        repo.git(&["merge", "--no-ff", "-m", "merge feature", "feature"]);
        let history = load_history(&repo.0, &HistoryQuery::default(), &cancel).unwrap();
        assert_eq!(history.commits.len(), 4);
        assert_eq!(history.commits[0].parents.len(), 2);
        assert_eq!(history.commits[0].subject, "merge feature");
        assert_eq!(history.tags, vec!["refs/tags/v1"]);
        assert!(
            history.commits[0]
                .refs
                .iter()
                .any(|r| r.contains("refs/heads/main"))
        );
        let page = load_history(
            &repo.0,
            &HistoryQuery {
                limit: 2,
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(page.commits.len(), 2);
        assert!(page.has_more);
        let feature_history = load_history(
            &repo.0,
            &HistoryQuery {
                reference: Some("refs/heads/feature".into()),
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(feature_history.commits.len(), 2);
        assert_eq!(feature_history.commits[0].oid, feature);
        let hash = load_history(
            &repo.0,
            &HistoryQuery {
                text: feature[..9].into(),
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(hash.commits.len(), 1);
        assert_eq!(hash.commits[0].oid, feature);
        let message = load_history(
            &repo.0,
            &HistoryQuery {
                text: "FEATURE MESSAGE".into(),
                author: "History Tester".into(),
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(message.commits.len(), 1);
        let path = load_history(
            &repo.0,
            &HistoryQuery {
                path: "中文 [1].txt".into(),
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(path.commits.len(), 1);
        assert_eq!(path.commits[0].oid, root);
        let tag = load_history(
            &repo.0,
            &HistoryQuery {
                reference: Some("refs/tags/v1".into()),
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(tag.commits.len(), 1);
        assert_eq!(tag.commits[0].oid, root);
        let details = load_commit_details(&repo.0, &root, &cancel).unwrap();
        assert!(details.parent.is_none());
        assert!(details.message.contains("full body"));
        assert_eq!(details.files[0].path, "中文 [1].txt");
        fs::write(repo.0.join("中文 [1].txt"), "uncommitted working tree\n").unwrap();
        let diff = load_commit_diff(
            &repo.0,
            &CommitDiffRequest {
                oid: root,
                parent: None,
            },
            &details.files[0],
            &cancel,
        )
        .unwrap();
        assert!(diff.patch.contains("+original"));
        assert!(!diff.patch.contains("uncommitted"));
        let merged = load_commit_details(&repo.0, &history.commits[0].oid, &cancel).unwrap();
        assert!(merged.merge);
        assert_eq!(merged.files.len(), 1);
        assert_eq!(merged.files[0].path, "feature.txt");
        fs::create_dir_all(repo.0.join("nested")).unwrap();
        let nested = load_commit_diff(
            &repo.0.join("nested"),
            &CommitDiffRequest {
                oid: merged.oid.clone(),
                parent: merged.parent.clone(),
            },
            &merged.files[0],
            &cancel,
        )
        .unwrap();
        assert!(nested.patch.contains("+feature"));
        let filtered = load_history(
            &repo.0.join("nested"),
            &HistoryQuery {
                path: "feature.txt".into(),
                ..Default::default()
            },
            &cancel,
        )
        .unwrap();
        assert!(!filtered.commits.is_empty());
        assert!(
            load_history(
                &repo.0,
                &HistoryQuery {
                    reference: Some("--all".into()),
                    ..Default::default()
                },
                &cancel
            )
            .is_err()
        );
    }

    #[test]
    fn empty_history_and_cancelled_reads_are_explicit() {
        let _locale = crate::test_support::chinese();
        let repo = Repo::new();
        for reference in [None, Some("HEAD".into())] {
            let result = load_history(
                &repo.0,
                &HistoryQuery {
                    reference,
                    ..Default::default()
                },
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(result.commits.is_empty());
            assert!(!result.has_more);
        }
        assert!(load_history(&repo.0, &HistoryQuery::default(), &AtomicBool::new(true)).is_err());
    }
}
