use super::{ConduitApp, DocumentTab, FileKey, FileViewSnapshot, ItemKind, Pane};
use crate::app::queue::tests::fixture;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, px, size};
use std::path::{Path, PathBuf};

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.simulate_next_frame(cx);
            let _ = window.draw(cx);
        });
    }
}

fn prepare(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, root: &Path) {
    std::fs::create_dir_all(root.join("other")).unwrap();
    std::fs::write(root.join("main.txt"), "main file\nneedle main\n").unwrap();
    std::fs::write(
        root.join("other/reference.txt"),
        "reference file\nneedle reference\n",
    )
    .unwrap();
    cx.simulate_resize(size(px(1800.), px(1000.)));
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = false;
            app.open_tabs = vec![DocumentTab::Chat(1)];
            app.store.ui.files_open_beside = true;
        })
    });
    settle(cx);
}

fn reference(root: &Path) -> FileKey {
    FileKey {
        workspace: root.join("other"),
        relative: "reference.txt".into(),
    }
}

#[gpui_kit::test]
fn content_panes_cross_workspace_reference_preserves_send_target(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-target");
    prepare(&app, cx, &root);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input.update(cx, |input, cx| {
                input.set_value("keep this main draft", window, cx)
            });
            let target = app.composer_target();
            let permission = app.pending_permission;
            let engine = app.new_session_provider;
            app.open_workspace_file(reference(&root), true, window, cx);
            assert_eq!(app.composer_target(), target);
            assert_eq!(app.pending_permission, permission);
            assert_eq!(app.new_session_provider, engine);
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));
            assert!(app.active_file.is_none());
            assert_eq!(app.input.read(cx).value(), "keep this main draft");
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("content-main").is_some());
    assert!(cx.debug_bounds("content-reference").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.session_is_visible(1));
            assert!(app.session_data_available());
            let id = app.files.panes.active.unwrap();
            assert!(app.files.documents[&id].text.contains("reference file"));
            assert_eq!(
                app.document_tabs_for(Pane::Main),
                vec![DocumentTab::Chat(1)]
            );
            assert_eq!(
                app.document_tabs_for(Pane::Reference),
                vec![DocumentTab::File(id)]
            );
            // Exercise the actual send entry point using the fixture's offline CLI.
            app.writable = true;
            app.persistence.flush();
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.send_ordinary_prompt(window, cx);
            assert!(
                app.session_control.runs.contains_key(&1),
                "{:?}",
                app.message
            );
            assert!(!app.session_control.runs.contains_key(&2));
            assert_eq!(app.session_control.runs[&1].worktree, root);
            assert!(
                app.session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .any(|item| item.kind == ItemKind::User && item.text == "keep this main draft")
            );
            app.close_documents(&app.document_tabs_for(Pane::Reference), window, cx);
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.open_tabs, vec![DocumentTab::Chat(1)]);
            assert!(app.files.documents.is_empty());
        })
    });
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_moves_restore_main_and_narrow_navigation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-moves");
    prepare(&app, cx, &root);
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input
                .update(cx, |input, cx| input.set_value("alpha draft", window, cx));
            app.open_workspace_file(reference(&root), true, window, cx);
            app.files.panes.active.unwrap()
        })
    });
    settle(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.files.documents[&id]
                .preview
                .update(cx, |preview, cx| preview.open_find(window, cx));
            app.move_file_to_pane(id, Pane::Main, window, cx);
            assert_eq!(app.active_file, Some(id));
            assert_eq!(app.selected_session, Some(2));
            assert!(!app.files.panes.split);
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("file-find-bar").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.move_file_to_pane(id, Pane::Reference, window, cx);
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));
            assert!(app.active_file.is_none());
            assert_eq!(app.files.panes.active, Some(id));
            assert_eq!(app.input.read(cx).value(), "alpha draft");
            assert_eq!(app.files.documents.len(), 1);
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("file-find-bar").is_some());
    // Focus outside either content pane must retain the last content choice.
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let outside = cx.focus_handle();
            outside.focus(window, cx);
            app.update_content_pane_layout(500., window, cx);
            assert!(!app.main_pane_visible());
            assert!(app.reference_pane_visible());
            assert!(!app.session_is_visible(1));
            app.show_content_pane(Pane::Main, window, cx);
            assert!(app.main_pane_visible());
            assert!(!app.reference_pane_visible());
            app.update_content_pane_layout(1200., window, cx);
            assert!(app.main_pane_visible() && app.reference_pane_visible());
            app.show_content_pane(Pane::Reference, window, cx);
            app.open_settings(window, cx);
            assert!(app.visible_file_documents().is_empty());
            app.close_settings(window, cx);
            app.update_content_pane_layout(500., window, cx);
            assert_eq!(app.files.panes.last, Pane::Reference);
            assert_eq!(app.files.panes.active, Some(id));
            assert_eq!(app.selected_session, Some(1));
        })
    });
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_refresh_both_files_retry_and_reject_closed_results(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-refresh");
    prepare(&app, cx, &root);
    let (main, reference_id) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.files_open_beside = false;
            app.open_workspace_file(
                FileKey {
                    workspace: root.clone(),
                    relative: "main.txt".into(),
                },
                true,
                window,
                cx,
            );
            let main = app.active_file.unwrap();
            app.store.ui.files_open_beside = true;
            app.open_workspace_file(reference(&root), true, window, cx);
            (main, app.files.panes.active.unwrap())
        })
    });
    settle(cx);
    std::fs::write(root.join("main.txt"), "main updated with more text").unwrap();
    std::fs::write(
        root.join("other/reference.txt"),
        "reference updated with more text",
    )
    .unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.files.last_poll = None;
            app.poll_files(cx);
        })
    });
    settle(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(
                app.files.documents[&main].text,
                "main updated with more text"
            );
            assert_eq!(
                app.files.documents[&reference_id].text,
                "reference updated with more text"
            );
            assert_eq!(
                FileViewSnapshot::new(app, Pane::Main).document.unwrap().0,
                main
            );
            assert_eq!(
                FileViewSnapshot::new(app, Pane::Reference)
                    .document
                    .unwrap()
                    .0,
                reference_id
            );
            app.load_file(reference_id, cx);
            app.load_file(reference_id, cx); // Another notification while reading.
            assert!(app.files.documents[&reference_id].dirty);
        })
    });
    std::fs::write(root.join("other/reference.txt"), "newest version").unwrap();
    settle(cx);
    std::fs::remove_file(root.join("other/reference.txt")).unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.load_file(reference_id, cx)));
    settle(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        let document = &app.files.documents[&reference_id];
        assert!(document.error.is_some());
        assert_eq!(document.text, "newest version");
        assert!(document.retry_at.is_some());
    });
    std::fs::write(root.join("other/reference.txt"), "recreated").unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.files.documents.get_mut(&reference_id).unwrap().retry_at = None;
            app.files.last_poll = None;
            app.poll_files(cx);
        })
    });
    settle(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.files.documents[&reference_id].text, "recreated");
            assert!(app.files.documents[&reference_id].error.is_none());
            app.load_file(reference_id, cx);
            app.close_documents(&[DocumentTab::File(reference_id)], window, cx);
            app.open_workspace_file(reference(&root), true, window, cx);
            assert_ne!(app.files.panes.active, Some(reference_id));
        })
    });
    settle(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.files.documents.contains_key(&reference_id));
        assert_eq!(
            app.files.documents[&app.files.panes.active.unwrap()].text,
            "recreated"
        );
        assert_eq!(app.active_file, Some(main));
    });
}

