use super::{Changes, ChatItem, DiffFile, EditFileStatus};
use serde_json::json;

fn edit(key: &str, path: &str, kind: &str, patch: &str) -> ChatItem {
    serde_json::from_value(json!({
        "key": key, "kind": "Tool", "text": "fileChange", "status": "completed",
        "input": json!([{"path": path, "kind": kind, "diff": patch}]).to_string()
    }))
    .unwrap()
}

fn messages() -> Vec<ChatItem> {
    vec![
        edit(
            "create",
            "C:/project/new.txt",
            "add",
            "@@ -0,0 +1,2 @@\n+first\n+second\n",
        ),
        serde_json::from_value(json!({"key":"next", "kind":"User", "text":"Next turn"})).unwrap(),
        edit(
            "update",
            "new.txt",
            "update",
            "@@ -1 +1 @@\n-first\n+latest\n",
        ),
        edit(
            "other",
            "C:/project/other.txt",
            "update",
            "@@ -1 +1,2 @@\n-before\n+after\n+extra\n",
        ),
        edit(
            "delete",
            "C:/project/gone.txt",
            "delete",
            "@@ -1 +0,0 @@\n-removed\n",
        ),
    ]
}

#[test]
fn session_files_merge_turns_paths_and_keep_only_the_latest_diff() {
    let _locale = crate::test_support::chinese();
    let changes = Changes::collect(&messages(), "C:/project");
    assert_eq!(
        changes.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["gone.txt", "new.txt", "other.txt"]
    );
    assert_eq!((changes.added, changes.removed), (5, 3));
    let new = &changes.files["new.txt"];
    assert_eq!(new.status, EditFileStatus::Added);
    assert_eq!((new.added, new.removed), (3, 1));
    assert!(new.latest.patch.contains("+latest"));
    assert!(!new.latest.patch.contains("+first"));
    assert_eq!(changes.files["other.txt"].status, EditFileStatus::Modified);
    assert_eq!(changes.files["gone.txt"].status, EditFileStatus::Deleted);
}

struct Repository(crate::transcript::tests::Fixture);

impl Repository {
    fn new() -> Self {
        let repo = Self(crate::transcript::tests::Fixture::new());
        repo.git(&["init", "-q"]);
        // Keep the test independent of the developer's global ignore patterns.
        std::fs::write(repo.0.root.join("global-ignore"), "*.global\n").unwrap();
        repo.git(&[
            "config",
            "core.excludesFile",
            repo.0.root.join("global-ignore").to_str().unwrap(),
        ]);
        repo
    }

    fn git(&self, args: &[&str]) {
        let output = std::process::Command::new(crate::git::resolve_git().expect("Git for tests"))
            .current_dir(&self.0.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0.root).unwrap();
    }
}

#[test]
fn session_files_exclude_git_ignored_paths_from_rows_and_totals() {
    let _locale = crate::test_support::chinese();
    let repo = Repository::new();
    std::fs::create_dir(repo.0.root.join("nested")).unwrap();
    std::fs::write(repo.0.root.join("tracked.log"), "tracked\n").unwrap();
    repo.git(&["add", "tracked.log"]);
    std::fs::write(
        repo.0.root.join(".gitignore"),
        "target/\n*.log\n!keep.log\n",
    )
    .unwrap();
    std::fs::write(
        repo.0.root.join("nested/.gitignore"),
        "*.cache\n!keep.cache\n",
    )
    .unwrap();
    std::fs::write(repo.0.root.join(".git/info/exclude"), "*.local\n").unwrap();
    let root = repo.0.root.to_string_lossy();
    let paths = [
        "src/main.rs".to_owned(),
        "untracked.rs".to_owned(),
        "deleted.rs".to_owned(),
        "tracked.log".to_owned(),
        "keep.log".to_owned(),
        "nested/keep.cache".to_owned(),
        // Absolute and Windows verbatim paths are normalized before filtering.
        repo.0
            .root
            .join("target/generated.rs")
            .to_string_lossy()
            .into_owned(),
        std::fs::canonicalize(&repo.0.root)
            .unwrap()
            .join("debug.log")
            .to_string_lossy()
            .into_owned(),
        "nested/deleted.cache".to_owned(),
        "private.local".to_owned(),
        "private.global".to_owned(),
        "target/中文 空格.txt".to_owned(),
    ];
    // Most paths no longer exist: filtering must use the recorded paths, not a directory walk.
    let items: Vec<_> = paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            edit(
                &format!("edit-{i}"),
                path,
                "update",
                "@@ -1 +1 @@\n-before\n+after\n",
            )
        })
        .collect();
    let changes = Changes::collect(&items, &root);
    assert_eq!(
        changes.files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "deleted.rs",
            "keep.log",
            "nested/keep.cache",
            "src/main.rs",
            "tracked.log",
            "untracked.rs"
        ]
    );
    assert_eq!((changes.added, changes.removed), (6, 6));

    // A workspace can point at a subdirectory and still inherit repository ignores.
    let changes = Changes::collect(
        &[
            edit(
                "ignored",
                "deleted.cache",
                "delete",
                "@@ -1 +0,0 @@\n-old\n",
            ),
            edit("kept", "keep.cache", "add", "@@ -0,0 +1 @@\n+new\n"),
        ],
        &repo.0.root.join("nested").to_string_lossy(),
    );
    assert_eq!(
        changes.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["keep.cache"]
    );
    assert_eq!((changes.added, changes.removed), (1, 0));
}

