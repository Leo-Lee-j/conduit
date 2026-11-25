use super::{DetailsUi, PullRequestPanel};
use crate::app::ConduitApp;
use crate::app::files::DocumentTab;
use gpui_kit::InputEvent as _;
use gpui_kit::{
    AppContext as _, Entity, Modifiers, MouseButton, SharedString, Task, TestAppContext,
    VisualTestContext, px, size,
};
use std::sync::{Arc, atomic::AtomicBool};

fn fixture<'a>(
    cx: &'a mut TestAppContext,
    name: &str,
) -> (
    Entity<ConduitApp>,
    Entity<PullRequestPanel>,
    &'a mut VisualTestContext,
) {
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, name);
    let panel = cx.update(|window, cx| {
        let mut pr = crate::github::parse(
            &serde_json::json!({
                "number":7, "url":"https://github.com/owner/repo/pull/7",
                "headRefOid":"a".repeat(40), "headRefName":"topic", "baseRefName":"main",
                "title":"Review remote changes", "state":"OPEN"
            })
            .to_string(),
        )
        .unwrap();
        pr.local_head = pr.head.clone();
        let mut details = DetailsUi::default();
        details.install_snapshot(crate::github::details::tests::loaded(&pr));
        let panel = cx.new(|cx| PullRequestPanel {
            host: app.downgrade(),
            workspace: root.clone(),
            focus: cx.focus_handle(),
            document: true,
            details,
            scrolls: Default::default(),
            snapshot: Some(pr),
            requested: None,
            error: None,
            sync_result: None,
            sign_in: Default::default(),
            loading: false,
            shown: 20,
            tab: 2,
            expanded_threads: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _subscription: None,
        });
        app.update(cx, |app, cx| {
            app.startup = None;
            app.github_ui.roots.insert(100, root);
            app.github_ui.panels.insert(100, panel.clone());
            app.open_tabs.push(DocumentTab::PullRequest(100));
            app.select_document(DocumentTab::PullRequest(100), window, cx);
        });
        panel
    });
    (app, panel, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

fn click(cx: &mut VisualTestContext, id: &str) {
    let bounds = cx.update(|window, _| {
        gpui_kit::base::test_support::find(window, &[], &SharedString::from(id.to_owned()).into())
            .unwrap_or_else(|| panic!("missing {id}"))
            .bounds()
    });
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn pane_footers_align_with_editor_and_discard_confirmation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (_, panel, cx) = fixture(cx, "request-footer-alignment");
    cx.simulate_resize(size(px(1600.), px(900.)));
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.open_request_file("src/新 文件.rs", window, cx)
        })
    });
    draw(cx);
    let aligned = |cx: &mut VisualTestContext, diff: bool| {
        let left = cx.debug_bounds("github-files-footer").unwrap();
        let middle = cx.debug_bounds("github-discussion-footer").unwrap();
        assert_eq!(left.size.height, px(48.));
        assert_eq!(middle.size.height, left.size.height);
        assert_eq!(middle.origin.y, left.origin.y);
        if diff {
            let right = cx.debug_bounds("git-diff-footer").unwrap();
            assert_eq!(right.size.height, left.size.height);
            assert_eq!(right.origin.y, left.origin.y);
            let column = cx.debug_bounds("github-diff-column").unwrap();
            assert!(right.right() <= column.right());
        }
    };
    aligned(cx, true);
    cx.update(|window, _| {
        for id in ["github-show-review", "github-publish-comment"] {
            let button = gpui_kit::base::test_support::find(window, &[], &id.into()).unwrap();
            assert_eq!(button.bounds().size.height, px(28.));
        }
    });
    click(cx, "github-publish-comment");
    aligned(cx, true);
    let editor = cx.update(|window, cx| {
        let form = panel
            .read(cx)
            .details
            .composers
            .values()
            .next()
            .unwrap()
            .clone();
        let editor = form.read(cx).editor.as_ref().unwrap().clone();
        editor.update(cx, |input, cx| input.set_value("review draft", window, cx));
        editor
    });
    click(cx, "github-comment-cancel");
    aligned(cx, true);
    click(cx, "github-keep-editing");
    cx.simulate_resize(size(px(850.), px(800.)));
    draw(cx);
    click(cx, "github-show-review");
    aligned(cx, false);
    cx.update(|_, cx| {
        assert_eq!(panel.read(cx).tab, 1);
        assert_eq!(editor.read(cx).value().as_str(), "review draft");
    });
}