#[gpui_kit::test]
fn content_panes_preview_slots_capacity_and_image_failure_keep_visible_documents(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-capacity");
    prepare(&app, cx, &root);
    let image_path = root.join("other/picture.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]))
        .save(&image_path)
        .unwrap();
    let (main, image_id) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.files_open_beside = false;
            app.open_workspace_file(
                FileKey {
                    workspace: root.clone(),
                    relative: "main.txt".into(),
                },
                false,
                window,
                cx,
            );
            let main = app.active_file.unwrap();
            app.store.ui.files_open_beside = true;
            app.open_workspace_file(reference(&root), false, window, cx);
            let old = app.files.panes.active.unwrap();
            app.open_workspace_file(
                FileKey {
                    workspace: root.join("other"),
                    relative: "picture.png".into(),
                },
                false,
                window,
                cx,
            );
            assert!(!app.files.documents.contains_key(&old));
            assert_eq!(app.files.preview, Some(main));
            assert_eq!(app.active_file, Some(main));
            let image_id = app.files.panes.active.unwrap();
            app.pin_file(image_id);
            (main, image_id)
        })
    });
    settle(cx);
    std::fs::remove_file(&image_path).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(app.files.documents[&image_id].image.is_some());
            app.load_file(image_id, cx);
            assert!(app.files.documents[&image_id].image.is_some());
        })
    });
    settle(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.files.documents[&image_id].error.is_some());
            assert!(app.files.documents[&image_id].image.is_some());
            // The original Main preview remains visible while Reference exceeds its budget.
            for n in 0..22 {
                let relative = PathBuf::from(format!("item-{n}.txt"));
                std::fs::write(root.join("other").join(&relative), format!("item {n}")).unwrap();
                app.open_workspace_file(
                    FileKey {
                        workspace: root.join("other"),
                        relative,
                    },
                    true,
                    window,
                    cx,
                );
                assert!(app.files.documents.contains_key(&main));
            }
            let selected = app.files.panes.active.unwrap();
            assert!(app.files.documents.contains_key(&selected));
            assert!(app.files.documents.len() <= 20);
            app.close_documents(&app.document_tabs_for(Pane::Reference), window, cx);
            assert_eq!(app.active_file, Some(main));
            assert_eq!(app.files.preview, Some(main));
            assert!(!app.files.panes.split);
            assert!(app.open_tabs.contains(&DocumentTab::Chat(1)));
        })
    });
    settle(cx);
}

