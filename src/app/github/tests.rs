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

#[gpui_kit::test]
fn pr_document_preserves_workspace_drafts_tabs_and_releases_closed_panels(cx: &mut TestAppContext) {
    use crate::app::files::DocumentTab;
    use gpui_kit::{App, Entity};
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "pr-document-navigation");
    let make_panel = |workspace, cx: &mut App| {
        cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace,
            focus: cx.focus_handle(),
            document: true,
            scrolls: Default::default(),
            snapshot: None,
            requested: None,
            error: Some("Offline fixture".into()),
            sync_result: None,
            sign_in: Default::default(),
            loading: false,
            shown: 40,
            tab: 1,
            expanded_threads: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: None,
        })
    };
    let panels: (Entity<PullRequestPanel>, Entity<PullRequestPanel>) = cx.update(|_, cx| {
        let other = app.read(cx).store.workspaces[1].path.clone();
        (make_panel(root.clone(), cx), make_panel(other, cx))
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            for (id, panel) in [(100, &panels.0), (101, &panels.1)] {
                app.github_ui
                    .roots
                    .insert(id, panel.read(cx).workspace.clone());
                app.github_ui.panels.insert(id, panel.clone());
                app.open_tabs.push(DocumentTab::PullRequest(id));
            }
            app.input.update(cx, |input, cx| {
                input.set_value("keep original draft", window, cx)
            });
            app.open_github_request(None, window, cx);
            assert_eq!(app.active_document(), Some(DocumentTab::PullRequest(100)));
            app.select_document(DocumentTab::Chat(2), window, cx);
            assert!(app.github_ui.active.is_none());
            app.input.update(cx, |input, cx| {
                input.set_value("keep other draft", window, cx)
            });
            app.open_github_request(None, window, cx);
            assert_eq!(app.active_document(), Some(DocumentTab::PullRequest(101)));
            app.select_document(DocumentTab::PullRequest(100), window, cx);
            assert_eq!(app.selected_workspace, Some(0));
            assert_eq!(app.input.read(cx).value().as_str(), "keep original draft");
            assert_eq!(panels.0.read(cx).tab, 1);
            assert_eq!(panels.0.read(cx).shown, 40);
            assert!(panels.0.read(cx).focus.is_focused(window));
            app.select_document(DocumentTab::Settings, window, cx);
            app.close_documents(&[DocumentTab::Settings], window, cx);
            assert_eq!(app.active_document(), Some(DocumentTab::PullRequest(100)));
            app.close_documents(&[DocumentTab::PullRequest(100)], window, cx);
            assert_eq!(app.active_document(), Some(DocumentTab::Chat(1)));
            assert!(!app.github_ui.panels.contains_key(&100));
            app.select_document(DocumentTab::Chat(2), window, cx);
            assert_eq!(app.input.read(cx).value().as_str(), "keep other draft");
            app.select_document(DocumentTab::PullRequest(101), window, cx);
            let other = app.github_ui.roots[&101].clone();
            app.forget_workspace_files(&other);
            assert!(app.github_ui.panels.is_empty());
            assert!(app.github_ui.active.is_none());
            assert!(app.session_control.runs.is_empty());
        })
    });
}

