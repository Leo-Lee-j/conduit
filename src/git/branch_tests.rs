// Included in the existing real-repository fixture module.
fn branch_request(repo: &TestRepository, name: &str, action: BranchAction) -> BranchRequest {
    let metadata = repo.metadata();
    BranchRequest {
        expected: metadata.context,
        selected: Some(
            metadata
                .branch_info
                .into_iter()
                .find(|b| b.name == name)
                .unwrap(),
        ),
        action,
    }
}

fn branch_run(repo: &TestRepository, name: &str, action: BranchAction) -> OperationResult {
    operate(
        &repo.path,
        Operation::Branch(Box::new(branch_request(repo, name, action))),
        &AtomicBool::new(false),
    )
}

fn seeded_branch_repo() -> TestRepository {
    let repo = TestRepository::new().expect("Git is required for branch integration tests");
    repo.stage("base\n");
    assert!(repo.commit("seed", false).committed);
    repo
}

fn assert_branch_ok(result: OperationResult) -> OperationResult {
    assert!(result.error.is_none(), "{result:?}");
    result
}

#[test]
fn branch_create_switch_rename_track_delete_and_validate_names() {
    let repo = seeded_branch_repo();
    for name in ["-bad", "bad..name", "bad name", "", "@{-1}"] {
        assert!(
            branch_run(
                &repo,
                "main",
                BranchAction::Create {
                    name: name.into(),
                    checkout: false
                }
            )
            .error
            .is_some(),
            "{name}"
        );
    }
    assert_branch_ok(branch_run(
        &repo,
        "main",
        BranchAction::Create {
            name: "feature/中文".into(),
            checkout: true,
        },
    ));
    assert_eq!(repo.metadata().current, "feature/中文");
    assert_branch_ok(branch_run(
        &repo,
        "feature/中文",
        BranchAction::Rename("feature/renamed".into()),
    ));
    assert!(
        branch_run(&repo, "feature/renamed", BranchAction::Delete)
            .error
            .is_some()
    );
    assert_branch_ok(branch_run(&repo, "main", BranchAction::Checkout));
    repo.seed_remote();
    assert_branch_ok(branch_run(
        &repo,
        "feature/renamed",
        BranchAction::Track(Some("refs/remotes/origin/main".into())),
    ));
    assert_eq!(
        repo.metadata()
            .branch_info
            .iter()
            .find(|b| b.name == "feature/renamed")
            .unwrap()
            .upstream
            .as_deref(),
        Some("refs/remotes/origin/main")
    );
    assert_branch_ok(branch_run(
        &repo,
        "feature/renamed",
        BranchAction::Track(None),
    ));
    assert_branch_ok(branch_run(&repo, "feature/renamed", BranchAction::Delete));
    assert!(
        !repo
            .metadata()
            .branches
            .iter()
            .any(|b| b.name == "feature/renamed")
    );
}

#[test]
fn branch_mutations_reject_stale_head_ref_upstream_and_cancel() {
    let repo = seeded_branch_repo();
    repo.run(&["branch", "other"]);
    let request = branch_request(&repo, "other", BranchAction::Delete);
    repo.stage("new\n");
    assert!(repo.commit("advance main", false).committed);
    assert!(
        operate(
            &repo.path,
            Operation::Branch(Box::new(request)),
            &AtomicBool::new(false)
        )
        .error
        .unwrap()
        .contains("HEAD")
    );
    let request = branch_request(&repo, "other", BranchAction::Delete);
    repo.run(&["branch", "-f", "other", "main"]);
    assert!(
        operate(
            &repo.path,
            Operation::Branch(Box::new(request)),
            &AtomicBool::new(false)
        )
        .error
        .is_some()
    );
    repo.seed_remote();
    let request = branch_request(&repo, "other", BranchAction::Delete);
    repo.run(&["branch", "--set-upstream-to=origin/main", "other"]);
    assert!(
        operate(
            &repo.path,
            Operation::Branch(Box::new(request)),
            &AtomicBool::new(false)
        )
        .error
        .is_some()
    );
    let request = branch_request(&repo, "other", BranchAction::Delete);
    assert!(
        operate(
            &repo.path,
            Operation::Branch(Box::new(request)),
            &AtomicBool::new(true)
        )
        .error
        .is_some()
    );
    assert!(repo.metadata().branches.iter().any(|b| b.name == "other"));
}

