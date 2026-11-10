use super::{PullRequestPanel, Store};
use crate::app::queue::tests::fixture;
use crate::github::threads::tests::{PR, snapshot};
use crate::review::Status;
use gpui_kit::ParentElement as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, Modifiers, MouseButton, SharedString, Task, TestAppContext, VisualTestContext,
};
use std::sync::{Arc, atomic::AtomicBool};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

fn click(cx: &mut VisualTestContext, id: &str, ancestors: &[gpui_kit::ElementId]) {
    let bounds = cx.update(|window, _| {
        gpui_kit::base::test_support::find(
            window,
            ancestors,
            &SharedString::from(id.to_owned()).into(),
        )
        .unwrap_or_else(|| panic!("missing {id}"))
        .bounds()
    });
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn pr_review_sync_and_attachments_keep_the_original_workspace_and_saved_status(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "pr-review-sync");
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(1000.), gpui_kit::px(1000.)));
    let panel = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence = crate::persistence::Persistence::new(root.join("reviews-state.json"));
            app.switch(1, Some(2), window, cx);
            app.input.update(cx, |input, cx| {
                input.set_value("other workspace draft", window, cx)
            });
        });
        let mut pr = crate::github::parse(
            &serde_json::json!({"number":7,"url":PR,"headRefOid":"a".repeat(40)}).to_string(),
        )
        .unwrap();
        pr.local_head = pr.head.clone();
        pr.threads = Some(snapshot());
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace: root.clone(),
            snapshot: Some(pr),
            error: None,
            sync_result: None,
            loading: false,
            shown: 20,
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: Some(cx.observe(&app, |_, _, cx| cx.notify())),
        });
        let view = panel.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title("PR")
                .content(move |content, _, _| content.child(view.clone()))
        });
        panel
    });
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|window, _| {
        let button =
            gpui_kit::base::test_support::find(window, &[], &"github-sync-threads".into()).unwrap();
        assert_eq!(button.disabled(), Some(true));
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = true;
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "github-sync-threads", &[]);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.store.reviews.remote.len(), 1);
            assert_eq!(app.store.reviews.remote[0].workspace, root);
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(app.input.read(cx).value().as_ref(), "other workspace draft");
            assert!(app.session_control.runs.is_empty());
            app.store.reviews.remote[0].set_status(Status::Resolved);
        })
    });
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.sync_threads(cx);
            let snapshot = panel.snapshot.as_mut().unwrap().threads.as_mut().unwrap();
            snapshot.requested_at += 1;
            snapshot.fetched_at += 1;
            snapshot.threads[0].is_outdated = true;
            snapshot.threads[0].comments[0].body = "Updated remote comment".into();
            panel.sync_threads(cx);
        })
    });
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let saved = &app.store.reviews.remote[0];
            assert_eq!(saved.status, Status::Resolved);
            assert!(saved.changed());
            app.persistence.flush();
            let restored: Store =
                serde_json::from_slice(&std::fs::read(root.join("reviews-state.json")).unwrap())
                    .unwrap();
            assert_eq!(restored.reviews.remote[0].status, Status::Resolved);
            assert!(restored.reviews.remote[0].changed());
        })
    });
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.attach_thread("T1", window, cx)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_workspace, Some(0));
            assert_ne!(app.selected_session, Some(2));
            assert_eq!(app.drafts[&2], "other workspace draft");
            assert!(
                app.input
                    .read(cx)
                    .value()
                    .contains("Updated remote comment")
            );
            assert!(app.input.read(cx).value().contains(&"b".repeat(40)));
            assert!(app.session_control.runs.is_empty());
            app.open_review_inbox(window, cx);
        })
    });
    draw(cx);
    // Removing a local copy has no GitHub write path and leaves the draft intact.
    click(cx, "remote-remove", &[]);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.store.reviews.remote.is_empty());
            assert!(
                app.input
                    .read(cx)
                    .value()
                    .contains("Updated remote comment")
            );
            app.persistence.flush();
            app.writable = false;
        })
    });
}

#[gpui_kit::test]
fn pr_review_loading_missing_workspaces_and_readonly_state_block_sync(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "pr-review-boundaries");
    cx.update(|_, cx| {
        let mut pr = crate::github::parse(
            &serde_json::json!({"number":7,"url":PR,"headRefOid":"a".repeat(40)}).to_string(),
        )
        .unwrap();
        pr.threads = Some(snapshot());
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace: root.clone(),
            snapshot: Some(pr),
            error: None,
            sync_result: None,
            loading: true,
            shown: 20,
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: Some(cx.observe(&app, |_, _, cx| cx.notify())),
        });
        panel.update(cx, |panel, cx| panel.sync_threads(cx));
        assert!(app.read(cx).store.reviews.remote.is_empty());
        app.update(cx, |app, _| app.writable = false);
        panel.update(cx, |panel, cx| {
            panel.loading = false;
            panel.sync_threads(cx);
            assert!(panel.error.is_some());
        });
        assert!(app.read(cx).store.reviews.remote.is_empty());
        app.update(cx, |app, _| app.writable = true);
        panel.update(cx, |panel, cx| {
            panel.workspace = root.join("missing");
            panel.sync_threads(cx);
            assert!(panel.error.is_some());
        });
        assert!(app.read(cx).store.reviews.remote.is_empty());
        app.update(cx, |app, _| app.writable = false);
    });
}
