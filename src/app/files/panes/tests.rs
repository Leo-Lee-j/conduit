use super::{ConduitApp, DocumentTab, FileKey, FileViewSnapshot, ItemKind, Pane};
use crate::app::queue::tests::fixture;
use gpui_kit::{Entity, TestAppContext, VisualTestContext, px, size};
use std::path::{Path, PathBuf};

mod composer_ui;
mod layout;
mod tabs;
mod watch;

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
fn content_panes_two_chats_keep_independent_sessions_drafts_and_send_targets(
    cx: &mut TestAppContext,
) {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::{Duration, Instant};

    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-panes-two-chats");
    prepare(&app, cx, &root);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            // The fixture already has session 2 in its second workspace.
            let second = app.session_mut(2).unwrap();
            second.title = "右侧会话".into();
            second.native_id = None;
            app.input
                .update(cx, |input, cx| input.set_value("左侧草稿保留", window, cx));
            app.show_chat_beside(2, window, cx);
            let target = app.composer_target();
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(target.and_then(|target| target.session), Some(1));
            assert_eq!(app.files.panes.chat, Some(2));
            assert!(app.reference_pane_visible());
            assert_eq!(app.document_pane(DocumentTab::Chat(2)), Pane::Reference);
            assert_eq!(app.input.read(cx).value(), "左侧草稿保留");

            let view = app.reference_chat_view(2).unwrap();
            let input = view.read(cx).input.clone();
            input.update(cx, |input, cx| input.set_value("右侧独立发送", window, cx));
            let binding = app.composer_binding_for_session(2, input.clone()).unwrap();
            assert_eq!(binding.pane, Pane::Reference);
            assert_eq!(binding.target.as_ref().unwrap().session, Some(2));
            assert_eq!(
                binding.target.as_ref().unwrap().workspace,
                root.join("other")
            );
            let main_input = app.input.clone();
            let main_target = app.composer_target();
            app.writable = true;
            app.persistence.flush();
            let (release, wait) = mpsc::channel();
            let (entered, snapshots) = mpsc::channel();
            let first = AtomicBool::new(true);
            app.persistence = crate::persistence::Persistence::with_writer(move |store| {
                if first.swap(false, Ordering::AcqRel) {
                    entered.send(store).map_err(|_| "test receiver dropped")?;
                    wait.recv_timeout(Duration::from_secs(5))
                        .map_err(|_| "test save timed out")?
                } else {
                    Ok(())
                }
            });
            app.send_prompt_for(binding, window, cx);
            let saved = snapshots.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));
            assert!(app.input == main_input);
            assert_eq!(app.composer_target(), main_target);
            assert_eq!(app.session_control.runs.len(), 1);
            assert!(app.session_control.runs.contains_key(&2));
            assert!(!app.session_control.runs.contains_key(&1));
            assert_eq!(app.session_control.runs[&2].worktree, root.join("other"));
            assert!(app.session_control.runs[&2].pending_save.is_some());
            assert!(app.session_control.runs[&2].api_request_started.is_none());
            assert!(saved.workspaces[0].sessions[0].messages.is_empty());
            assert!(
                saved.workspaces[1].sessions[0].messages.iter().any(|item| {
                    item.kind == ItemKind::User && item.text == "右侧独立发送"
                })
            );
            assert!(
                app.session(2).unwrap().messages.iter().any(|item| {
                    item.kind == ItemKind::User && item.text == "右侧独立发送"
                })
            );
            assert_eq!(input.read(cx).value(), "右侧独立发送");
            assert_eq!(app.input.read(cx).value(), "左侧草稿保留");

            // Only a successful save receipt may release the send and clear
            // the bound input. Main remains selected throughout this flow.
            release.send(Ok(())).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while app.persistence.busy() {
                assert!(Instant::now() < deadline, "send save did not finish");
                app.poll_persistence(cx);
                std::thread::yield_now();
            }
            app.poll_pending_sends(window, cx);
            app.persistence.flush();
            assert!(app.session_control.runs[&2].pending_save.is_none());
            assert!(app.session_control.runs[&2].api_request_started.is_some());
            assert!(input.read(cx).value().is_empty());
            assert_eq!(app.input.read(cx).value(), "左侧草稿保留");
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));
            assert_eq!(app.composer_target(), main_target);

            input.update(cx, |input, cx| {
                input.set_value("右侧切换后草稿", window, cx)
            });
            let mut third = app.session(2).unwrap().clone();
            third.id = 3;
            third.title = "右侧第三会话".into();
            third.native_id = None;
            app.store.workspaces[0].sessions.push(third);
            app.show_chat_beside(3, window, cx);
            assert_eq!(app.document_pane(DocumentTab::Chat(2)), Pane::Reference);
            assert_eq!(app.document_pane(DocumentTab::Chat(3)), Pane::Reference);
            assert_eq!(
                app.pane_document(Pane::Reference),
                Some(DocumentTab::Chat(3))
            );
            app.select_reference_chat(2, window, cx);
            assert_eq!(input.read(cx).value(), "右侧切换后草稿");
            app.move_chat_to_main(2, window, cx);
            assert_eq!(app.selected_session, Some(2));
            assert_eq!(
                app.drafts.get(&2).map(String::as_str),
                Some("右侧切换后草稿")
            );
            assert_eq!(app.files.panes.chat, Some(3));
            app.close_documents(&[DocumentTab::Chat(3)], window, cx);
            assert_eq!(app.selected_session, Some(2));
            assert!(app.files.panes.chat.is_none());
            assert!(!app.files.panes.split);
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("content-main").is_some());
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