#[gpui_kit::test]
fn loading_skeletons_preserve_inline_drafts_and_disable_refresh(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (_, panel, cx) = fixture(cx, "request-loading-skeletons");
    cx.simulate_resize(size(px(1600.), px(1000.)));
    draw(cx);
    click(cx, "github-publish-comment");
    let (pr, snapshot, editor) = cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            let form = panel.details.composers.values().next().unwrap();
            let editor = form.read(cx).editor.as_ref().unwrap().clone();
            editor.update(cx, |input, cx| {
                input.set_value("keep review draft", window, cx)
            });
            panel.loading = true;
            cx.notify();
            (
                panel.snapshot.clone().unwrap(),
                panel.details.snapshot.clone().unwrap(),
                editor,
            )
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("github-workspace-tag").is_some());
    assert!(cx.debug_bounds("github-content-skeleton").is_some());
    assert!(cx.debug_bounds("github-files-skeleton").is_some());
    cx.update(|window, _| {
        let refresh =
            gpui_kit::base::test_support::find(window, &[], &"github-refresh".into()).unwrap();
        assert_eq!(refresh.disabled(), Some(true));
        assert_eq!(refresh.label(), Some("刷新"));
    });
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.snapshot = None;
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("github-metadata-skeleton").is_some());
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.snapshot = Some(pr);
            panel.loading = false;
            panel.details.loading = true;
            panel.details.snapshot = None;
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("github-description-skeleton").is_some());
    assert!(cx.debug_bounds("github-files-skeleton").is_some());
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.details.loading = false;
            panel.details.install_snapshot((*snapshot).clone());
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("github-files-skeleton").is_none());
    assert!(cx.debug_bounds("github-description-skeleton").is_none());
    assert!(cx.debug_bounds("github-file-1").is_some());
    cx.update(|_, cx| assert_eq!(editor.read(cx).value().as_str(), "keep review draft"));
}

#[gpui_kit::test]
fn remote_diff_preserves_drafts_and_tracks_comparison_revision(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (_, panel, cx) = fixture(cx, "remote-request-diff");
    cx.simulate_resize(size(px(1600.), px(1000.)));
    draw(cx);
    click(cx, "github-publish-comment");
    let form = cx.update(|window, cx| {
        let form = panel
            .read(cx)
            .details
            .composers
            .values()
            .next()
            .unwrap()
            .clone();
        form.read(cx)
            .editor
            .as_ref()
            .unwrap()
            .clone()
            .update(cx, |editor, cx| {
                editor.set_value("keep this review draft", window, cx);
            });
        form
    });
    let bounds = cx.debug_bounds("github-file-1").unwrap();
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
    cx.update(|_, cx| assert!(panel.read(cx).details.diff.is_none()));
    let bounds = cx.debug_bounds("github-file-1").unwrap();
    cx.update(|window, cx| {
        window.dispatch_event(
            gpui_kit::MouseDownEvent {
                button: MouseButton::Left,
                position: bounds.center(),
                modifiers: Modifiers::default(),
                click_count: 2,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            gpui_kit::MouseUpEvent {
                button: MouseButton::Left,
                position: bounds.center(),
                modifiers: Modifiers::default(),
                click_count: 2,
            }
            .to_platform_input(),
            cx,
        );
    });
    draw(cx);
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            let view = panel.details.diff.as_ref().unwrap().clone();
            let patch = view.read(cx).last_patch.clone().unwrap();
            assert!(patch.contains("+new remote"));
            assert!(view.read(cx).remote.is_some());
            assert_eq!(view.read(cx).versions().1, "topic · aaaaaaaaaaaa");
            view.update(cx, |view, cx| view.reload(window, cx));
            assert_eq!(view.read(cx).last_patch.as_ref(), Some(&patch));
            let old_view = view.entity_id();
            let mut changed = panel.details.snapshot.as_ref().unwrap().as_ref().clone();
            changed.comparison = "d".repeat(40);
            panel.details.install_snapshot(changed);
            panel.ensure_request_diff(window, cx);
            assert_ne!(panel.details.diff.as_ref().unwrap().entity_id(), old_view);
        });
    });
    click(cx, "github-focus-diff");
    click(cx, "github-restore-discussion");
    click(cx, "github-close-diff");
    cx.update(|_, cx| {
        assert!(panel.read(cx).details.diff.is_none());
        assert_eq!(
            form.read(cx)
                .editor
                .as_ref()
                .unwrap()
                .read(cx)
                .value()
                .as_str(),
            "keep this review draft"
        );
    });
}

