// Included in the real-repository fixture module alongside branch_tests.rs.
#[test]
fn new_branch_commit_and_push_publishes_and_tracks_the_same_name() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    let base = repo.run(&["rev-parse", "main"]);
    assert_branch_ok(branch_run(
        &repo,
        "main",
        BranchAction::Create {
            name: "feature/中文".into(),
            checkout: true,
        },
    ));
    assert!(repo.metadata().upstream.is_none());

    for content in ["first push\n", "follow-up push\n"] {
        repo.stage(content);
        let result = repo.commit(content.trim(), true);
        assert!(result.committed && result.error.is_none(), "{result:?}");
        let metadata = repo.metadata();
        assert_eq!(metadata.upstream.as_deref(), Some("origin/feature/中文"));
        assert_eq!(metadata.ahead, 0);
        assert_eq!(
            repo.run(&["rev-parse", "HEAD"]),
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "rev-parse",
                "refs/heads/feature/中文",
            ])
        );
    }
    assert_eq!(
        base,
        repo.run(&["--git-dir", remote.to_str().unwrap(), "rev-parse", "main"])
    );
}

#[test]
fn first_push_honors_explicit_remotes_before_origin() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    for name in ["fork", "backup"] {
        repo.run(&["remote", "add", name, remote.to_str().unwrap()]);
    }
    for (index, push_remote, push_default, branch_remote, expected) in [
        (0, "", "", "", "origin"),
        (1, "", "fork", "backup", "fork"),
        (2, "backup", "fork", "origin", "backup"),
        (3, "", "", "fork", "fork"),
    ] {
        let branch = format!("feature/remote-{index}");
        repo.run(&["switch", "--no-track", "-c", &branch, "main"]);
        repo.run(&["config", "remote.pushDefault", push_default]);
        repo.run(&[
            "config",
            &format!("branch.{branch}.pushRemote"),
            push_remote,
        ]);
        repo.run(&["config", &format!("branch.{branch}.remote"), branch_remote]);
        repo.stage(&branch);
        let result = repo.commit(&branch, true);
        assert!(result.committed && result.error.is_none(), "{result:?}");
        assert_eq!(
            repo.metadata().upstream,
            Some(format!("{expected}/{branch}"))
        );
    }
}

#[test]
fn first_push_uses_the_only_remote_without_origin() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    repo.seed_remote();
    repo.run(&["remote", "rename", "origin", "company"]);
    repo.run(&["switch", "--no-track", "-c", "feature/only", "main"]);
    repo.stage("only remote\n");
    let result = repo.commit("publish", true);
    assert!(result.committed && result.error.is_none(), "{result:?}");
    assert_eq!(
        repo.metadata().upstream.as_deref(),
        Some("company/feature/only")
    );
}

#[test]
fn first_push_reports_ambiguous_or_invalid_remotes_without_setting_upstream() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    repo.run(&["remote", "rename", "origin", "first"]);
    repo.run(&["remote", "add", "second", remote.to_str().unwrap()]);
    repo.run(&["switch", "--no-track", "-c", "feature/choose", "main"]);
    repo.stage("choose remote\n");
    let result = repo.commit("choose", true);
    assert!(result.committed && result.error.unwrap().contains("多个远程"));
    assert!(repo.metadata().upstream.is_none());

    repo.run(&["config", "remote.pushDefault", "missing"]);
    repo.stage("invalid remote\n");
    let result = repo.commit("invalid", true);
    assert!(result.committed && result.error.unwrap().contains("missing 不存在"));
    assert!(repo.metadata().upstream.is_none());
    assert!(
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "for-each-ref",
            "refs/heads/feature/choose",
        ])
        .is_empty()
    );
}

#[test]
fn commit_push_preserves_configured_destination_when_tracking_ref_is_missing() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    repo.run(&["switch", "--no-track", "-c", "feature/local", "main"]);
    repo.run(&["push", "-u", "origin", "feature/local:feature/published"]);
    repo.run(&["update-ref", "-d", "refs/remotes/origin/feature/published"]);
    // An existing upstream must keep its destination even with another push default.
    repo.run(&["config", "remote.pushDefault", "missing"]);
    repo.stage("configured target\n");
    let result = repo.commit("update", true);
    assert!(result.committed && result.error.is_none(), "{result:?}");
    assert_eq!(
        repo.metadata().upstream.as_deref(),
        Some("origin/feature/published")
    );
    assert_eq!(
        repo.run(&["rev-parse", "HEAD"]),
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "rev-parse",
            "refs/heads/feature/published",
        ])
    );
    assert!(
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "for-each-ref",
            "refs/heads/feature/local",
        ])
        .is_empty()
    );
}

