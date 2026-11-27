// Included in the real-repository fixture module.
#[test]
fn lock_contention_preserves_index_and_reports_recovery_in_linked_worktrees() {
    let _locale = crate::test_support::chinese();
    for linked in [false, true] {
        let repo = seeded_branch_repo();
        let workspace = if linked {
            let path = repo.path.join("linked 中文 space");
            repo.run(&["worktree", "add", "-b", "linked", path.to_str().unwrap()]);
            assert!(path.join(".git").is_file());
            path
        } else {
            repo.path.clone()
        };
        let cancel = AtomicBool::new(false);
        let index = PathBuf::from(
            run_git(
                &repo.git,
                &workspace,
                &["rev-parse", "--path-format=absolute", "--git-path", "index"],
                &cancel,
            )
            .unwrap()
            .trim(),
        );
        let lock = index.with_file_name("index.lock");
        fs::write(workspace.join("tracked.txt"), "staged edit\n").unwrap();
        run_git(&repo.git, &workspace, &["add", "tracked.txt"], &cancel).unwrap();
        let original_index = fs::read(&index).unwrap();
        let original_head =
            run_git(&repo.git, &workspace, &["rev-parse", "HEAD"], &cancel).unwrap();
        fs::write(&lock, "another writer owns this lock").unwrap();

        // Status remains available, including when .git is a worktree pointer.
        let snapshot = load(&workspace, &cancel).unwrap();
        let stage = operate(
            &workspace,
            Operation::Stage {
                files: vec!["tracked.txt".into()],
                target: snapshot.target.clone(),
            },
            &cancel,
        );
        let error = stage.error.unwrap();
        assert!(error.contains("Git 锁文件已存在"), "{error}");
        assert!(
            error
                .replace('\\', "/")
                .contains(&lock.display().to_string().replace('\\', "/")),
            "{error}"
        );
        assert!(error.contains("确认没有写操作"), "{error}");
        assert!(error.contains("退出码 128"), "{error}");
        assert!(error.contains("File exists"), "{error}");

        // Selected commits acquire the ordinary lock themselves. They must give
        // the same guidance and must never remove a lock they did not create.
        let result = operate(
            &workspace,
            Operation::Commit {
                files: Some(vec!["tracked.txt".into()]),
                message: "must not commit".into(),
                target: snapshot.target,
                push: false,
                tag: None,
            },
            &cancel,
        );
        assert!(!result.committed);
        let error = result.error.unwrap();
        assert!(error.contains("Git 锁文件已存在"), "{error}");
        assert!(error.contains(&lock.display().to_string()), "{error}");
        assert_eq!(
            fs::read_to_string(&lock).unwrap(),
            "another writer owns this lock"
        );
        assert_eq!(fs::read(&index).unwrap(), original_index);
        assert_eq!(
            run_git(&repo.git, &workspace, &["rev-parse", "HEAD"], &cancel).unwrap(),
            original_head
        );
        assert_eq!(
            fs::read_to_string(workspace.join("tracked.txt")).unwrap(),
            "staged edit\n"
        );

        // The fixture represents a writer that has now finished. Only the
        // fixture removes its own lock; the application never cleans it up.
        fs::remove_file(&lock).unwrap();
        let result = operate(
            &workspace,
            Operation::Commit {
                files: Some(vec!["tracked.txt".into()]),
                message: "retry after owner finished".into(),
                target: commit_target(&repo.git, &workspace, &cancel).unwrap(),
                push: false,
                tag: None,
            },
            &cancel,
        );
        assert!(result.committed, "{result:?}");
        assert!(result.error.is_none(), "{result:?}");
        assert!(!lock.exists());
        assert_eq!(fs::read(&index).unwrap(), original_index);
    }
}

#[test]
fn lock_acquisition_distinguishes_contention_from_other_io_errors() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let missing_parent = repo.path.join("does-not-exist/index.lock");
    let cancel = AtomicBool::new(false);
    let error = locks::acquire(&missing_parent, &cancel).err().unwrap();
    let message = format!("{error:#}");
    assert!(message.contains("无法创建 Git 锁文件"), "{message}");
    assert!(!message.contains("锁文件已存在"), "{message}");
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );
    let lock = repo.path.join(".git/index.lock");
    let guard = locks::acquire(&lock, &cancel).unwrap();
    fs::write(&lock, "owned").unwrap();
    let error = locks::acquire(&lock, &cancel).err().unwrap();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_to_string(lock).unwrap(), "owned");
    drop(guard);
}