#[gpui_kit::test]
fn pr_document_checks_fit_and_attachment_returns_to_chat_without_losing_the_page(
    cx: &mut TestAppContext,
) {
    use crate::app::files::DocumentTab;
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "pr-document-layout");
    let panel = cx.update(|window, cx| {
        let mut pr = crate::github::parse(&serde_json::json!({
            "number":7,"url":PR,"headRefOid":"a".repeat(40),"headRefName":"fix-ci","baseRefName":"main",
            "title":"修复构建与检查失败", "state":"OPEN", "reviewDecision":"CHANGES_REQUESTED",
            "statusCheckRollup":[{"name":"Windows CI", "conclusion":"FAILURE"}, {"name":"Lint", "conclusion":"SUCCESS"}]
        }).to_string()).unwrap();
        pr.local_head = pr.head.clone();
        pr.threads = Some(snapshot());
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(), workspace: root.clone(), focus: cx.focus_handle(), document: true,
            scrolls: Default::default(), snapshot: Some(pr), requested: None, error: None, sign_in: Default::default(), sync_result: None,
            loading: false, shown: 20, tab: 0, expanded_threads: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)), _task: Task::ready(()), _subscription: None,
        });
        app.update(cx, |app, cx| {
            app.startup = None;
            app.github_ui.roots.insert(100, root.clone());
            app.github_ui.panels.insert(100, panel.clone());
            app.open_tabs.push(DocumentTab::PullRequest(100));
            app.input.update(cx, |input, cx| input.set_value("existing request", window, cx));
            app.select_document(DocumentTab::PullRequest(100), window, cx);
        });
        panel
    });
    for (width, height) in [(850., 500.), (1200., 800.)] {
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)));
        draw(cx);
        let page = cx.debug_bounds("github-document").expect("PR document");
        let content = cx.debug_bounds("github-content").expect("PR list");
        assert!(
            content.size.height >= gpui_kit::px(80.),
            "list too small: {content:?} within {page:?}"
        );
        assert!(page.right() <= gpui_kit::px(width) && page.bottom() <= gpui_kit::px(height));
        cx.update(|window, _| {
            let refresh = gpui_kit::base::test_support::find(window, &[], &"github-refresh".into())
                .unwrap()
                .bounds();
            assert!(refresh.bottom() <= page.bottom());
            assert!(
                gpui_kit::base::test_support::find(window, &[], &"github-publish-comment".into())
                    .is_none()
            );
        });
    }
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.attach("CI evidence".into(), window, cx);
        })
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.active_document(), Some(DocumentTab::Chat(1)));
        assert!(app.input.read(cx).value().contains("existing request"));
        assert!(app.input.read(cx).value().contains("CI evidence"));
        assert!(app.github_ui.panels.contains_key(&100));
        assert!(app.session_control.runs.is_empty());
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_github_request(None, window, cx);
            assert_eq!(app.github_ui.panels.len(), 1);
            assert_eq!(panel.read(cx).tab, 0);
        })
    });
    draw(cx);
    cx.simulate_keystrokes("ctrl-w");
    draw(cx);
    cx.update(|_, cx| assert!(app.read(cx).github_ui.panels.is_empty()));
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn gitlab_document_renders_mr_and_keeps_sync_and_composer_bound_to_gitlab(cx: &mut TestAppContext) {
    use crate::app::files::DocumentTab;
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "gitlab-document");
    let panel = cx.update(|window, cx| {
        let pr = crate::github::gitlab::tests::loaded();
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace: root.clone(),
            focus: cx.focus_handle(),
            document: true,
            scrolls: Default::default(),
            snapshot: Some(pr),
            requested: None,
            error: None,
            sync_result: None,
            sign_in: Default::default(),
            loading: false,
            shown: 20,
            tab: 0,
            expanded_threads: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: None,
        });
        app.update(cx, |app, cx| {
            app.startup = None;
            app.github_ui.roots.insert(100, root.clone());
            app.github_ui.panels.insert(100, panel.clone());
            app.open_tabs.push(DocumentTab::PullRequest(100));
            app.select_document(DocumentTab::PullRequest(100), window, cx);
        });
        assert!(panel.read(cx).tab_title().starts_with("MR !7"));
        panel
    });
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(850.), gpui_kit::px(500.)));
    draw(cx);
    cx.update(|window, cx| {
        assert!(
            gpui_kit::base::test_support::find(window, &[], &"forge-provider".into()).is_some()
        );
        panel.update(cx, |panel, cx| {
            panel.sync_threads(cx);
            panel.tab = 2;
            cx.notify();
        });
        let saved = &app.read(cx).store.reviews.remote;
        assert_eq!(saved.len(), 1);
        assert!(saved[0].pr.contains("/-/merge_requests/7"));
        assert!(saved[0].feedback().unwrap().contains("#note_31"));
    });
    draw(cx);
    click(cx, "github-publish-comment", &[]);
    cx.update(|window, _| {
        assert!(
            gpui_kit::base::test_support::find(window, &[], &"github-comment-submit".into())
                .is_some()
        );
    });
    // Exercise the form without sending any remote mutation.
    click(cx, "github-comment-cancel", &[]);
    cx.update(|_, cx| assert!(app.read(cx).session_control.runs.is_empty()));
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
            requested: None,
            error: None,
            sync_result: None,
            sign_in: Default::default(),
            loading: false,
            shown: 20,
            tab: 1,
            expanded_threads: Default::default(),
            focus: cx.focus_handle(),
            document: false,
            scrolls: Default::default(),
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
            requested: None,
            error: None,
            sync_result: None,
            sign_in: Default::default(),
            loading: true,
            shown: 20,
            tab: 0,
            expanded_threads: Default::default(),
            focus: cx.focus_handle(),
            document: false,
            scrolls: Default::default(),
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

