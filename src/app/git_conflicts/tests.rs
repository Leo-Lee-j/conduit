use super::{ConduitApp, Decision, Store, Workspace, git};
use gpui_kit::component::Root;
use gpui_kit::{AppContext, Modifiers, MouseButton, TestAppContext, VisualTestContext};
use std::{sync::atomic::AtomicBool, time::Duration};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.background_executor
        .advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let point = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"))
        .center();
    cx.simulate_mouse_move(point, None, Modifiers::default());
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn conflict_review_preserves_drafts_and_refuses_stale_save(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let repo = git::conflicts::tests::Repo::new();
    repo.conflict();
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = Store {
                workspaces: vec![Workspace {
                    path: repo.path.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.selected_workspace = Some(0);
            app.writable = true;
            app.git.workspace = Some(repo.path.clone());
            app.git.branch_probe = None;
            app.git.snapshot = Some(git::load(&repo.path, &AtomicBool::new(false)).unwrap());
            app.git.repository =
                Some(git::load_repository(&repo.path, &AtomicBool::new(false)).unwrap());
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        Root::new(app, window, cx)
    });
    let app = output.borrow().clone().unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.discover_git_conflicts(window, cx)));
    draw(cx);
    assert!(cx.debug_bounds("git-conflicts-dialog").is_some());
    let browser = cx.update(|_, cx| app.read(cx).git.conflict_browsers[&repo.path].clone());
    for _ in 0..200 {
        draw(cx);
        if cx.update(|_, cx| !browser.read(cx).loading) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|_, cx| {
        let browser = browser.read(cx);
        assert!(!browser.loading, "load timed out");
        assert!(browser.error.is_none(), "{:?}", browser.error);
        assert_eq!(browser.pending(), 1);
    });
    // Opening the dialog should immediately expose the editor, with three aligned
    // panes and the save action inside the dialog even at the test window size.
    let dialog = cx.debug_bounds("git-conflicts-dialog").unwrap();
    let left = cx.debug_bounds("merge-pane-0-0").unwrap();
    let middle = cx.debug_bounds("merge-pane-0-1").unwrap();
    let right = cx.debug_bounds("merge-pane-0-2").unwrap();
    assert!(left.size.width > gpui_kit::px(100.));
    assert!(f32::from(left.size.width - middle.size.width).abs() < 1.);
    assert!(f32::from(middle.size.width - right.size.width).abs() < 1.);
    assert_eq!(left.origin.y, right.origin.y);
    assert!(dialog.contains(&cx.debug_bounds("merge-save").unwrap().center()));
    click(cx, "merge-simple");
    cx.update(|window, cx| {
        browser.update(cx, |browser, cx| {
            assert_eq!(
                browser.pending(),
                1,
                "bulk apply must not resolve a true conflict"
            );
            let index = browser
                .draft()
                .unwrap()
                .blocks
                .iter()
                .position(|b| b.block.conflict)
                .unwrap();
            browser.change(index, 0, Decision::Apply, window, cx);
            browser.change(index, 1, Decision::Ignore, window, cx);
            assert_eq!(browser.pending(), 0);
            browser.change(index, 0, Decision::Apply, window, cx);
            assert_eq!(browser.pending(), 1, "undo restores unresolved status");
            browser.change(index, 0, Decision::Apply, window, cx);
        })
    });
    // Closing and reopening must not silently throw away the user's decisions.
    cx.simulate_keystrokes("escape");
    draw(cx);
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_git_conflicts(window, cx)));
    draw(cx);
    cx.update(|_, cx| assert_eq!(browser.read(cx).pending(), 0));
    let original = std::fs::read(repo.path.join("file[1].txt")).unwrap();
    repo.write("file[1].txt", "external edit\n");
    click(cx, "merge-save");
    for _ in 0..200 {
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_git_actions(window, cx)));
        draw(cx);
        if cx.update(|_, cx| !browser.read(cx).saving) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|_, cx| {
        let browser = browser.read(cx);
        assert!(!browser.saving);
        assert!(browser.error.is_some());
        assert!(browser.draft().is_some());
    });
    assert_eq!(
        std::fs::read_to_string(repo.path.join("file[1].txt")).unwrap(),
        "external edit\n"
    );
    // Once the reviewed snapshot is restored, the same preserved draft can be
    // saved, staged and removed from the remaining-conflict list.
    repo.write("file[1].txt", original);
    click(cx, "merge-save");
    for _ in 0..200 {
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_git_actions(window, cx)));
        draw(cx);
        if cx.update(|_, cx| !browser.read(cx).saving) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|_, cx| {
        let browser = browser.read(cx);
        assert!(!browser.saving);
        assert!(browser.error.is_none(), "{:?}", browser.error);
        assert!(browser.paths.is_empty());
        assert!(browser.selected.is_none());
    });
    assert_eq!(repo.run(&["show", ":file[1].txt"]), "first\nours\nlast\n");
    assert!(repo.run(&["ls-files", "-u"]).is_empty());
    cx.update(|window, _| window.remove_window());
    output.borrow_mut().take();
    drop(browser);
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn disk_edits_are_preserved_and_must_be_reviewed_before_saving(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let repo = git::conflicts::tests::Repo::new();
    repo.conflict();
    let original = std::fs::read_to_string(repo.path.join("file[1].txt")).unwrap();
    let edited = original.replacen("first\n", "manually revised header\n", 1);
    repo.write("file[1].txt", &edited);
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = Store {
                workspaces: vec![Workspace {
                    path: repo.path.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.selected_workspace = Some(0);
            app.writable = true;
            app.git.workspace = Some(repo.path.clone());
            app.git.branch_probe = None;
            app.git.snapshot = Some(git::load(&repo.path, &AtomicBool::new(false)).unwrap());
            app.git.repository =
                Some(git::load_repository(&repo.path, &AtomicBool::new(false)).unwrap());
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        Root::new(app, window, cx)
    });
    let app = output.borrow().clone().unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.discover_git_conflicts(window, cx)));
    draw(cx);
    let browser = cx.update(|_, cx| app.read(cx).git.conflict_browsers[&repo.path].clone());
    for _ in 0..300 {
        draw(cx);
        if cx.update(|_, cx| !browser.read(cx).loading) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let editor = cx.update(|_, cx| {
        let browser = browser.read(cx);
        assert!(!browser.loading);
        assert!(browser.error.is_none(), "{:?}", browser.error);
        let draft = browser.draft().unwrap();
        assert_eq!(draft.blocks.len(), 1);
        assert!(draft.blocks[0].manual);
        assert_eq!(draft.blocks[0].panes[1].read(cx).value().as_ref(), edited);
        draft.blocks[0].panes[1].clone()
    });
    click(cx, "merge-simple");
    cx.update(|window, cx| {
        browser.update(cx, |browser, cx| {
            browser.change(0, 0, Decision::Apply, window, cx);
            assert_eq!(editor.read(cx).value().as_ref(), edited);
            assert_eq!(browser.pending(), 1);
            browser.save_result(cx);
            assert!(!browser.saving, "an unreviewed disk edit must not be saved");
        })
    });
    let resolved = "manually revised header\nmerged by hand\nlast\n";
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.set_value(resolved, window, cx);
            cx.emit(super::InputEvent::Change);
        })
    });
    draw(cx);
    click(cx, "merge-review-0");
    click(cx, "merge-save");
    for _ in 0..300 {
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_git_actions(window, cx)));
        draw(cx);
        if cx.update(|_, cx| !browser.read(cx).saving) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|_, cx| {
        let browser = browser.read(cx);
        assert!(!browser.saving);
        assert!(browser.error.is_none(), "{:?}", browser.error);
        assert!(browser.paths.is_empty());
    });
    assert_eq!(repo.run(&["show", ":file[1].txt"]), resolved);
    cx.update(|window, _| window.remove_window());
    output.borrow_mut().take();
    drop(editor);
    drop(browser);
    drop(app);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn switching_files_keeps_drafts_and_saving_opens_the_next_conflict(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let repo = git::conflicts::tests::Repo::new();
    for path in ["a.txt", "b.txt"] {
        repo.write(path, "base\n");
    }
    repo.commit();
    repo.run(&["checkout", "-qb", "incoming"]);
    for path in ["a.txt", "b.txt"] {
        repo.write(path, "incoming\n");
    }
    repo.commit();
    repo.run(&["checkout", "main"]);
    for path in ["a.txt", "b.txt"] {
        repo.write(path, "current\n");
    }
    repo.commit();
    let merge = std::process::Command::new("git")
        .args(["merge", "incoming"])
        .current_dir(&repo.path)
        .output()
        .unwrap();
    assert!(!merge.status.success());
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = Store {
                workspaces: vec![Workspace {
                    path: repo.path.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.selected_workspace = Some(0);
            app.writable = true;
            app.git.workspace = Some(repo.path.clone());
            app.git.branch_probe = None;
            app.git.snapshot = Some(git::load(&repo.path, &AtomicBool::new(false)).unwrap());
            app.git.repository =
                Some(git::load_repository(&repo.path, &AtomicBool::new(false)).unwrap());
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        Root::new(app, window, cx)
    });
    let app = output.borrow().clone().unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_git_conflicts(window, cx)));
    let browser = cx.update(|_, cx| app.read(cx).git.conflict_browsers[&repo.path].clone());
    let wait_for_load = |cx: &mut VisualTestContext| {
        for _ in 0..300 {
            draw(cx);
            if cx.update(|_, cx| !browser.read(cx).loading) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cx.update(|_, cx| {
            assert!(!browser.read(cx).loading);
            assert!(
                browser.read(cx).error.is_none(),
                "{:?}",
                browser.read(cx).error
            );
        });
    };
    wait_for_load(cx);
    cx.update(|_, cx| assert_eq!(browser.read(cx).selected.as_deref(), Some("a.txt")));
    click(cx, "merge-apply-0-0");
    click(cx, "merge-ignore-0-1");
    cx.update(|_, cx| assert_eq!(browser.read(cx).pending(), 0));
    click(cx, "conflict-file-1");
    wait_for_load(cx);
    cx.update(|_, cx| assert_eq!(browser.read(cx).selected.as_deref(), Some("b.txt")));
    click(cx, "conflict-file-0");
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            browser.read(cx).pending(),
            0,
            "switching files must keep the reviewed draft"
        )
    });
    click(cx, "merge-save");
    for _ in 0..300 {
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_git_actions(window, cx)));
        draw(cx);
        if cx.update(|_, cx| !browser.read(cx).saving && !browser.read(cx).loading) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|_, cx| {
        let browser = browser.read(cx);
        assert!(browser.error.is_none(), "{:?}", browser.error);
        assert_eq!(browser.selected.as_deref(), Some("b.txt"));
        assert_eq!(browser.paths, ["b.txt"]);
        assert_eq!(browser.pending(), 1);
    });
    assert_eq!(repo.run(&["show", ":a.txt"]), "current\n");
    assert!(repo.run(&["ls-files", "-u", "--", "a.txt"]).is_empty());
    assert!(!repo.run(&["ls-files", "-u", "--", "b.txt"]).is_empty());
    cx.update(|window, _| window.remove_window());
    output.borrow_mut().take();
    drop(browser);
    drop(app);
    cx.run_until_parked();
}