#[test]
fn checkout_preserves_local_edits_and_refuses_overwrite() {
    let repo = seeded_branch_repo();
    repo.run(&["branch", "other"]);
    std::fs::write(repo.path.join("tracked.txt"), "local\n").unwrap();
    assert_branch_ok(branch_run(&repo, "other", BranchAction::Checkout));
    assert_eq!(
        std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
        "local\n"
    );
    repo.run(&["restore", "tracked.txt"]);
    repo.stage("other\n");
    assert!(repo.commit("other change", false).committed);
    assert_branch_ok(branch_run(&repo, "main", BranchAction::Checkout));
    std::fs::write(repo.path.join("tracked.txt"), "private\n").unwrap();
    assert!(
        branch_run(&repo, "other", BranchAction::Checkout)
            .error
            .is_some()
    );
    assert_eq!(repo.metadata().current, "main");
    assert_eq!(
        std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
        "private\n"
    );
}

#[test]
fn remote_checkout_creates_tracking_branch_and_rejects_local_collision() {
    let repo = seeded_branch_repo();
    repo.seed_remote();
    repo.run(&["push", "origin", "main:feature/test"]);
    assert_branch_ok(branch_run(
        &repo,
        "origin/feature/test",
        BranchAction::Checkout,
    ));
    assert_eq!(repo.metadata().current, "feature/test");
    assert_eq!(
        repo.metadata().upstream.as_deref(),
        Some("origin/feature/test")
    );
    assert_branch_ok(branch_run(&repo, "main", BranchAction::Checkout));
    assert!(
        branch_run(&repo, "origin/feature/test", BranchAction::Checkout)
            .error
            .is_some()
    );
    assert_eq!(repo.metadata().current, "main");
}

#[test]
fn push_selected_branch_uses_its_tip_instead_of_current_head() {
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    repo.run(&["branch", "other"]);
    let other_tip = repo.run(&["rev-parse", "other"]).trim().to_string();
    repo.stage("main advanced\n");
    assert!(repo.commit("advance main", false).committed);
    assert_branch_ok(branch_run(
        &repo,
        "other",
        BranchAction::Push {
            remote: "origin".into(),
            destination: "feature/destination".into(),
            set_upstream: true,
        },
    ));
    let remote_tip = repo.run(&[
        "--git-dir",
        remote.to_str().unwrap(),
        "rev-parse",
        "refs/heads/feature/destination",
    ]);
    assert_eq!(remote_tip.trim(), other_tip);
    assert_eq!(repo.metadata().current, "main");
    assert_eq!(
        repo.metadata()
            .branch_info
            .iter()
            .find(|b| b.name == "other")
            .unwrap()
            .upstream
            .as_deref(),
        Some("refs/remotes/origin/feature/destination")
    );
}

#[test]
fn worktree_creation_reports_path_and_enforces_shared_branch_occupancy() {
    let repo = seeded_branch_repo();
    let linked = std::fs::canonicalize(&repo.path).unwrap().join("linked");
    let result = assert_branch_ok(branch_run(
        &repo,
        "main",
        BranchAction::Worktree {
            path: linked.clone(),
            new_branch: Some("linked-branch".into()),
        },
    ));
    assert_eq!(
        result.worktree_created,
        Some(std::fs::canonicalize(&linked).unwrap())
    );
    assert_eq!(repository_scope(&repo.path), repository_scope(&linked));
    assert_ne!(worktree_root(&repo.path), worktree_root(&linked));
    for action in [
        BranchAction::Checkout,
        BranchAction::Rename("oops".into()),
        BranchAction::Delete,
    ] {
        assert!(branch_run(&repo, "linked-branch", action).error.is_some());
    }
    assert!(
        branch_run(
            &repo,
            "main",
            BranchAction::Worktree {
                path: repo.path.join("second"),
                new_branch: None
            }
        )
        .error
        .is_some()
    );
    repo.run(&["worktree", "remove", linked.to_str().unwrap()]);
    assert_branch_ok(branch_run(&repo, "linked-branch", BranchAction::Delete));
}