#[gpui_kit::test]
fn content_proposal_stream_routing_is_transient_and_cancels_on_conflict(cx: &mut TestAppContext) {
    use crate::app::queue::tests::running;
    use crate::protocol::{
        Event,
        file_proposals::{Input, Update},
    };
    use std::sync::Arc;
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-proposal-routing");
    prepare(&app, cx, &root);
    let id = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_workspace_file(reference(&root), true, window, cx);
            app.files.panes.active.unwrap()
        })
    });
    settle(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let sender = running(app, 1);
            let before = app.files.documents[&id].text.clone();
            let count = app.session(1).unwrap().messages.len();
            let content = Update::Content(Arc::new(Input {
                path: "other/reference.txt".into(),
                old: Some("needle".into()),
                new: "proposed".into(),
                complete: false,
            }));
            for update in [Update::Started, content.clone()] {
                sender
                    .send(Event::FileProposal {
                        key: "edit".into(),
                        update,
                    })
                    .unwrap();
            }
            app.poll_streams(window, cx);
            let preview = app.files.documents[&id].preview.clone();
            preview.update(cx, |preview, cx| {
                assert_eq!(preview.visible_source(), before);
                preview.start_proposal(cx);
                assert!(preview.visible_source().contains("proposed"));
            });
            assert_eq!(app.files.documents[&id].text, before);
            assert_eq!(app.session(1).unwrap().messages.len(), count);
            assert_eq!(
                std::fs::read_to_string(root.join("other/reference.txt")).unwrap(),
                before.as_ref()
            );
            // A second live writer removes both offers, even after one writer ends.
            for update in [Update::Started, content.clone()] {
                sender
                    .send(Event::FileProposal {
                        key: "collision".into(),
                        update,
                    })
                    .unwrap();
            }
            app.poll_streams(window, cx);
            assert_eq!(preview.read(cx).visible_source(), before);
            sender
                .send(Event::FileProposal {
                    key: "collision".into(),
                    update: Update::Ended,
                })
                .unwrap();
            sender
                .send(Event::FileProposal {
                    key: "edit".into(),
                    update: content.clone(),
                })
                .unwrap();
            app.poll_streams(window, cx);
            preview.update(cx, |preview, cx| preview.start_proposal(cx));
            assert_eq!(preview.read(cx).visible_source(), before);
            app.end_file_proposals(1, None, cx);
            // Another authoritative tool can conflict before this invocation's
            // path/new_string has arrived. A late delta must not resurrect it.
            sender.send(Event::FileProposal { key: "pending".into(), update: Update::Started }).unwrap();
            sender.send(Event::Tool { key: "full".into(), title: "Write".into(), detail: String::new(), status: crate::message_status::ToolStatus::Running }).unwrap();
            sender.send(Event::ToolInput { key: "full".into(), input: serde_json::json!({"file_path":"other/reference.txt", "content":"competing"}).to_string() }).unwrap();
            app.poll_streams(window, cx);
            sender.send(Event::FileProposal { key: "pending".into(), update: content.clone() }).unwrap();
            app.poll_streams(window, cx);
            assert!(!preview.read(cx).has_proposal());
            app.end_file_proposals(1, None, cx);
            // A queue reset is request-scoped, not a tool completion. It must
            // restore the disk view and drop every candidate without history.
            for update in [Update::Started, content.clone()] {
                sender.send(Event::FileProposal { key: "backpressure".into(), update }).unwrap();
            }
            app.poll_streams(window, cx);
            preview.update(cx, |preview, cx| preview.start_proposal(cx));
            assert!(preview.read(cx).visible_source().contains("proposed"));
            let before_reset = app.session(1).unwrap().messages.len();
            sender.send(Event::FileProposal { key: String::new(), update: Update::Reset }).unwrap();
            sender.send(Event::FileProposal { key: "backpressure".into(), update: content.clone() }).unwrap();
            app.poll_streams(window, cx);
            assert!(!preview.read(cx).has_proposal());
            assert_eq!(preview.read(cx).visible_source(), before);
            assert_eq!(app.session(1).unwrap().messages.len(), before_reset);
            for update in [Update::Started, content.clone()] {
                sender
                    .send(Event::FileProposal {
                        key: "cancel".into(),
                        update,
                    })
                    .unwrap();
            }
            app.poll_streams(window, cx);
            preview.update(cx, |preview, cx| preview.start_proposal(cx));
            assert!(preview.read(cx).visible_source().contains("proposed"));
            app.session_control.runs.get_mut(&1).unwrap().cancel();
            app.poll_streams(window, cx);
            assert_eq!(preview.read(cx).visible_source(), before);
            sender
                .send(Event::FileProposal {
                    key: "cancel".into(),
                    update: content,
                })
                .unwrap();
            app.poll_streams(window, cx);
            preview.update(cx, |preview, cx| preview.start_proposal(cx));
            assert_eq!(preview.read(cx).visible_source(), before);
        })
    });
}