#[test]
fn rejected_first_push_keeps_commit_without_upstream_or_remote_overwrite() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let remote = repo.seed_remote();
    let base = repo.run(&["rev-parse", "main"]);
    repo.stage("remote advance\n");
    assert!(repo.commit("remote advance", false).committed);
    repo.run(&["push", "origin", "main:feature/collision"]);
    let remote_tip = repo.run(&["rev-parse", "HEAD"]);
    repo.run(&[
        "switch",
        "--no-track",
        "-c",
        "feature/collision",
        base.trim(),
    ]);
    repo.stage("local divergence\n");
    let result = repo.commit("local divergence", true);
    assert!(result.committed && result.error.is_some(), "{result:?}");
    assert_eq!(
        repo.run(&["log", "-1", "--format=%s"]).trim(),
        "local divergence"
    );
    assert!(repo.metadata().upstream.is_none());
    assert_eq!(repo.metadata().ahead, 1);
    assert_eq!(
        remote_tip,
        repo.run(&[
            "--git-dir",
            remote.to_str().unwrap(),
            "rev-parse",
            "refs/heads/feature/collision",
        ])
    );
}

#[test]
fn tagged_first_push_sets_upstream_only_after_atomic_success() {
    let _locale = crate::test_support::chinese();
    for atomic_supported in [true, false] {
        let repo = seeded_branch_repo();
        let remote = repo.seed_remote();
        let cancel = AtomicBool::new(false);
        repo.run(&["switch", "--no-track", "-c", "feature/tagged", "main"]);
        repo.run(&["tag", "-a", "local-only", "-m", "local only"]);
        repo.run(&["config", "push.followTags", "true"]);
        if !atomic_supported {
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "config",
                "receive.advertiseAtomic",
                "false",
            ]);
        }
        repo.stage("tagged first push\n");
        let result = operate(
            &repo.path,
            Operation::Commit {
                files: Some(vec!["tracked.txt".into()]),
                message: "tagged first push".into(),
                target: commit_target(&repo.git, &repo.path, &cancel).unwrap(),
                push: true,
                tag: Some("v0.2.0".into()),
            },
            &cancel,
        );
        assert!(result.committed, "{result:?}");
        if atomic_supported {
            assert!(result.error.is_none(), "{result:?}");
            assert!(result.retry_tag.is_none());
        } else {
            assert!(result.error.is_some(), "{result:?}");
            assert!(repo.metadata().upstream.is_none());
            assert!(
                repo.run(&[
                    "--git-dir",
                    remote.to_str().unwrap(),
                    "for-each-ref",
                    "refs/heads/feature/tagged",
                    "refs/tags/v0.2.0",
                ])
                .is_empty()
            );
            let retry = result
                .retry_tag
                .expect("retain the chosen remote for retry");
            assert_eq!(retry.remote, "origin");
            assert_eq!(retry.merge, "refs/heads/feature/tagged");
            assert_branch_ok(branch_run(
                &repo,
                "feature/tagged",
                BranchAction::Push {
                    remote: "origin".into(),
                    destination: "feature/tagged".into(),
                    set_upstream: true,
                },
            ));
            assert_branch_ok(operate(&repo.path, Operation::RetryTag(retry), &cancel));
        }
        assert_eq!(
            repo.metadata().upstream.as_deref(),
            Some("origin/feature/tagged")
        );
        assert_eq!(repo.metadata().ahead, 0);
        for reference in ["refs/heads/feature/tagged", "refs/tags/v0.2.0"] {
            assert_eq!(
                repo.run(&["rev-parse", "HEAD"]),
                repo.run(&[
                    "--git-dir",
                    remote.to_str().unwrap(),
                    "rev-parse",
                    reference
                ])
            );
        }
        assert!(
            repo.run(&[
                "--git-dir",
                remote.to_str().unwrap(),
                "tag",
                "-l",
                "local-only"
            ])
            .trim()
            .is_empty()
        );
    }
}
