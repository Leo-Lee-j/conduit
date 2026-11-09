use super::*;

pub(crate) struct Repo {
    pub path: PathBuf,
    git: PathBuf,
}
impl Repo {
    pub fn new() -> Self {
        let scratch = Scratch::new_in(&std::env::temp_dir(), "conflict-test").unwrap();
        let path = scratch.0.clone();
        std::mem::forget(scratch);
        let repo = Self {
            path,
            git: resolve_git().unwrap(),
        };
        repo.run(&["init", "-q", "-b", "main"]);
        repo.run(&["config", "user.name", "Merge Tester"]);
        repo.run(&["config", "user.email", "merge@example.invalid"]);
        repo.run(&["config", "commit.gpgsign", "false"]);
        repo.run(&["config", "core.autocrlf", "false"]);
        repo.run(&["config", "merge.conflictStyle", "merge"]);
        repo
    }
    pub fn run(&self, args: &[&str]) -> String {
        let output = self.command(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
    fn command(&self, args: &[&str]) -> std::process::Output {
        std::process::Command::new(&self.git)
            .args(args)
            .current_dir(&self.path)
            .output()
            .unwrap()
    }
    pub fn write(&self, path: &str, bytes: impl AsRef<[u8]>) {
        fs::write(self.path.join(path), bytes).unwrap();
    }
    pub fn commit(&self) {
        self.run(&["add", "-A"]);
        self.run(&["commit", "-qm", "fixture"]);
    }
    pub fn conflict(&self) {
        self.write("file[1].txt", "first\nbase\nlast\n");
        self.commit();
        self.run(&["checkout", "-qb", "incoming"]);
        self.write("file[1].txt", "first\ntheirs\nlast\n");
        self.commit();
        self.run(&["checkout", "main"]);
        self.write("file[1].txt", "first\nours\nlast\n");
        self.commit();
        assert!(!self.command(&["merge", "incoming"]).status.success());
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn three_way_plan_reconstructs_sources_and_applies_only_non_conflicting_changes() {
    let repo = Repo::new();
    let cancel = AtomicBool::new(false);
    for (base, ours, theirs) in [
        (
            "a\nb\nc\nd\ne\nf\n",
            "A\nb\nours\nd\ne\nf\n",
            "a\nb\ntheirs\nd\ne\nF\n",
        ),
        ("", "left\n", "right\n"),
        (
            "one\ntwo\nthree",
            "one\ninserted\ntwo\nthree",
            "one\ntwo\nTHREE",
        ),
        ("a\nb\nc\n", "b\nc\n", "a\nb\nnew\nc\n"),
        ("a\nb\nc\n", "a\nB\nc\n", "a\nB\nc\n"),
        ("first\r\nlast", "FIRST\r\nlast", "first\r\nLAST"),
        ("a\nb\nc\n", "a\nX\nb\nc\n", "a\nB\nc\n"),
    ] {
        let mut plan =
            MergePlan::build(&repo.git, &repo.path, base, ours, theirs, &cancel).unwrap();
        assert_eq!(
            plan.blocks
                .iter()
                .map(|b| b.base.as_str())
                .collect::<String>(),
            base
        );
        assert_eq!(
            plan.blocks
                .iter()
                .map(|b| b.ours.as_str())
                .collect::<String>(),
            ours
        );
        assert_eq!(
            plan.blocks
                .iter()
                .map(|b| b.theirs.as_str())
                .collect::<String>(),
            theirs
        );
        for block in &mut plan.blocks {
            block.apply_non_conflicting();
            assert_eq!(block.pending(), block.conflict);
            if block.conflict {
                block.decisions = [Decision::Apply, Decision::Ignore];
                assert_eq!(block.result(), block.ours);
                block.decisions = [Decision::Ignore, Decision::Apply];
                assert_eq!(block.result(), block.theirs);
                block.reset();
                assert!(block.pending());
                assert_eq!(block.result(), block.base);
            }
        }
    }
}

#[test]
fn saves_reviewed_merge_and_rejects_stale_worktree_index_and_markers() {
    let repo = Repo::new();
    repo.conflict();
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
    assert!(
        file.plan
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .any(|b| b.conflict)
    );
    let original = fs::read(repo.path.join("file[1].txt")).unwrap();
    repo.write("unrelated.txt", "unrelated staged\n");
    repo.run(&["add", "unrelated.txt"]);
    repo.write("file1.txt", "literal path must not touch this\n");
    repo.write("file[1].txt", "external edit\n");
    let resolution = Resolution {
        expected: file.snapshot.clone(),
        content: ResolutionContent::Merged("reviewed\n".into()),
    };
    assert!(resolve(&repo.git, &repo.path, resolution.clone(), &cancel).is_err());
    assert_eq!(
        fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
        "external edit\n"
    );
    repo.write("file[1].txt", original);
    let invalid = Resolution {
        content: ResolutionContent::Merged("<<<<<<< ours\nstill conflicted\n".into()),
        ..resolution.clone()
    };
    assert!(resolve(&repo.git, &repo.path, invalid, &cancel).is_err());
    assert!(!repo.path.join(".git/index.lock").exists());
    resolve(&repo.git, &repo.path, resolution.clone(), &cancel).unwrap();
    assert!(repo.run(&["ls-files", "-u"]).is_empty());
    assert_eq!(repo.run(&["show", ":file[1].txt"]), "reviewed\n");
    assert_eq!(repo.run(&["show", ":unrelated.txt"]), "unrelated staged\n");
    assert_eq!(
        fs::read_to_string(repo.path.join("file1.txt")).unwrap(),
        "literal path must not touch this\n"
    );
    assert!(resolve(&repo.git, &repo.path, resolution, &cancel).is_err());
    assert!(
        repo.path.join(".git/MERGE_HEAD").exists(),
        "resolving does not commit the merge"
    );
}

#[test]
fn whole_version_resolution_preserves_binary_bytes_and_handles_deletion() {
    let cancel = AtomicBool::new(false);
    for keep in [true, false] {
        let repo = Repo::new();
        repo.write("binary", [0, 255, 1]);
        repo.write("deleted", "base");
        repo.commit();
        repo.run(&["checkout", "-qb", "incoming"]);
        repo.write("binary", [0, 255, 2]);
        repo.run(&["rm", "deleted"]);
        repo.commit();
        repo.run(&["checkout", "main"]);
        repo.write("binary", [0, 255, 3]);
        repo.write("deleted", "changed");
        repo.commit();
        assert!(!repo.command(&["merge", "incoming"]).status.success());
        for path in ["binary", "deleted"] {
            let file = load(&repo.path, path, &cancel).unwrap();
            assert!(file.plan.is_none());
            resolve(
                &repo.git,
                &repo.path,
                Resolution {
                    expected: file.snapshot,
                    content: if keep {
                        ResolutionContent::Ours
                    } else {
                        ResolutionContent::Theirs
                    },
                },
                &cancel,
            )
            .unwrap();
        }
        assert_eq!(
            fs::read(repo.path.join("binary")).unwrap(),
            vec![0, 255, if keep { 3 } else { 2 }]
        );
        assert_eq!(repo.path.join("deleted").exists(), keep);
        assert!(repo.run(&["ls-files", "-u"]).is_empty());
    }
}

#[test]
fn rebase_and_linked_worktree_use_their_own_index_and_stage_meanings() {
    let repo = Repo::new();
    repo.conflict();
    repo.run(&["merge", "--abort"]);
    repo.run(&["checkout", "incoming"]);
    assert!(!repo.command(&["rebase", "main"]).status.success());
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
    assert_eq!(file.snapshot.integration, Some(IntegrationState::Rebase));
    let blocks = &file.plan.unwrap().blocks;
    assert_eq!(
        blocks.iter().map(|b| b.ours.as_str()).collect::<String>(),
        "first\nours\nlast\n"
    );
    assert_eq!(
        blocks.iter().map(|b| b.theirs.as_str()).collect::<String>(),
        "first\ntheirs\nlast\n"
    );
    repo.run(&["rebase", "--abort"]);
    repo.run(&["checkout", "main"]);
    let linked = repo.path.join("linked");
    repo.run(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "linked",
        linked.to_str().unwrap(),
        "main",
    ]);
    let output = std::process::Command::new(&repo.git)
        .args(["merge", "incoming"])
        .current_dir(&linked)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let file = load(&linked, "file[1].txt", &cancel).unwrap();
    resolve(
        &repo.git,
        &linked,
        Resolution {
            expected: file.snapshot,
            content: ResolutionContent::Theirs,
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(linked.join("file[1].txt")).unwrap(),
        "first\ntheirs\nlast\n"
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
        "first\nours\nlast\n"
    );
}

#[test]
fn failed_staging_rolls_back_worktree_and_keeps_unmerged_index() {
    let repo = Repo::new();
    repo.conflict();
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
    let before = fs::read(repo.path.join("file[1].txt")).unwrap();
    repo.write(".gitattributes", "* filter=fail\n");
    repo.run(&["config", "filter.fail.clean", "exit 1"]);
    repo.run(&["config", "filter.fail.required", "true"]);
    assert!(
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot,
                content: ResolutionContent::Merged("edited\n".into())
            },
            &cancel
        )
        .is_err()
    );
    assert_eq!(fs::read(repo.path.join("file[1].txt")).unwrap(), before);
    assert!(!repo.run(&["ls-files", "-u"]).is_empty());
    assert!(!repo.path.join(".git/index.lock").exists());
}

#[test]
fn merged_text_preserves_worktree_encoding() {
    for (encoding, eol) in [
        ("ISO-8859-1", "lf"),
        ("ISO-8859-1", "crlf"),
        ("UTF-16LE", "crlf"),
        ("UTF-8", "crlf"),
    ] {
        let encode = |text: &str| -> Vec<u8> {
            let text = if eol == "crlf" {
                text.replace('\n', "\r\n")
            } else {
                text.into()
            };
            match encoding {
                "ISO-8859-1" => text
                    .chars()
                    .map(|c| u8::try_from(u32::from(c)).unwrap())
                    .collect(),
                "UTF-16LE" => text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                _ => text.into_bytes(),
            }
        };
        let repo = Repo::new();
        repo.write(
            ".gitattributes",
            format!("*.txt text eol={eol} working-tree-encoding={encoding}\n"),
        );
        repo.write("encoded.txt", encode("base café\n"));
        repo.commit();
        repo.run(&["checkout", "-qb", "incoming"]);
        repo.write("encoded.txt", encode("theirs café\n"));
        repo.commit();
        repo.run(&["checkout", "main"]);
        repo.write("encoded.txt", encode("ours café\n"));
        repo.commit();
        assert!(!repo.command(&["merge", "incoming"]).status.success());
        let cancel = AtomicBool::new(false);
        let file = load(&repo.path, "encoded.txt", &cancel).unwrap();
        assert!(file.plan.is_some(), "{:?}", file.note);
        assert!(
            file.plan
                .as_ref()
                .unwrap()
                .blocks
                .iter()
                .all(|b| b.worktree.is_none())
        );
        if encoding == "ISO-8859-1" {
            let before = fs::read(repo.path.join("encoded.txt")).unwrap();
            assert!(
                resolve(
                    &repo.git,
                    &repo.path,
                    Resolution {
                        expected: file.snapshot.clone(),
                        content: ResolutionContent::Merged("无法编码\n".into()),
                    },
                    &cancel
                )
                .is_err()
            );
            assert_eq!(fs::read(repo.path.join("encoded.txt")).unwrap(), before);
            assert!(!repo.run(&["ls-files", "-u"]).is_empty());
        }
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot,
                content: ResolutionContent::Merged("ours café\n".into()),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(repo.run(&["show", ":encoded.txt"]), "ours café\n");
        assert_eq!(
            fs::read(repo.path.join("encoded.txt")).unwrap(),
            encode("ours café\n")
        );
    }
}

#[test]
fn original_merge_styles_keep_block_actions_and_existing_edits_are_preserved() {
    let cancel = AtomicBool::new(false);
    for style in ["merge", "diff3", "zdiff3"] {
        let repo = Repo::new();
        repo.run(&["config", "merge.conflictStyle", style]);
        repo.write(".gitattributes", "*.txt conflict-marker-size=12\n");
        repo.conflict();
        let original = fs::read_to_string(repo.path.join("file[1].txt")).unwrap();
        let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
        assert!(
            file.plan
                .as_ref()
                .unwrap()
                .blocks
                .iter()
                .all(|b| b.worktree.is_none()),
            "{style}"
        );

        let edited = original.replacen("first\n", "manually revised header\n", 1);
        repo.write("file[1].txt", &edited);
        let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
        let mut plan = file.plan.unwrap();
        assert_eq!(plan.blocks.len(), 1);
        plan.blocks[0].apply_non_conflicting();
        assert_eq!(plan.blocks[0].result(), edited);
        assert!(plan.blocks[0].pending());

        let resolved = "manually revised header\nmerged by hand\nlast\n";
        repo.write("file[1].txt", resolved);
        let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
        assert_eq!(file.plan.as_ref().unwrap().blocks[0].result(), resolved);
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot,
                content: ResolutionContent::Merged(resolved.into()),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(repo.run(&["show", ":file[1].txt"]), resolved);
    }
}

#[test]
fn nonconflicting_executable_bit_survives_text_merge() {
    for (base, ours, theirs, expected) in [
        (false, false, true, "100755"),
        (true, true, false, "100644"),
        (false, true, false, "100755"),
        (true, false, true, "100644"),
    ] {
        let repo = Repo::new();
        let commit = |text: &str, executable: bool| {
            repo.write("script.sh", text);
            repo.run(&["add", "script.sh"]);
            repo.run(&[
                "update-index",
                if executable {
                    "--chmod=+x"
                } else {
                    "--chmod=-x"
                },
                "script.sh",
            ]);
            repo.run(&["commit", "-qm", "fixture"]);
        };
        commit("base\n", base);
        repo.run(&["checkout", "-qb", "incoming"]);
        commit("theirs\n", theirs);
        repo.run(&["checkout", "main"]);
        commit("ours\n", ours);
        assert!(!repo.command(&["merge", "incoming"]).status.success());
        let cancel = AtomicBool::new(false);
        let file = load(&repo.path, "script.sh", &cancel).unwrap();
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot,
                content: ResolutionContent::Merged("merged\n".into()),
            },
            &cancel,
        )
        .unwrap();
        let entry = repo.run(&["ls-files", "--stage", "--", "script.sh"]);
        assert!(entry.starts_with(expected), "mode was lost: {entry}");
    }
}