#[test]
fn branch_compare_tracks_final_content_unique_commits_and_worktree_from_subdirectory() {
    let repo = seeded_branch_repo();
    repo.run(&["branch", "other"]);
    repo.stage("main changed\n");
    assert!(repo.commit("main only", false).committed);
    let sub = repo.path.join("src");
    std::fs::create_dir(&sub).unwrap();
    let metadata = repo.metadata();
    let mut request = ComparisonRequest {
        selected: metadata
            .branch_info
            .iter()
            .find(|b| b.name == "other")
            .unwrap()
            .clone(),
        expected: metadata.context,
        kind: ComparisonKind::Branches,
    };
    let comparison = load_comparison(&sub, request.clone(), &AtomicBool::new(false)).unwrap();
    assert_eq!(comparison.files.len(), 1);
    assert!(comparison.commits.contains(">") && comparison.commits.contains("main only"));
    let diff = load_comparison_diff(
        &sub,
        &request,
        &comparison.files[0],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(diff.patch.contains("-base") && diff.patch.contains("+main changed"));
    std::fs::write(repo.path.join("untracked.txt"), "private\n").unwrap();
    std::fs::write(repo.path.join("tracked.txt"), "working\n").unwrap();
    request.kind = ComparisonKind::WorkingTree;
    let comparison = load_comparison(&sub, request.clone(), &AtomicBool::new(false)).unwrap();
    assert!(
        comparison
            .files
            .iter()
            .any(|file| file.path == "untracked.txt")
    );
    let diff = load_comparison_diff(
        &sub,
        &request,
        comparison
            .files
            .iter()
            .find(|f| f.path == "tracked.txt")
            .unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(diff.patch.contains("-base") && diff.patch.contains("+working"));
    let file = comparison
        .files
        .iter()
        .find(|f| f.path == "untracked.txt")
        .unwrap();
    assert!(
        load_comparison_diff(&sub, &request, file, &AtomicBool::new(false))
            .unwrap()
            .patch
            .contains("+private")
    );
    assert_eq!(
        std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
        "working\n"
    );
}

#[test]
fn merge_conflict_is_detected_blocks_mutations_and_can_abort_or_continue() {
    let repo = seeded_branch_repo();
    repo.run(&["switch", "-c", "other"]);
    repo.stage("other\n");
    assert!(repo.commit("other", false).committed);
    repo.run(&["switch", "main"]);
    repo.stage("main\n");
    assert!(repo.commit("main", false).committed);
    let original = repo.run(&["rev-parse", "HEAD"]);
    let result = branch_run(&repo, "other", BranchAction::Merge);
    assert!(result.error.is_some(), "{result:?}");
    assert_eq!(repo.metadata().integration, Some(IntegrationState::Merge));
    assert_eq!(repo.metadata().conflicts, vec!["tracked.txt"]);
    assert!(
        branch_run(&repo, "other", BranchAction::Checkout)
            .error
            .is_some()
    );
    assert!(repo.commit("wrong commit route", false).error.is_some());
    let recover = |action| {
        operate(
            &repo.path,
            Operation::Branch(Box::new(BranchRequest {
                expected: repo.metadata().context,
                selected: None,
                action,
            })),
            &AtomicBool::new(false),
        )
    };
    assert!(
        recover(BranchAction::Continue(IntegrationState::Merge))
            .error
            .is_some()
    );
    assert_branch_ok(recover(BranchAction::Abort(IntegrationState::Merge)));
    assert_eq!(repo.run(&["rev-parse", "HEAD"]), original);
    assert!(
        branch_run(&repo, "other", BranchAction::Merge)
            .error
            .is_some()
    );
    repo.stage("resolved\n");
    assert_branch_ok(recover(BranchAction::Continue(IntegrationState::Merge)));
    assert_eq!(repo.metadata().integration, None);
    assert_eq!(
        repo.run(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
}

#[test]
fn checkout_rebase_uses_original_current_branch_and_reports_conflict_after_checkout() {
    let repo = seeded_branch_repo();
    repo.run(&["branch", "other"]);
    repo.stage("main\n");
    assert!(repo.commit("main", false).committed);
    assert_branch_ok(branch_run(&repo, "other", BranchAction::CheckoutRebase));
    assert_eq!(repo.metadata().current, "other");
    assert_eq!(
        repo.run(&["rev-parse", "HEAD"]),
        repo.run(&["rev-parse", "main"])
    );
    repo.stage("other\n");
    assert!(repo.commit("other", false).committed);
    repo.run(&["switch", "main"]);
    repo.stage("main2\n");
    assert!(repo.commit("main2", false).committed);
    let result = branch_run(&repo, "other", BranchAction::CheckoutRebase);
    assert!(result.message.contains("已切换到 other"), "{result:?}");
    assert!(result.error.is_some());
    assert_eq!(repo.metadata().integration, Some(IntegrationState::Rebase));
    let request = BranchRequest {
        expected: repo.metadata().context,
        selected: None,
        action: BranchAction::Abort(IntegrationState::Rebase),
    };
    assert_branch_ok(operate(
        &repo.path,
        Operation::Branch(Box::new(request)),
        &AtomicBool::new(false),
    ));
    assert_eq!(repo.metadata().current, "other");
}

#[test]
fn rebase_conflict_can_continue_after_external_resolution() {
    let repo = seeded_branch_repo();
    repo.run(&["switch", "-c", "other"]);
    repo.stage("other\n");
    assert!(repo.commit("other", false).committed);
    repo.run(&["switch", "main"]);
    repo.stage("main\n");
    assert!(repo.commit("main", false).committed);
    assert!(
        branch_run(&repo, "other", BranchAction::Rebase)
            .error
            .is_some()
    );
    assert_eq!(repo.metadata().integration, Some(IntegrationState::Rebase));
    repo.stage("resolved\n");
    let request = BranchRequest {
        expected: repo.metadata().context,
        selected: None,
        action: BranchAction::Continue(IntegrationState::Rebase),
    };
    assert_branch_ok(operate(
        &repo.path,
        Operation::Branch(Box::new(request)),
        &AtomicBool::new(false),
    ));
    assert_eq!(repo.metadata().current, "main");
    assert_eq!(repo.metadata().integration, None);
    repo.run(&["merge-base", "--is-ancestor", "other", "main"]);
}

#[test]
fn integration_refuses_dirty_files_and_delete_refuses_unmerged_tip() {
    let repo = seeded_branch_repo();
    repo.run(&["switch", "-c", "other"]);
    repo.stage("other\n");
    assert!(repo.commit("other", false).committed);
    repo.run(&["switch", "main"]);
    std::fs::write(repo.path.join("tracked.txt"), "private\n").unwrap();
    for action in [
        BranchAction::Merge,
        BranchAction::Rebase,
        BranchAction::CheckoutRebase,
    ] {
        assert!(branch_run(&repo, "other", action).error.is_some());
        assert_eq!(repo.metadata().current, "main");
        assert_eq!(
            std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
            "private\n"
        );
    }
    assert!(
        branch_run(&repo, "other", BranchAction::Delete)
            .error
            .is_some()
    );
}

#[test]
fn update_fetches_and_fast_forwards_noncurrent_branch_without_checkout() {
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    repo.run(&["branch", "--track", "other", "origin/main"]);
    let clone = repo.path.join("clone");
    repo.run(&[
        "clone",
        "-q",
        remote.to_str().unwrap(),
        clone.to_str().unwrap(),
    ]);
    std::fs::write(clone.join("upstream.txt"), "upstream\n").unwrap();
    repo.run(&["-C", clone.to_str().unwrap(), "add", "upstream.txt"]);
    repo.run(&[
        "-C",
        clone.to_str().unwrap(),
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgSign=false",
        "commit",
        "-qm",
        "remote change",
    ]);
    repo.run(&["-C", clone.to_str().unwrap(), "push", "-q"]);
    let main = repo.run(&["rev-parse", "main"]);
    assert_branch_ok(branch_run(
        &repo,
        "other",
        BranchAction::Update { rebase: true },
    ));
    assert_eq!(repo.metadata().current, "main");
    assert_eq!(repo.run(&["rev-parse", "main"]), main);
    assert_eq!(
        repo.run(&["rev-parse", "other"]),
        repo.run(&["rev-parse", "origin/main"])
    );
    assert_branch_ok(branch_run(
        &repo,
        "main",
        BranchAction::Update { rebase: false },
    ));
    assert!(repo.path.join("upstream.txt").exists());
    assert_eq!(
        repo.run(&["rev-parse", "HEAD"]),
        repo.run(&["rev-parse", "other"])
    );
}

#[test]
fn checkout_update_partial_failure_keeps_new_branch_and_reports_it() {
    let repo = seeded_branch_repo();
    repo.run(&["branch", "other"]);
    let result = branch_run(
        &repo,
        "other",
        BranchAction::CheckoutUpdate { rebase: false },
    );
    assert!(
        result.error.is_some() && result.message.contains("已切换到 other"),
        "{result:?}"
    );
    assert_eq!(repo.metadata().current, "other");
}

#[test]
fn update_noncurrent_ahead_branch_keeps_its_commits_and_rejects_divergence() {
    let repo = seeded_branch_repo();
    repo.seed_remote();
    repo.run(&["switch", "-c", "other", "--track", "origin/main"]);
    repo.stage("other\n");
    assert!(repo.commit("other", false).committed);
    let other = repo.run(&["rev-parse", "other"]);
    repo.run(&["switch", "main"]);
    assert_branch_ok(branch_run(
        &repo,
        "other",
        BranchAction::Update { rebase: false },
    ));
    assert_eq!(repo.run(&["rev-parse", "other"]), other);
    repo.stage("main\n");
    assert!(repo.commit("main", false).committed);
    repo.run(&["push", "-q"]);
    assert!(
        branch_run(&repo, "other", BranchAction::Update { rebase: true })
            .error
            .is_some()
    );
    assert_eq!(repo.run(&["rev-parse", "other"]), other);
    assert_eq!(repo.metadata().current, "main");
}

#[test]
fn current_branch_update_respects_merge_or_rebase_choice_on_divergence() {
    for rebase in [false, true] {
        let repo = seeded_branch_repo();
        repo.seed_remote();
        repo.run(&["switch", "-c", "other"]);
        std::fs::write(repo.path.join("upstream.txt"), "upstream\n").unwrap();
        repo.run(&["add", "upstream.txt"]);
        assert!(repo.commit("upstream", false).committed);
        repo.run(&["push", "origin", "other:main"]);
        repo.run(&["switch", "main"]);
        std::fs::write(repo.path.join("local.txt"), "local\n").unwrap();
        repo.run(&["add", "local.txt"]);
        assert!(repo.commit("local", false).committed);
        assert_branch_ok(branch_run(&repo, "main", BranchAction::Update { rebase }));
        assert!(repo.path.join("local.txt").exists() && repo.path.join("upstream.txt").exists());
        let parents = repo.run(&["rev-list", "--parents", "-n", "1", "HEAD"]);
        assert_eq!(
            parents.split_whitespace().count(),
            if rebase { 2 } else { 3 }
        );
        assert_eq!(repo.metadata().current, "main");
        assert_eq!(repo.metadata().integration, None);
    }
}

#[test]
fn worktree_comparison_reads_recreated_untracked_file_against_selected_branch() {
    let repo = seeded_branch_repo();
    repo.run(&["branch", "other"]);
    repo.run(&["rm", "tracked.txt"]);
    assert!(repo.commit("delete", false).committed);
    std::fs::write(repo.path.join("tracked.txt"), "recreated\n").unwrap();
    let metadata = repo.metadata();
    let request = ComparisonRequest {
        selected: metadata
            .branch_info
            .iter()
            .find(|b| b.name == "other")
            .unwrap()
            .clone(),
        expected: metadata.context,
        kind: ComparisonKind::WorkingTree,
    };
    let comparison = load_comparison(&repo.path, request.clone(), &AtomicBool::new(false)).unwrap();
    let file = comparison
        .files
        .iter()
        .find(|file| file.path == "tracked.txt")
        .unwrap();
    assert_eq!(file.kind(), ChangeKind::Modified);
    let diff = load_comparison_diff(&repo.path, &request, file, &AtomicBool::new(false)).unwrap();
    assert!(
        diff.patch.contains("-base") && diff.patch.contains("+recreated"),
        "{}",
        diff.patch
    );
    assert!(!diff.patch.contains("conduit-compare-"));
    assert!(
        repo.run(&["status", "--porcelain"])
            .contains("?? tracked.txt")
    );
}