#[gpui_kit::test]
fn failed_check_task_preserves_workspace_drafts_and_user_acceptance(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "pr-check-task");
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(1100.), gpui_kit::px(1100.)));
    let panel = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence = crate::persistence::Persistence::new(root.join("check-tasks.json"));
            app.switch(1, Some(2), window, cx);
            app.input.update(cx, |input, cx| input.set_value("keep other draft", window, cx));
        });
        let mut pr = crate::github::parse(&serde_json::json!({
            "number":7,"url":PR,"headRefOid":"a".repeat(40),"headRefName":"fix-ci",
            "statusCheckRollup":[{"name":"Windows CI","conclusion":"FAILURE","detailsUrl":"https://github.com/example/repo/actions/runs/42"}]
        }).to_string()).unwrap();
        pr.local_head = "b".repeat(40);
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(), workspace: root.clone(), snapshot: Some(pr), requested: None,
            error: None, sign_in: Default::default(), sync_result: None, loading: false, shown: 20,
            tab: 0,
            expanded_threads: Default::default(),
            focus: cx.focus_handle(),
            document: false,
            scrolls: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)), _task: Task::ready(()),
            _subscription: Some(cx.observe(&app, |_, _, cx| cx.notify())),
        });
        let view = panel.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let view=view.clone();
            dialog.title("PR").content(move |content, _, _| content.child(view.clone()))
        });
        panel
    });
    draw(cx);
    let bounds = cx.update(|window, _| {
        gpui_kit::base::test_support::find(window, &[], &("github-check-task", 0usize).into())
            .expect("repair task button")
            .bounds()
    });
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    let source = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_workspace_path(), Some(root.clone()));
            assert!(app.workflow_panels.contains_key(&root));
            app.switch(1, Some(2), window, cx);
            assert_eq!(app.selected_workspace, Some(1));
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(app.input.read(cx).value().as_str(), "keep other draft");
            assert!(app.session_control.runs.is_empty());
            assert_eq!(app.store.workflow.tasks.len(), 1);
            let task = &mut app.store.workflow.tasks[0];
            assert_eq!(task.workspace, root);
            assert_eq!(task.status, crate::workflow::TaskStatus::Planned);
            assert!(task.criteria.iter().all(|c| !c.done));
            assert!(task.sessions.is_empty());
            assert!(task.goal.contains(&"b".repeat(40)));
            task.goal = "Keep edited goal".into();
            task.criteria[0].done = true;
            task.archived = true;
            task.check_source.clone().unwrap()
        })
    });
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.snapshot.as_mut().unwrap().checks[0].state = "SUCCESS".into();
            panel.open_check_task(&source, window, cx);
            assert!(panel.error.is_none());
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            assert_eq!(app.store.workflow.tasks.len(), 1);
            assert_eq!(app.store.workflow.tasks[0].goal, "Keep edited goal");
            assert!(app.store.workflow.tasks[0].criteria[0].done);
            assert!(!app.store.workflow.tasks[0].criteria[1].done);
            assert!(app.store.workflow.tasks[0].archived);
            app.persist();
            app.persistence.flush();
            let restored: Store =
                serde_json::from_slice(&std::fs::read(root.join("check-tasks.json")).unwrap())
                    .unwrap();
            assert_eq!(
                restored.workflow.tasks[0].check_source.as_ref(),
                Some(&source)
            );
            assert_eq!(restored.workflow.tasks[0].goal, "Keep edited goal");
            app.writable = false;
        })
    });
}