#[test]
fn merged_text_uses_clean_and_smudge_filters() {
    let repo = Repo::new();
    repo.run(&["config", "filter.review.clean", "sed 's/WORK:/REPO:/g'"]);
    repo.run(&["config", "filter.review.smudge", "sed 's/REPO:/WORK:/g'"]);
    repo.run(&["config", "filter.review.required", "true"]);
    repo.write(".gitattributes", "*.txt text eol=lf filter=review\n");
    repo.write("filtered.txt", "WORK:base\n");
    repo.commit();
    repo.run(&["checkout", "-qb", "incoming"]);
    repo.write("filtered.txt", "WORK:theirs\n");
    repo.commit();
    repo.run(&["checkout", "main"]);
    repo.write("filtered.txt", "WORK:ours\n");
    repo.commit();
    assert!(!repo.command(&["merge", "incoming"]).status.success());
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "filtered.txt", &cancel).unwrap();
    assert!(
        file.plan
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .all(|b| b.worktree.is_none())
    );
    resolve(
        &repo.git,
        &repo.path,
        Resolution {
            expected: file.snapshot,
            content: ResolutionContent::Merged("REPO:merged\n".into()),
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(repo.run(&["show", ":filtered.txt"]), "REPO:merged\n");
    assert_eq!(
        fs::read_to_string(repo.path.join("filtered.txt")).unwrap(),
        "WORK:merged\n"
    );
}

#[test]
fn conflict_tokens_allow_plain_separators_but_reject_partial_and_custom_markers() {
    for text in [
        "Title\n==============\n",
        "======= ordinary content\n",
        "<<<<<<<operator\n",
        "x >>>>>>> y\n",
    ] {
        assert!(!has_conflict_markers(text, 7), "{text}");
    }
    for text in [
        "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\n",
        "<<<<<<< HEAD\r\npartial\r\n",
        "||||||| ancestor\nbase\n",
        ">>>>>>> branch\n",
        "<<<<<<<<<<<< ours\nx\n============\ny\n>>>>>>>>>>>> theirs\n",
    ] {
        assert!(has_conflict_markers(text, 7), "{text}");
    }
    let repo = Repo::new();
    repo.write(".gitattributes", "*.txt conflict-marker-size=3\n");
    repo.conflict();
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
    let before = fs::read_to_string(repo.path.join("file[1].txt")).unwrap();
    assert!(before.contains("<<< HEAD"));
    assert!(
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot,
                content: ResolutionContent::Merged(before.clone()),
            },
            &cancel
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
        before
    );
    assert!(!repo.run(&["ls-files", "-u"]).is_empty());
}