#[test]
fn selected_commit_preparation_is_unlocked_and_rejects_concurrent_staging() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    repo.stage("selected change\n");
    let cancel = AtomicBool::new(false);
    let target = commit_target(&repo.git, &repo.path, &cancel).unwrap();
    let selected = vec!["tracked.txt".into()];
    let prepared =
        selected::prepare(&repo.git, &repo.path, &target, &selected, None, &cancel).unwrap();
    assert!(!repo.path.join(".git/index.lock").exists());
    // A different client can stage an unrelated file during preparation.
    fs::write(repo.path.join("unrelated.txt"), "external staging\n").unwrap();
    repo.run(&["add", "unrelated.txt"]);
    let index = fs::read(repo.path.join(".git/index")).unwrap();
    let head = repo.run(&["rev-parse", "HEAD"]);
    let error = prepared
        .finish(&repo.git, &target, "stale selection", &cancel)
        .unwrap_err();
    assert!(error.to_string().contains("暂存区"), "{error:#}");
    assert_eq!(repo.run(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(repo.path.join(".git/index")).unwrap(), index);
    assert_eq!(repo.run(&["show", ":unrelated.txt"]), "external staging\n");
    assert!(!repo.path.join(".git/index.lock").exists());
    // Refreshing and preparing again can commit the selection, preserving the
    // concurrently staged file byte for byte in the real index.
    let result = commit_selection(&repo, &["tracked.txt"]);
    assert!(result.committed, "{result:?}");
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(fs::read(repo.path.join(".git/index")).unwrap(), index);
    assert_eq!(
        repo.run(&["diff", "--cached", "--name-only"]).trim(),
        "unrelated.txt"
    );
}

#[test]
fn index_lock_wait_is_bounded_cancellable_and_keeps_foreign_lock() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    let lock = repo.path.join(".git/index.lock");
    fs::write(&lock, "foreign writer").unwrap();
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    assert!(locks::acquire(&lock, &cancel).is_err());
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(3));
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    let path = lock.clone();
    let waiter = thread::spawn(move || locks::acquire(&path, &signal));
    thread::sleep(Duration::from_millis(40));
    cancelled.store(true, Ordering::Release);
    let error = waiter.join().unwrap().err().unwrap();
    assert!(error.to_string().contains("取消"), "{error:#}");
    assert_eq!(fs::read_to_string(&lock).unwrap(), "foreign writer");
    // Only the simulated owner releases it. The waiting operation can then
    // acquire a fresh lock and its guard removes only that lock on return.
    let path = lock.clone();
    let owner = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        fs::remove_file(path).unwrap();
    });
    let guard = locks::acquire(&lock, &cancel).unwrap();
    owner.join().unwrap();
    assert!(lock.exists());
    drop(guard);
    assert!(!lock.exists());
}

#[test]
fn stopping_selected_commit_waits_for_worker_and_releases_owned_lock() {
    let _locale = crate::test_support::chinese();
    let repo = seeded_branch_repo();
    repo.stage("selected edit\n");
    let head = repo.run(&["rev-parse", "HEAD"]);
    let index = fs::read(repo.path.join(".git/index")).unwrap();
    fs::write(
        repo.path.join("empty-hooks/pre-commit"),
        "#!/bin/sh\nprintf ready > .git/commit-ready\nsleep 30\n",
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let operation = OperationProbe::start(
        repo.path.clone(),
        Operation::Commit {
            files: Some(vec!["tracked.txt".into()]),
            message: "cancel slow hook".into(),
            target: commit_target(&repo.git, &repo.path, &cancel).unwrap(),
            push: false,
            tag: None,
        },
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !repo.path.join(".git/commit-ready").exists() {
        assert!(Instant::now() < deadline, "commit hook did not start");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(repo.path.join(".git/index.lock").exists());
    operation.cancel();
    let result = operation
        .events
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert!(!result.committed, "{result:?}");
    assert!(result.error.unwrap().contains("取消"));
    assert_eq!(repo.run(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(repo.path.join(".git/index")).unwrap(), index);
    assert!(!repo.path.join(".git/index.lock").exists());
    assert!(fs::read_dir(repo.path.join(".git")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("conduit-commit-")
    }));
}