#[gpui_kit::test]
fn cached_request_draft_blocks_close_until_discarded(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, panel, cx) = fixture(cx, "request-draft-close");
    draw(cx);
    click(cx, "github-publish-comment");
    let (first, second) = cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            let first = panel.details.composers.values().next().unwrap().clone();
            first
                .read(cx)
                .editor
                .as_ref()
                .unwrap()
                .clone()
                .update(cx, |editor, cx| {
                    editor.set_value("draft on request 7", window, cx);
                });
            let mut pr = panel.snapshot.clone().unwrap();
            pr.number = 8;
            pr.url = "https://github.com/owner/repo/pull/8".into();
            panel.details.reset_request();
            panel
                .details
                .install_snapshot(crate::github::details::tests::loaded(&pr));
            panel.snapshot = Some(pr);
            panel.open_comment(window, cx);
            let second = panel.details.composers["https://github.com/owner/repo/pull/8"].clone();
            assert_ne!(first.entity_id(), second.entity_id());
            assert!(
                second
                    .read(cx)
                    .editor
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .value()
                    .is_empty()
            );
            (first, second)
        })
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.close_documents(&[DocumentTab::PullRequest(100)], window, cx)
        });
        assert!(app.read(cx).github_ui.panels.contains_key(&100));
        assert_eq!(panel.read(cx).snapshot.as_ref().unwrap().number, 7);
        assert!(first.read(cx).confirm_discard);
        assert!(!second.read(cx).confirm_discard);
    });
    draw(cx);
    click(cx, "github-discard");
    cx.update(|window, cx| {
        second.update(cx, |form, _| form.busy = true);
        app.update(cx, |app, cx| {
            app.close_documents(&[DocumentTab::PullRequest(100)], window, cx)
        });
        assert!(app.read(cx).github_ui.panels.contains_key(&100));
        assert_eq!(panel.read(cx).snapshot.as_ref().unwrap().number, 8);
        second.update(cx, |form, _| form.busy = false);
        app.update(cx, |app, cx| {
            app.close_documents(&[DocumentTab::PullRequest(100)], window, cx)
        });
        assert!(!app.read(cx).github_ui.panels.contains_key(&100));
    });
}

#[gpui_kit::test]
fn file_filter_reveals_collapsed_matches_and_narrow_layout_keeps_composer(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (_, panel, cx) = fixture(cx, "request-files-filter");
    cx.simulate_resize(size(px(850.), px(800.)));
    draw(cx);
    click(cx, "github-publish-comment");
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.details.collapsed.insert("src".into());
            assert_eq!(panel.visible_file_rows(cx).len(), 1);
            panel
                .details
                .input
                .as_ref()
                .unwrap()
                .update(cx, |input, cx| input.set_value("新 文件", window, cx));
            assert_eq!(panel.visible_file_rows(cx).len(), 2);
            panel.open_request_file("src/新 文件.rs", window, cx);
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("github-diff-column").is_some());
    assert!(cx.debug_bounds("github-discussion-column").is_none());
    click(cx, "github-restore-discussion");
    assert!(cx.debug_bounds("github-discussion-column").is_some());
    assert!(cx.debug_bounds("github-diff-column").is_none());
    cx.update(|_, cx| {
        assert!(panel.read(cx).details.diff.is_some());
        assert_eq!(panel.read(cx).details.composers.len(), 1);
    });
    cx.update(|_, cx| {
        panel.update(cx, |panel, _| {
            panel.details.files_width = 420.;
            panel.details.discussion_width = 700.;
        })
    });
    cx.simulate_resize(size(px(1600.), px(800.)));
    draw(cx);
    draw(cx);
    let page = cx.debug_bounds("github-document").unwrap();
    let diff = cx.debug_bounds("github-diff-column").unwrap();
    assert!(
        diff.right() <= page.right(),
        "resized columns overflow: {diff:?} within {page:?}"
    );
}