#[test]
fn session_files_match_canonical_paths_under_workspace_aliases() {
    let _locale = crate::test_support::chinese();
    let repo = Repository::new();
    std::fs::create_dir(repo.0.root.join("nested")).unwrap();
    std::fs::write(repo.0.root.join(".gitignore"), "*.log\n").unwrap();
    let canonical_root = std::fs::canonicalize(&repo.0.root).unwrap();
    // Reproduce distinct workspace/tool spellings without relying on a host's
    // Windows short temp paths. Neither recorded file needs to exist.
    let alias = repo.0.root.join("nested/..");
    let changes = Changes::collect(
        &[
            edit(
                "ignored",
                &canonical_root.join("debug.log").to_string_lossy(),
                "delete",
                "@@ -1 +0,0 @@\n-old\n",
            ),
            edit(
                "kept",
                &canonical_root.join("source.rs").to_string_lossy(),
                "add",
                "@@ -0,0 +1 @@\n+new\n",
            ),
        ],
        &alias.to_string_lossy(),
    );
    assert_eq!(
        changes.files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["source.rs"]
    );
    assert_eq!((changes.added, changes.removed), (1, 0));
}

#[test]
fn session_files_keep_recorded_changes_outside_git_repositories() {
    let _locale = crate::test_support::chinese();
    let fixture = crate::transcript::tests::Fixture::new();
    std::fs::write(fixture.root.join(".gitignore"), "*.log\n").unwrap();
    let changes = Changes::collect(
        &[edit("write", "debug.log", "add", "@@ -0,0 +1 @@\n+entry\n")],
        &fixture.root.to_string_lossy(),
    );
    assert_eq!(changes.files.len(), 1);
    assert_eq!((changes.added, changes.removed), (1, 0));
}

#[test]
fn session_files_filter_large_batches_and_handle_empty_git_results() {
    let _locale = crate::test_support::chinese();
    let repo = Repository::new();
    std::fs::write(repo.0.root.join(".gitignore"), "*.ignored\n").unwrap();
    let root = repo.0.root.to_string_lossy();
    let ordinary = edit("kept", "ordinary.rs", "add", "@@ -0,0 +1 @@\n+source\n");
    let changes = Changes::collect(std::slice::from_ref(&ordinary), &root);
    assert_eq!(
        (changes.files.len(), changes.added, changes.removed),
        (1, 1, 0)
    );

    let mut items: Vec<_> = (0..600)
        .map(|i| {
            edit(
                &format!("ignored-{i}"),
                &format!("中文 空格 {i} {}.ignored", "x".repeat(160)),
                "add",
                "@@ -0,0 +1 @@\n+generated\n",
            )
        })
        .collect();
    let changes = Changes::collect(&items, &root);
    assert!(changes.files.is_empty());
    assert_eq!((changes.added, changes.removed), (0, 0));
    items.push(ordinary);
    let changes = Changes::collect(&items, &root);
    assert_eq!(
        (changes.files.len(), changes.added, changes.removed),
        (1, 1, 0)
    );
}

#[test]
fn failed_pending_and_denied_edits_do_not_replace_completed_changes() {
    let _locale = crate::test_support::chinese();
    let mut items = messages();
    for status in ["failed", "running", "cancelled", "interrupted"] {
        let mut item = edit(status, "new.txt", "delete", "@@ -1 +0,0 @@\n-no\n");
        item.status = status.into();
        items.push(item);
    }
    let mut denied = edit("denied", "new.txt", "delete", "@@ -1 +0,0 @@\n-no\n");
    denied.approval = Some(crate::domain::ToolApproval {
        request: "approval".into(),
        status: crate::message_status::ApprovalStatus::Denied,
        questions: Vec::new(),
    });
    items.push(denied);
    let changes = Changes::collect(&items, "C:/project");
    assert_eq!((changes.added, changes.removed), (5, 3));
    assert_eq!(changes.files["new.txt"].status, EditFileStatus::Added);
    assert!(changes.files["new.txt"].latest.patch.contains("+latest"));
}