#[gpui_kit::test]
fn content_proposal_empty_file_mounts_and_external_version_ends_preview(cx: &mut TestAppContext) {
    use crate::app::queue::tests::running;
    use crate::protocol::{
        Event,
        file_proposals::{Input, Update},
    };
    use std::sync::Arc;
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "content-proposal-empty");
    prepare(&app, cx, &root);
    std::fs::write(root.join("other/reference.txt"), "").unwrap();
    let (id, sender) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_workspace_file(reference(&root), true, window, cx);
            (app.files.panes.active.unwrap(), running(app, 1))
        })
    });
    settle(cx);
    let content = Update::Content(Arc::new(Input {
        path: "other/reference.txt".into(),
        old: None,
        new: "proposed\n".into(),
        complete: false,
    }));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            for update in [Update::Started, content.clone()] {
                sender
                    .send(Event::FileProposal {
                        key: "write".into(),
                        update,
                    })
                    .unwrap();
            }
            app.poll_streams(window, cx);
        })
    });
    settle(cx);
    assert!(cx.debug_bounds("file-proposal-bar").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let document = &app.files.documents[&id];
            assert!(document.text.is_empty());
            document.preview.update(cx, |preview, cx| {
                preview.start_proposal(cx);
                assert_eq!(preview.visible_source(), "proposed\n");
            });
        })
    });
    std::fs::write(root.join("other/reference.txt"), "external\n").unwrap();
    cx.update(|_, cx| app.update(cx, |app, cx| app.load_file(id, cx)));
    settle(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            sender
                .send(Event::FileProposal {
                    key: "write".into(),
                    update: content,
                })
                .unwrap();
            app.poll_streams(window, cx);
            let preview = &app.files.documents[&id].preview;
            assert!(!preview.read(cx).has_proposal());
            assert_eq!(preview.read(cx).visible_source(), "external\n");
        })
    });
}