#[gpui_kit::test]
fn content_panes_chat_citation_reveals_the_reference_not_main(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-citation");
    prepare(&app, cx, &root);
    std::fs::write(
        root.join("other/reference.txt"),
        (1..=240)
            .map(|line| format!("line {line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let (main, reference) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.files_open_beside = false;
            app.open_workspace_file(
                FileKey {
                    workspace: root.clone(),
                    relative: "main.txt".into(),
                },
                true,
                window,
                cx,
            );
            let main = app.active_file.unwrap();
            app.store.ui.files_open_beside = true;
            app.open_chat_link(1, "other/reference.txt#L200", window, cx);
            (main, app.files.panes.active.unwrap())
        })
    });
    settle(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        let scroll = &app.files.documents[&reference].preview.read(cx).scroll;
        assert!(scroll.offset().y < -scroll.bounds().size.height);
        assert_eq!(
            app.files.documents[&main]
                .preview
                .read(cx)
                .scroll
                .offset()
                .y,
            px(0.)
        );
        assert_eq!(app.selected_session, Some(1));
    });
}

#[gpui_kit::test]
fn content_review_tool_matching_and_snapshot_replay_do_not_replace_current_file(
    cx: &mut TestAppContext,
) {
    use crate::domain::ChatItemData;
    use crate::message_status::ToolStatus;
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-review-evidence");
    prepare(&app, cx, &root);
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_workspace_file(reference(&root), true, window, cx);
            app.files.panes.active.unwrap()
        })
    });
    settle(cx);
    let before = "reference file\nneedle reference\n";
    let after = "reference file\nupdated reference\n";
    let mut item: crate::domain::ChatItem = ChatItemData {
        key: "verified-edit".into(), kind: ItemKind::Tool, text: "Edit".into(),
        input: serde_json::json!({"file_path":"reference.txt", "old_string":"needle", "new_string":"updated"}).to_string(),
        status: ToolStatus::Running, detail: String::new(), created_at: 0, reasoning_summary: None, elapsed_ms: None, approval: None, model: None
    }.into();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.observe_file_tool(1, &root.join("other"), &item, cx)
        })
    });
    std::fs::write(root.join("other/reference.txt"), after).unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.load_file(id, cx)));
    settle(cx);
    cx.update(|_, cx| {
        assert!(app.read(cx).files.documents[&id].matched_tool.is_none());
    });
    item.status = ToolStatus::Completed;
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.observe_file_tool(1, &root.join("other"), &item, cx)
        })
    });
    settle(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let document = &app.files.documents[&id];
            assert_eq!(document.matched_tool, Some((1, "verified-edit".into())));
            assert_eq!(document.changes.as_ref().unwrap().hunks.len(), 1);
            document.preview.update(cx, |preview, cx| {
                preview.start_replay(cx);
                preview.pause_replay(cx);
                assert_eq!(preview.visible_source(), before);
            });
            assert_eq!(document.text, after);
        })
    });
    std::fs::write(
        root.join("other/reference.txt"),
        "external newest version\n",
    )
    .unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.load_file(id, cx)));
    settle(cx);
    cx.update(|_, cx| {
        let document = &app.read(cx).files.documents[&id];
        assert!(document.matched_tool.is_none());
        assert_eq!(
            document.preview.read(cx).visible_source(),
            "external newest version\n"
        );
    });
    // Two simultaneous tools describing the same write cannot establish ownership.
    item.key = "collision-a".into();
    item.text = "Write".into();
    item.status = ToolStatus::Running;
    item.input =
        serde_json::json!({"file_path":"reference.txt","content":"two writers\n"}).to_string();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.observe_file_tool(1, &root.join("other"), &item, cx)
        })
    });
    item.key = "collision-b".into();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.observe_file_tool(2, &root.join("other"), &item, cx)
        })
    });
    item.status = ToolStatus::Completed;
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.observe_file_tool(2, &root.join("other"), &item, cx)
        })
    });
    item.key = "collision-a".into();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.observe_file_tool(1, &root.join("other"), &item, cx)
        })
    });
    std::fs::write(root.join("other/reference.txt"), "two writers\n").unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.load_file(id, cx)));
    settle(cx);
    cx.update(|_, cx| assert!(app.read(cx).files.documents[&id].matched_tool.is_none()));
    // Deleting every line must keep the change controls mounted and synchronize
    // the visible snapshot, instead of replacing it with the initial-empty view.
    std::fs::write(root.join("other/reference.txt"), "").unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.load_file(id, cx)));
    settle(cx);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let document = &app.files.documents[&id];
            assert_eq!(document.changes.as_ref().unwrap().removed, 1);
            assert!(document.preview.read(cx).visible_source().is_empty());
            document.preview.update(cx, |preview, cx| {
                preview.start_replay(cx);
                preview.pause_replay(cx);
                assert_eq!(preview.visible_source(), "two writers\n");
                preview.stop_replay(cx);
                assert!(preview.visible_source().is_empty());
            });
        })
    });
}