#[test]
fn session_files_keep_diff_previews_beyond_the_inline_tool_limit() {
    let _locale = crate::test_support::chinese();
    let changes: Vec<_> = (0..12)
        .map(|index| {
            json!({
                "path": format!("file-{index}.txt"), "kind": "add",
                "diff": "@@ -0,0 +1 @@\n+content\n"
            })
        })
        .collect();
    let item: ChatItem = serde_json::from_value(json!({
        "key": "many", "kind": "Tool", "text": "fileChange", "status": "completed",
        "input": serde_json::to_string(&changes).unwrap()
    }))
    .unwrap();
    let changes = Changes::collect(&[item], "C:/project");
    assert_eq!(changes.files.len(), 12);
    assert_eq!(changes.added, 12);
    for file in changes.files.values() {
        assert_eq!(DiffFile::parse(&file.latest.patch).unwrap().len(), 1);
        assert!(file.latest.patch.contains("+content"));
    }
}

#[test]
fn session_files_and_diff_survive_restart_without_workspace_files() {
    let _locale = crate::test_support::chinese();
    let fixture = crate::transcript::tests::Fixture::new();
    let mut store = fixture.store(1);
    store.workspaces[0].sessions[0].messages = messages();
    // Save before ever building the card, as happens when its panel stays closed.
    store.save_to(&fixture.state).unwrap();
    drop(store);
    let restored = crate::store::Store::load_from(&fixture.state).unwrap();
    let cold = &restored.workspaces[0].sessions[0];
    assert!(!cold.transcript.loaded);
    assert!(cold.messages.is_empty());
    let loaded = crate::transcript::load(cold.clone()).unwrap();
    let changes = Changes::collect(&loaded.messages, "C:/project");
    assert_eq!(changes.files.len(), 3);
    assert_eq!((changes.added, changes.removed), (5, 3));
    let new = &changes.files["new.txt"];
    assert_eq!(new.status, EditFileStatus::Added);
    let files = DiffFile::parse(&new.latest.patch).unwrap();
    assert_eq!(files.len(), 1);
    assert!(new.latest.patch.contains("-first"));
    assert!(new.latest.patch.contains("+latest"));
}

#[cfg(feature = "ui-tests")]
#[gpui_kit::test]
fn session_files_double_click_opens_latest_diff_without_edit_navigation(
    cx: &mut gpui_kit::TestAppContext,
) {
    use conduit::plugins::{builtins, ids::ViewId};
    use gpui_kit::{Modifiers, MouseButton, MouseUpEvent, VisualTestContext, px, size};

    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "session-files");
    let draw = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    };
    cx.simulate_resize(size(px(1400.), px(1100.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().messages = vec![
                edit("first", "src/a.txt", "add", "@@ -0,0 +1 @@\n+first\n"),
                serde_json::from_value(json!({"key":"next", "kind":"User", "text":"Next turn"}))
                    .unwrap(),
                edit(
                    "last",
                    "src/a.txt",
                    "update",
                    "@@ -1 +1 @@\n-first\n+latest\n",
                ),
            ];
            assert!(app.open_right_view(&ViewId::new(builtins::SESSION_DATA).unwrap(), window, cx));
            cx.notify();
        });
    });
    draw(cx);
    draw(cx);
    let position = cx
        .debug_bounds("session-data-file-0")
        .expect("file row")
        .center();
    assert!(cx.debug_bounds("session-data-file-1").is_none());
    assert!(cx.debug_bounds("turn-diff-preview").is_none());
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(
        cx.debug_bounds("turn-diff-preview").is_none(),
        "single click must not open diff"
    );
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 2,
    });
    draw(cx);
    assert!(cx.debug_bounds("turn-diff-preview").is_some());
    assert!(cx.debug_bounds("turn-diff-edit-0").is_none());
    assert!(cx.debug_bounds("turn-diff-previous-edit").is_none());
    assert!(cx.debug_bounds("turn-diff-next-edit").is_none());
    let value = cx.update(|_, cx| {
        let app = app.read(cx);
        let state = app.plugins.session_data.values().next().unwrap().read(cx);
        state.files.value.as_ref().unwrap().clone()
    });
    assert_eq!(value.files.len(), 1);
    assert!(value.files["src/a.txt"].latest.patch.contains("+latest"));
}