#[gpui_kit::test]
fn failed_check_task_rejects_stale_readonly_loading_and_missing_workspace_actions(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "pr-check-task-boundaries");
    cx.update(|window, cx| {
        let pr = crate::github::parse(
            &serde_json::json!({
                "number":7,"url":PR,"headRefOid":"a".repeat(40),
                "statusCheckRollup":[{"name":"Windows CI","conclusion":"FAILURE"}]
            })
            .to_string(),
        )
        .unwrap();
        let source = pr.check_source(0).unwrap();
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace: root.clone(),
            snapshot: Some(pr),
            requested: None,
            error: None,
            sync_result: None,
            sign_in: Default::default(),
            loading: true,
            shown: 20,
            tab: 0,
            expanded_threads: Default::default(),
            focus: cx.focus_handle(),
            document: false,
            scrolls: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: Some(cx.observe(&app, |_, _, cx| cx.notify())),
        });
        panel.update(cx, |panel, cx| {
            panel.open_check_task(&source, window, cx);
            assert!(panel.error.is_some());
            panel.loading = false;
        });
        app.update(cx, |app, _| app.writable = false);
        panel.update(cx, |panel, cx| {
            panel.open_check_task(&source, window, cx);
            assert!(panel.error.is_some());
        });
        app.update(cx, |app, _| app.writable = true);
        panel.update(cx, |panel, cx| {
            panel.workspace = root.join("removed");
            panel.open_check_task(&source, window, cx);
            assert!(panel.error.is_some());
            panel.workspace = root.clone();
            panel.snapshot.as_mut().unwrap().head = "c".repeat(40);
            panel.open_check_task(&source, window, cx);
            assert!(panel.error.is_some());
            panel.snapshot.as_mut().unwrap().head = source.head.clone();
            panel.snapshot.as_mut().unwrap().checks[0].state = "IN_PROGRESS".into();
            panel.open_check_task(&source, window, cx);
            assert!(panel.error.is_some());
        });
        app.update(cx, |app, _| {
            assert!(app.store.workflow.tasks.is_empty());
            assert!(app.session_control.runs.is_empty());
            app.writable = false;
        });
    });
}

#[gpui_kit::test]
fn github_write_form_blocks_removed_readonly_and_uncertain_resubmission(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "github-write-guards");
    cx.update(|window, cx| {
        let mut pr = crate::github::parse(
            &serde_json::json!({
                "number":7, "url":PR, "headRefOid":"a".repeat(40),
                "headRefName":"topic", "state":"OPEN"
            })
            .to_string(),
        )
        .unwrap();
        pr.local_head = "b".repeat(40);
        let parent = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace: root.clone(),
            snapshot: Some(pr.clone()),
            requested: None,
            error: None,
            sync_result: None,
            sign_in: Default::default(),
            loading: false,
            shown: 20,
            tab: 0,
            expanded_threads: Default::default(),
            focus: cx.focus_handle(),
            document: false,
            scrolls: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: Some(cx.observe(&app, |_, _, cx| cx.notify())),
        });
        let editor =
            cx.new(|cx| super::TextareaState::new(window, cx).default_value("Keep this draft"));
        let form = cx.new(|cx| super::GithubCommentPanel {
            host: app.downgrade(),
            parent: parent.downgrade(),
            workspace: root.clone(),
            expected: pr,
            action: crate::github::write::Action::Comment(String::new()),
            editor: Some(editor.clone()),
            error: None,
            busy: false,
            finished: false,
            uncertain: false,
            confirm_discard: false,
            _editor_subscription: None,
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: cx.observe(&app, |_, _, cx| cx.notify()),
        });
        app.update(cx, |app, _| app.writable = false);
        form.update(cx, |form, cx| {
            form.submit(cx);
            assert!(!form.busy);
            assert!(!form.can_close(cx));
            assert!(form.confirm_discard);
            assert!(!form.can_close(cx));
            form.confirm_discard = false;
            form.busy = true;
            assert!(!form.can_close(cx));
            form.busy = false;
        });
        app.update(cx, |app, _| app.writable = true);
        form.update(cx, |form, cx| {
            form.workspace = root.join("removed");
            form.submit(cx);
            assert!(!form.busy);
            form.workspace = root;
            form.uncertain = true;
            form.submit(cx);
            assert!(!form.busy);
            form.uncertain = false;
            form.finished = true;
            form.submit(cx);
            assert!(!form.busy);
            assert!(form.can_close(cx));
        });
        assert_eq!(editor.read(cx).value().as_str(), "Keep this draft");
        assert!(app.read(cx).session_control.runs.is_empty());
        app.update(cx, |app, _| app.writable = false);
    });
}
