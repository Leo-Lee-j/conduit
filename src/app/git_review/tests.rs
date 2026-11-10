use super::{
    ConduitApp, DiffLineRange, DiffSide, GitDiffDialog, PermissionLevel, Provider, Status, Store,
    Workspace, git, label,
};
use gpui_kit::{AppContext as _, Task, TestAppContext};
use std::{
    sync::atomic::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};

const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-old\n+new\n";

#[gpui_kit::test]
fn comments_keep_their_scope_and_evidence_and_attachments_only_append_drafts(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let directory = std::env::temp_dir().join(format!(
        "conduit-reviews-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let state = directory.join("state.json");
    let (_, cx) = cx.add_window_view(|window, cx| {
        let host = cx.new(|cx| {
            let first = crate::app::tests::session();
            let mut second = first.clone();
            second.id = 2;
            let store = Store {
                workspaces: vec![
                    Workspace {
                        path: directory.clone(),
                        sessions: vec![first, second],
                    },
                    Workspace {
                        path: directory.join("other"),
                        sessions: vec![],
                    },
                ],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.persistence = crate::persistence::Persistence::new(state.clone());
            app.switch(0, Some(1), window, cx);
            app.input.update(cx, |input, cx| {
                input.set_value("original draft", window, cx)
            });
            app.drafts.insert(2, "destination draft".into());
            app
        });
        let view = cx.new(|cx| {
            let mut diff = GitDiffDialog::new_document(
                directory.clone(),
                git::ChangedFile {
                    index: ' ',
                    worktree: 'M',
                    path: "a.rs".into(),
                    original: None,
                },
                window,
                cx,
            )
            .with_review(host.downgrade(), cx);
            diff.cancelled.store(true, Ordering::Relaxed);
            diff._load_task = Task::ready(());
            diff.finish_load(
                Ok(git::FileDiff {
                    patch: PATCH.into(),
                    notes: vec![],
                }),
                window,
                cx,
            );
            diff
        });
        view.update(cx, |diff, cx| {
            let range = DiffLineRange::new("a.rs", DiffSide::Modified, 1, 1);
            diff.begin_review_comment(range.clone().with_end_side(DiffSide::Original), window, cx);
            assert!(diff.review.anchor.is_none());
            diff.begin_review_comment(range, window, cx);
            diff.review
                .editor
                .as_ref()
                .unwrap()
                .update(cx, |editor, cx| {
                    editor.set_value("verify empty input", window, cx)
                });
            let original_scope = diff.scope;
            diff.scope = git::FileDiffScope::Staged;
            diff.save_review_comment(cx);
            assert!(diff.review.error.is_some());
            assert!(host.read(cx).store.reviews.comments.is_empty());
            assert!(diff.review.editor.is_some());
            diff.scope = original_scope;
            diff.save_review_comment(cx);
            assert!(diff.review.error.is_none());
            assert_eq!(
                host.read(cx).store.reviews.comments[0].anchor.snippet,
                "new"
            );
            diff.toggle_reviewed(cx);
            assert!(
                host.read(cx)
                    .store
                    .reviews
                    .is_viewed(&diff.review_target(), &diff.review.fingerprint)
            );
            diff.finish_load(
                Ok(git::FileDiff {
                    patch: PATCH.replace("+new", "+changed"),
                    notes: vec![],
                }),
                window,
                cx,
            );
            assert!(
                !host
                    .read(cx)
                    .store
                    .reviews
                    .is_viewed(&diff.review_target(), &diff.review.fingerprint)
            );
            diff.attach_review_feedback(Some(2), window, cx);
            assert!(diff.review.error.is_none());
            host.update(cx, |app, cx| {
                assert_eq!(app.selected_session, Some(2));
                assert_eq!(app.drafts[&1], "original draft");
                let text = app.input.read(cx).value();
                assert!(text.starts_with("destination draft\n\n"));
                assert!(text.contains("verify empty input"));
                assert!(text.contains(label("review.outdated")));
                assert!(text.contains("    new"));
                assert!(app.session_control.runs.is_empty());
                assert_eq!(app.store.reviews.comments[0].status, Status::Open);
                let previous = text.to_string();
                assert!(
                    app.append_review_draft(
                        &directory.join("other"),
                        Some(2),
                        None,
                        "wrong workspace",
                        window,
                        cx
                    )
                    .is_err()
                );
                assert_eq!(app.input.read(cx).value().as_ref(), previous);
                app.writable = false;
                assert!(
                    app.append_review_draft(&directory, Some(2), None, "read only", window, cx)
                        .is_err()
                );
                app.writable = true;
                app.persistence.flush();
                let restored: Store =
                    serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
                assert_eq!(restored.reviews.comments.len(), 1);
                assert_eq!(restored.reviews.comments[0].text, "verify empty input");
            });
            diff.prepare_ai_review(Provider::Codex, window, cx);
            host.update(cx, |app, cx| {
                let id = app.selected_session.unwrap();
                assert_ne!(id, 2);
                assert_eq!(
                    app.session(id).unwrap().permission,
                    PermissionLevel::ReadOnly
                );
                assert!(app.input.read(cx).value().contains("+changed"));
                assert!(app.drafts[&2].starts_with("destination draft"));
                assert!(app.session_control.runs.is_empty());
                let request = &app.store.reviews.ai[0];
                assert_eq!(request.session, id);
                assert_eq!(
                    request.session_created_at,
                    app.session(id).unwrap().created_at
                );
                assert_eq!(request.patch, PATCH.replace("+new", "+changed"));
                assert!(app.input.read(cx).value().contains(&request.key));
                assert!(app.input.read(cx).value().contains("\"findings\""));
                app.persistence.flush();
                let restored: Store =
                    serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
                assert_eq!(restored.reviews.ai[0].key, request.key);
                app.writable = false;
            });
        });
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let directory = directory.canonicalize().unwrap();
    assert!(directory.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    for attempt in 0..50 {
        match std::fs::remove_dir_all(&directory) {
            Ok(()) => break,
            Err(error) if attempt == 49 => panic!("fixture cleanup failed: {error}"),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
}

mod structured {
    use super::*;
    use crate::app::queue::tests::{fixture, running};
    use crate::app::{Event, ItemKind};
    use crate::review::{
        Target,
        ai::{Rejection, ReportState, Review},
    };
    use serde_json::json;

    fn prepare(app: &mut ConduitApp, workspace: &std::path::Path) -> String {
        app.persistence =
            crate::persistence::Persistence::new(workspace.join("ai-review-state.json"));
        let mut request = Review::capture(
            Target {
                workspace: workspace.into(),
                path: "a.rs".into(),
                source: "unstaged".into(),
            },
            PATCH,
        )
        .unwrap();
        request.session = 1;
        request.session_created_at = app.session(1).unwrap().created_at;
        app.session_mut(1).unwrap().messages = serde_json::from_value(json!([
            {"key":"turn-0-user","kind":"User","text":request.instructions()}
        ]))
        .unwrap();
        let text = json!({"version":1,"review_id":request.key,"fingerprint":request.fingerprint,
            "findings":[{"path":"a.rs","side":"new","start_line":1,"end_line":1,
                "priority":"P2","title":"Handle empty input","body":"Empty input panics; add an empty-input regression case."}]}).to_string();
        app.store.reviews.ai.push(request);
        text
    }

    #[gpui_kit::test]
    fn structured_review_completion_preserves_evidence_drafts_and_resolved_status(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = fixture(cx, "structured-review-completion");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let text = prepare(app, &root);
                let sender = running(app, 1);
                app.switch(1, Some(2), window, cx);
                app.input
                    .update(cx, |input, cx| input.set_value("other draft", window, cx));
                std::fs::write(root.join("a.rs"), "changed after review").unwrap();
                sender
                    .send(Event::Text {
                        key: "reply".into(),
                        text: text.clone(),
                    })
                    .unwrap();
                sender
                    .send(Event::Finished {
                        error: None,
                        cancelled: false,
                    })
                    .unwrap();
                app.poll_streams(window, cx);
                assert_eq!(app.store.reviews.comments.len(), 1);
                assert_eq!(app.store.reviews.comments[0].anchor.snippet, "new");
                assert_eq!(app.store.reviews.comments[0].status, Status::NeedsReview);
                assert!(app.store.reviews.viewed.is_empty());
                assert!(matches!(
                    app.store.reviews.ai[0].state,
                    ReportState::Imported(1)
                ));
                assert_eq!(app.selected_session, Some(2));
                assert_eq!(app.input.read(cx).value().as_ref(), "other draft");
                assert!(
                    app.session(1)
                        .unwrap()
                        .messages
                        .iter()
                        .any(|m| m.kind == ItemKind::Assistant && m.text == text)
                );
                app.store.reviews.comments[0].status = Status::Resolved;
                app.store.reviews =
                    serde_json::from_value(serde_json::to_value(&app.store.reviews).unwrap())
                        .unwrap();
                app.finish_ai_review(1, "turn-0", false, cx);
                assert_eq!(app.store.reviews.comments.len(), 1);
                assert_eq!(app.store.reviews.comments[0].status, Status::Resolved);
                let origin = app.store.reviews.comments[0].ai.clone().unwrap();
                app.store.reviews.ai.clear();
                assert!(
                    app.open_ai_review_session(
                        &root,
                        origin.session,
                        origin.session_created_at,
                        window,
                        cx
                    )
                    .is_ok()
                );
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.drafts[&2], "other draft");
                assert!(
                    app.open_ai_review_session(
                        &root,
                        origin.session,
                        origin.session_created_at + 1,
                        window,
                        cx
                    )
                    .is_err()
                );
                app.persistence.flush();
                app.writable = false;
            })
        });
    }

    #[gpui_kit::test]
    fn structured_review_failed_cancelled_and_unrelated_turns_never_import(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = fixture(cx, "structured-review-failures");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let text = prepare(app, &root);
                for cancelled in [false, true] {
                    let sender = running(app, 1);
                    sender
                        .send(Event::Text {
                            key: "reply".into(),
                            text: text.clone(),
                        })
                        .unwrap();
                    sender
                        .send(Event::Finished {
                            error: (!cancelled).then(|| "failed".into()),
                            cancelled,
                        })
                        .unwrap();
                    app.poll_streams(window, cx);
                    assert!(app.store.reviews.comments.is_empty());
                    assert!(matches!(
                        app.store.reviews.ai[0].state,
                        ReportState::Rejected(Rejection::Incomplete)
                    ));
                }
                app.finish_ai_review(1, "turn-1", false, cx);
                assert!(matches!(
                    app.store.reviews.ai[0].state,
                    ReportState::Rejected(Rejection::NoReply)
                ));
                // A tool's JSON is never treated as an assistant response.
                let sender = running(app, 1);
                app.session_control.runs.get_mut(&1).unwrap().prefix = "turn-1".into();
                sender
                    .send(Event::Tool {
                        key: "tool".into(),
                        title: "tool".into(),
                        detail: text.clone(),
                        status: "done".into(),
                    })
                    .unwrap();
                sender
                    .send(Event::Finished {
                        error: None,
                        cancelled: false,
                    })
                    .unwrap();
                app.poll_streams(window, cx);
                assert!(matches!(
                    app.store.reviews.ai[0].state,
                    ReportState::Rejected(Rejection::NoReply)
                ));
                // Native commands must not trigger structured review processing.
                let sender = running(app, 1);
                app.session_control.runs.get_mut(&1).unwrap().intent =
                    crate::commands::RequestIntent::Discover;
                sender
                    .send(Event::Text {
                        key: "reply".into(),
                        text: text.clone(),
                    })
                    .unwrap();
                sender
                    .send(Event::Finished {
                        error: None,
                        cancelled: false,
                    })
                    .unwrap();
                app.poll_streams(window, cx);
                assert!(app.store.reviews.comments.is_empty());
                // Binding uses the workspace, creation timestamp and original user request.
                let target = app.store.reviews.ai[0].target.workspace.clone();
                app.store.reviews.ai[0].target.workspace = root.join("other");
                app.finish_ai_review(1, "turn-0", false, cx);
                assert!(app.store.reviews.comments.is_empty());
                app.store.reviews.ai[0].target.workspace = target;
                app.store.reviews.ai[0].session_created_at += 1;
                app.finish_ai_review(1, "turn-0", false, cx);
                assert!(app.store.reviews.comments.is_empty());
                app.store.reviews.ai[0].session_created_at -= 1;
                let original = app.session(1).unwrap().messages[0].text.clone();
                app.session_mut(1).unwrap().messages[0].text = "different question".into();
                app.finish_ai_review(1, "turn-0", false, cx);
                assert!(app.store.reviews.comments.is_empty());
                app.session_mut(1).unwrap().messages[0].text = original;
                let sender = running(app, 1);
                app.session_control.runs.get_mut(&1).unwrap().prefix = "turn-2".into();
                sender
                    .send(Event::Text {
                        key: "reply".into(),
                        text,
                    })
                    .unwrap();
                sender
                    .send(Event::Finished {
                        error: None,
                        cancelled: false,
                    })
                    .unwrap();
                app.poll_streams(window, cx);
                assert_eq!(app.store.reviews.comments.len(), 1);
                assert_eq!(
                    app.store.reviews.comments[0].ai.as_ref().unwrap().message,
                    "turn-2-reply"
                );
                app.persistence.flush();
                app.writable = false;
            })
        });
    }

    #[gpui_kit::test]
    fn structured_review_capacity_rejects_before_creating_or_switching_a_draft(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = fixture(cx, "structured-review-capacity");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                prepare(app, &root);
                let review = app.store.reviews.ai[0].clone();
                app.store.reviews.ai = vec![review.clone(); 50];
                app.input
                    .update(cx, |input, cx| input.set_value("keep draft", window, cx));
                let count = app.store.workspaces[0].sessions.len();
                assert!(
                    app.prepare_structured_review(
                        review,
                        Provider::Claude,
                        "new review",
                        window,
                        cx
                    )
                    .is_err()
                );
                assert_eq!(app.store.workspaces[0].sessions.len(), count);
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.input.read(cx).value().as_ref(), "keep draft");
                assert!(app.session_control.runs.is_empty());
                app.persistence.flush();
                app.writable = false;
            })
        });
    }
}
