use super::{ConduitApp, Store, Workspace, git};
use gpui_kit::component::Root;
use gpui_kit::{AppContext, Entity, Modifiers, MouseButton, px, size};
use gpui_kit::{TestAppContext, VisualTestContext};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

struct Repo(PathBuf);
impl Repo {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "conduit-branch-view-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Log Tester"]);
        repo.git(&["config", "user.email", "log@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.0.join("history.txt"), format!("{label}\n")).unwrap();
        repo.git(&["add", "--", "history.txt"]);
        repo.git(&["commit", "-qm", label]);
        repo
    }
    fn git(&self, args: &[&str]) -> String {
        let result = std::process::Command::new(git::resolve_git().unwrap())
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8_lossy(&result.stdout).trim().into()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
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
fn assert_commit_visible(cx: &mut VisualTestContext) {
    let panel = cx.debug_bounds("git-log-panel").unwrap();
    let row = cx
        .debug_bounds("git-log-commit-0")
        .expect("loaded history must render a visible commit row");
    let details = cx.debug_bounds("git-log-details").unwrap();
    assert!(
        row.size.width > px(100.),
        "commit row is too narrow: {row:?}"
    );
    assert_eq!(row.size.height, px(30.));
    assert!(
        row.left() >= panel.left() && row.right() <= panel.right() + px(1.),
        "commit row {row:?} is outside panel {panel:?}"
    );
    assert!(
        row.top() >= panel.top() && row.bottom() <= panel.bottom() + px(1.),
        "commit row {row:?} is outside panel {panel:?}"
    );
    assert!(
        row.right() <= details.left() + px(1.) || row.bottom() <= details.top() + px(1.),
        "commit row {row:?} overlaps details {details:?}"
    );
}
fn wait(cx: &mut VisualTestContext, app: &Entity<ConduitApp>) {
    for _ in 0..100 {
        draw(cx);
        let done = cx.update(|_, cx| {
            app.update(cx, |app, cx| app.poll_branches(cx));
            let app = app.read(cx);
            let view = app.branch_manager.as_ref().unwrap().read(cx);
            !view.loading && !view.details_loading && app.git.branch_probe.is_none()
        });
        if done {
            draw(cx);
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("history did not finish loading");
}

#[gpui_kit::test]
fn branch_manager_navigation_filters_favorites_commit_diff_and_workspace_isolation(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let repo = Repo::new("first workspace");
    let other = Repo::new("second workspace");
    repo.git(&["branch", "feature"]);
    std::fs::write(repo.0.join("history.txt"), "updated\n").unwrap();
    repo.git(&["commit", "-qam", "main update"]);
    repo.git(&["tag", "v1"]);
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = output.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = Store {
                workspaces: vec![
                    Workspace {
                        path: repo.0.clone(),
                        sessions: vec![super::super::tests::session()],
                    },
                    Workspace {
                        path: other.0.clone(),
                        sessions: vec![],
                    },
                ],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.writable = false;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.git.workspace = Some(repo.0.clone());
            app.git.branch_probe = None;
            app.git.repository =
                Some(git::load_repository(&repo.0, &AtomicBool::new(false)).unwrap());
            app.input.update(cx, |input, cx| {
                input.set_value("preserved draft", window, cx)
            });
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        Root::new(app, window, cx)
    });
    let app = output.borrow().as_ref().unwrap().clone();
    draw(cx);
    let trigger = cx.debug_bounds("git-branch-trigger").unwrap();
    let settings = cx.debug_bounds("nav-settings").unwrap();
    assert_eq!(trigger.left(), settings.left());
    assert!(trigger.bottom() <= settings.top());
    click(cx, "git-branch-trigger");
    wait(cx, &app);
    assert!(cx.debug_bounds("git-log-panel").is_some());
    assert!(cx.debug_bounds("git-branch-navigation").is_some());
    assert_commit_visible(cx);
    let manager = cx.update(|_, cx| app.read(cx).branch_manager.clone().unwrap());
    // Document shortcuts return to their target instead of leaving the log over it.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_document(super::super::files::DocumentTab::Chat(1), window, cx)
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("git-log-panel").is_none());
    click(cx, "git-branch-trigger");
    wait(cx, &app);

    manager.update(cx, |view, _| {
        assert_eq!(view.history.as_ref().unwrap().commits.len(), 2);
        assert!(view.error.is_none());
    });
    click(cx, "git-log-search");
    cx.simulate_input("main update");
    wait(cx, &app);
    manager.update(cx, |view, _| {
        assert_eq!(view.history.as_ref().unwrap().commits.len(), 1)
    });
    cx.simulate_keystrokes("ctrl-a backspace");
    wait(cx, &app);
    click(cx, "git-branch-search");
    cx.simulate_input("feature");
    draw(cx);
    assert!(cx.debug_bounds("git-branch-row-0").is_none());
    assert!(cx.debug_bounds("git-branch-row-1").is_some());
    click(cx, "git-branch-favorite-1");
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).store.git.favorite_branches[&git::repository_scope(&repo.0)],
            vec!["refs/heads/feature"]
        )
    });
    click(cx, "git-branch-row-1");
    wait(cx, &app);
    assert_commit_visible(cx);
    manager.update(cx, |view, _| {
        assert_eq!(view.query.reference.as_deref(), Some("refs/heads/feature"));
        assert_eq!(view.history.as_ref().unwrap().commits.len(), 1);
    });
    assert_eq!(
        repo.git(&["branch", "--show-current"]),
        "main",
        "selecting the history filter must not check out a branch"
    );
    click(cx, "git-log-file-0");
    assert!(cx.debug_bounds("git-diff-dialog").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("git-diff-dialog").is_none());
    // Creating from HEAD stays available when the current branch is hidden by search.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "git-log-new-branch");
    assert!(cx.debug_bounds("branch-form").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("branch-form").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        })
    });
    for (width, height) in [(1400., 850.), (850., 500.)] {
        cx.simulate_resize(size(px(width), px(height)));
        draw(cx);
        assert_commit_visible(cx);
        let panel = cx.debug_bounds("git-log-panel").unwrap();
        let details = cx.debug_bounds("git-log-details").unwrap();
        assert!(panel.right() <= px(width) + px(1.));
        assert!(
            details.bottom() <= panel.bottom() + px(1.),
            "{details:?} exceeds {panel:?}"
        );
    }
    // A pending query for the old repository must not replace the new repository's history.
    click(cx, "git-log-search");
    cx.simulate_input("second workspace");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(1);
            app.sync_git_workspace(cx);
            cx.notify();
        })
    });
    wait(cx, &app);
    manager.update(cx, |view, _| {
        assert_eq!(view.workspace.as_ref(), Some(&other.0));
        assert_eq!(
            view.history.as_ref().unwrap().commits[0].subject,
            "second workspace"
        );
        assert!(
            view.details
                .as_ref()
                .unwrap()
                .message
                .contains("second workspace")
        );
    });
    click(cx, "nav-workspaces");
    assert!(cx.debug_bounds("git-log-panel").is_none());
    cx.update(|_, cx| assert_eq!(app.read(cx).input.read(cx).value(), "preserved draft"));
    cx.update(|window, _| window.remove_window());
    output.borrow_mut().take();
    drop(manager);
    drop(app);
    cx.run_until_parked();
}
