use super::*;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn open_both(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, root: &Path) -> (u64, u64) {
    prepare(app, cx, root);
    let ids = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.files_open_beside = false;
            app.open_workspace_file(
                FileKey {
                    workspace: root.into(),
                    relative: "main.txt".into(),
                },
                true,
                window,
                cx,
            );
            let main = app.active_file.unwrap();
            app.store.ui.files_open_beside = true;
            app.open_workspace_file(reference(root), true, window, cx);
            (main, app.files.panes.active.unwrap())
        })
    });
    settle(cx);
    ids
}

fn tick(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            // Freeze metadata polling, proving that updates below come from native
            // notifications rather than the two-second fallback.
            app.files.last_poll = Some(Instant::now());
            app.poll_files(cx);
        })
    });
    settle(cx);
}

fn warm(app: &Entity<ConduitApp>, cx: &mut VisualTestContext) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(400) {
        tick(app, cx);
        thread::sleep(Duration::from_millis(10));
    }
}

fn await_state(
    app: &Entity<ConduitApp>,
    cx: &mut VisualTestContext,
    label: &str,
    start: Instant,
    predicate: impl Fn(&ConduitApp) -> bool,
) {
    loop {
        tick(app, cx);
        if cx.update(|_, cx| predicate(app.read(cx))) {
            eprintln!(
                "content_watch_latency {label}: {} ms",
                start.elapsed().as_millis()
            );
            return;
        }
        assert!(
            start.elapsed() < Duration::from_millis(1500),
            "timed out: {label}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn same_stamp(path: &Path, text: &str) {
    let before = crate::workspace_files::stamp(path).unwrap();
    fs::write(path, text).unwrap();
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before.modified.unwrap()))
        .unwrap();
    assert_eq!(crate::workspace_files::stamp(path).unwrap(), before);
}

#[cfg(windows)]
#[gpui_kit::test]
fn content_watch_updates_visible_snapshots_without_poll_delay(cx: &mut TestAppContext) {
    use crate::protocol::file_proposals::{Input, Update};
    use std::sync::Arc;
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-watch-live");
    let (main, other) = open_both(&app, cx, &root);
    warm(&app, cx);
    let path = root.join("main.txt");
    let reference = root.join("other/reference.txt");
    let start = Instant::now();
    fs::write(&path, "normal").unwrap();
    await_state(&app, cx, "write", start, |app| {
        app.files.documents[&main].text == "normal"
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.file_proposal_event(1, "edit", &root, Update::Started, cx);
            app.file_proposal_event(
                1,
                "edit",
                &root,
                Update::Content(Arc::new(Input {
                    path: "main.txt".into(),
                    old: Some("normal".into()),
                    new: "proposed".into(),
                    complete: false,
                })),
                cx,
            );
            assert!(app.files.documents[&main].preview.read(cx).has_proposal());
        })
    });
    let start = Instant::now();
    same_stamp(&path, "NORMAL");
    await_state(&app, cx, "same-metadata", start, |app| {
        app.files.documents[&main].text == "NORMAL"
    });
    cx.update(|_, cx| {
        assert!(
            !app.read(cx).files.documents[&main]
                .preview
                .read(cx)
                .has_proposal()
        )
    });
    let start = Instant::now();
    let temp = root.join("other/atomic.tmp");
    fs::write(&temp, "atomic").unwrap();
    crate::store::replace_file(&temp, &reference).unwrap();
    await_state(&app, cx, "atomic-replace", start, |app| {
        app.files.documents[&other].text == "atomic"
    });
    let start = Instant::now();
    fs::rename(&reference, root.join("other/renamed.txt")).unwrap();
    await_state(&app, cx, "rename-away", start, |app| {
        app.files.documents[&other].error.is_some()
    });
    cx.update(|_, cx| assert_eq!(app.read(cx).files.documents[&other].text, "atomic"));
    let start = Instant::now();
    fs::write(&reference, "recreated").unwrap();
    await_state(&app, cx, "recreate", start, |app| {
        let document = &app.files.documents[&other];
        document.text == "recreated" && document.error.is_none()
    });
    let start = Instant::now();
    fs::write(&path, "left final").unwrap();
    fs::write(&reference, "right final").unwrap();
    await_state(&app, cx, "both-files", start, |app| {
        app.files.documents[&main].text == "left final"
            && app.files.documents[&other].text == "right final"
    });
    cx.update(|_, cx| assert_eq!(app.read(cx).selected_session, Some(1)));
}

#[gpui_kit::test]
fn content_watch_content_verification_catches_unchanged_metadata(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-watch-fallback");
    let (main, other) = open_both(&app, cx, &root);
    warm(&app, cx);
    // No elapsed debounce and no repeated notification drain: only a forced
    // content verification can detect this same-size, same-timestamp overwrite.
    same_stamp(&root.join("main.txt"), "MAIN FILE\nNEEDLE MAIN\n");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.files.watcher.take_changes(Instant::now()).is_empty());
            app.files.documents.get_mut(&main).unwrap().last_verified =
                Some(Instant::now() - Duration::from_secs(11));
            app.files.last_poll = None;
            app.poll_files(cx);
        })
    });
    settle(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.files.documents[&main].text, "MAIN FILE\nNEEDLE MAIN\n");
        assert_eq!(
            app.files.documents[&other].text,
            "reference file\nneedle reference\n"
        );
        assert!(
            app.files.documents[&main].last_verified.unwrap().elapsed() < Duration::from_secs(2)
        );
    });
}

#[cfg(windows)]
#[gpui_kit::test]
fn content_watch_discards_notifications_on_close_hide_and_workspace_change(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-watch-lifecycle");
    let (main, other) = open_both(&app, cx, &root);
    warm(&app, cx);
    fs::write(root.join("other/reference.txt"), "queued old notification").unwrap();
    // Let the worker queue a notification without allowing the UI to drain it.
    thread::sleep(Duration::from_millis(200));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.close_documents(&[DocumentTab::File(other)], window, cx);
            app.open_workspace_file(reference(&root), true, window, cx);
            assert_ne!(app.files.panes.active, Some(other));
        })
    });
    settle(cx);
    warm(&app, cx);
    let current = cx.update(|_, cx| app.read(cx).files.panes.active.unwrap());
    fs::write(root.join("other/reference.txt"), "hidden edit").unwrap();
    thread::sleep(Duration::from_millis(200));
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
    settle(cx);
    let sequences = cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.visible_file_documents().is_empty());
        (
            app.files.documents[&main].sequence,
            app.files.documents[&current].sequence,
        )
    });
    tick(&app, cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.files.documents.contains_key(&other));
        assert_eq!(
            (
                app.files.documents[&main].sequence,
                app.files.documents[&current].sequence
            ),
            sequences
        );
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.close_settings(window, cx)));
    settle(cx);
    await_state(&app, cx, "reveal", Instant::now(), |app| {
        app.files.documents[&current].text == "hidden edit"
    });
    // Clearing the main workspace hides its file while the cross-workspace
    // reference intentionally stays visible. A queued main-file event is stale.
    fs::write(root.join("main.txt"), "late workspace edit").unwrap();
    thread::sleep(Duration::from_millis(200));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.clear_workspace_selection(window, cx);
        })
    });
    settle(cx);
    let sequence = cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.visible_file_documents(), [current]);
        app.files.documents[&main].sequence
    });
    tick(&app, cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.files.documents[&main].sequence, sequence);
        assert_eq!(app.files.documents[&main].text, "main file\nneedle main\n");
    });
}