#[test]
fn edits_during_conversion_are_not_overwritten_or_rolled_back() {
    let repo = Repo::new();
    repo.conflict();
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
    repo.write(".gitattributes", "*.txt filter=external\n");
    repo.run(&[
        "config",
        "filter.external.smudge",
        "printf 'external edit\\n' > 'file[1].txt'; cat",
    ]);
    assert!(
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot,
                content: ResolutionContent::Merged("reviewed\n".into()),
            },
            &cancel
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
        "external edit\n"
    );
    assert!(!repo.run(&["ls-files", "-u"]).is_empty());
    assert!(!repo.path.join(".git/index.lock").exists());
}

#[test]
fn added_file_mode_disagreement_requires_an_explicit_side() {
    let repo = Repo::new();
    repo.run(&["commit", "--allow-empty", "-qm", "base"]);
    repo.run(&["checkout", "-qb", "incoming"]);
    repo.write("new.sh", "theirs\n");
    repo.run(&["add", "new.sh"]);
    repo.run(&["update-index", "--chmod=+x", "new.sh"]);
    repo.run(&["commit", "-qm", "incoming"]);
    repo.run(&["checkout", "main"]);
    repo.write("new.sh", "ours\n");
    repo.commit();
    assert!(!repo.command(&["merge", "incoming"]).status.success());
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "new.sh", &cancel).unwrap();
    let before = fs::read(repo.path.join("new.sh")).unwrap();
    assert!(
        resolve(
            &repo.git,
            &repo.path,
            Resolution {
                expected: file.snapshot.clone(),
                content: ResolutionContent::Merged("merged\n".into()),
            },
            &cancel
        )
        .is_err()
    );
    assert_eq!(fs::read(repo.path.join("new.sh")).unwrap(), before);
    assert!(!repo.run(&["ls-files", "-u"]).is_empty());
    resolve(
        &repo.git,
        &repo.path,
        Resolution {
            expected: file.snapshot,
            content: ResolutionContent::Theirs,
        },
        &cancel,
    )
    .unwrap();
    assert!(
        repo.run(&["ls-files", "--stage", "new.sh"])
            .starts_with("100755 ")
    );
    assert_eq!(repo.run(&["show", ":new.sh"]), "theirs\n");
}

#[test]
fn markdown_heading_is_not_a_conflict_marker() {
    let repo = Repo::new();
    repo.conflict();
    let cancel = AtomicBool::new(false);
    let file = load(&repo.path, "file[1].txt", &cancel).unwrap();
    let resolved = "Document title\n==============\nmerged content\n";
    resolve(
        &repo.git,
        &repo.path,
        Resolution {
            expected: file.snapshot,
            content: ResolutionContent::Merged(resolved.into()),
        },
        &cancel,
    )
    .unwrap();
    assert_eq!(repo.run(&["show", ":file[1].txt"]), resolved);
}
