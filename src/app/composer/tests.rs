use super::ComposerBinding;
use crate::app::queue::tests::{fixture, running};
use crate::app::{ConduitApp, DocumentTab, Store};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    Context, Modifiers, MouseButton, TestAppContext, VisualTestContext, Window, px, size,
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn hold_save(app: &mut ConduitApp) -> (mpsc::Sender<Result<(), String>>, mpsc::Receiver<Store>) {
    app.persistence.flush();
    let (release, wait) = mpsc::channel();
    let (entered, snapshot) = mpsc::channel();
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
    (release, snapshot)
}

fn finish_save(app: &mut ConduitApp, window: &mut Window, cx: &mut Context<ConduitApp>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.persistence.busy() {
        assert!(Instant::now() < deadline);
        app.poll_persistence(cx);
        std::thread::yield_now();
    }
    app.poll_pending_sends(window, cx);
    app.persistence.flush();
}

fn reference(
    app: &mut ConduitApp,
    root: &Path,
    window: &mut Window,
    cx: &mut Context<ConduitApp>,
) -> ComposerBinding {
    std::fs::create_dir_all(root.join("other")).unwrap();
    app.startup = None;
    app.writable = true;
    app.show_chat_beside(2, window, cx);
    let input = app.reference_chat_view(2).unwrap().read(cx).input.clone();
    app.composer_binding_for_session(2, input).unwrap()
}

#[gpui_kit::test]
fn reference_host_chat_commands_preserve_files_and_only_control_their_bound_session(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-host-commands");
    let file = root.join("attached.txt");
    std::fs::write(&file, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            let binding = reference(app, &root, window, cx);
            app.input
                .update(cx, |input, cx| input.set_value("main draft", window, cx));
            app.attach_paths(std::slice::from_ref(&file), window, cx);
            let main_files = app.current_files().unwrap().references.clone();
            let main_title = app.session(1).unwrap().title.clone();
            let message_count = app.session(2).unwrap().messages.len();
            app.with_composer(binding.clone(), cx, |app, cx| {
                app.attach_paths(std::slice::from_ref(&file), window, cx);
            });
            let reference_files = app.composer_files[binding.target.as_ref().unwrap()]
                .references
                .clone();
            for text in ["/rename  新标题 中文  ", "/pin", "/pin"] {
                binding
                    .input
                    .update(cx, |input, cx| input.set_value(text, window, cx));
                app.send_prompt_for(binding.clone(), window, cx);
                assert!(binding.input.read(cx).value().is_empty());
            }
            assert_eq!(app.session(2).unwrap().title, "新标题 中文");
            assert!(app.session(2).unwrap().pinned);
            for _ in 0..2 {
                binding
                    .input
                    .update(cx, |input, cx| input.set_value("/unpin", window, cx));
                app.send_prompt_for(binding.clone(), window, cx);
                assert!(!app.session(2).unwrap().pinned);
                assert!(binding.input.read(cx).value().is_empty());
            }
            binding
                .input
                .update(cx, |input, cx| input.set_value("/rename", window, cx));
            app.send_prompt_for(binding.clone(), window, cx);
            assert_eq!(app.rename_target, Some(2));
            app.apply_rename("窗口重命名".into(), cx);
            window.close_dialog(cx);
            assert_eq!(app.session(2).unwrap().title, "窗口重命名");

            let _main = running(app, 1);
            let _reference = running(app, 2);
            for id in [1, 2] {
                app.queued_messages.insert(
                    id,
                    crate::store::QueuedMessage {
                        text: "queued follow-up".into(),
                        references: vec![],
                        paused: None,
                    },
                );
            }
            binding
                .input
                .update(cx, |input, cx| input.set_value("/stop", window, cx));
            app.send_prompt_for(binding.clone(), window, cx);
            assert!(app.session_control.runs[&2].runtime.is_cancelled());
            assert!(!app.session_control.runs[&1].runtime.is_cancelled());
            assert!(app.queued_messages[&2].paused.is_some());
            assert!(app.queued_messages[&1].paused.is_none());
            assert!(binding.input.read(cx).value().is_empty());
            assert_eq!(app.queued_messages[&2].text, "queued follow-up");

            let invalid = "/rename first\nsecond";
            binding
                .input
                .update(cx, |input, cx| input.set_value(invalid, window, cx));
            app.send_prompt_for(binding.clone(), window, cx);
            assert_eq!(binding.input.read(cx).value().as_ref(), invalid);
            assert_eq!(app.session(2).unwrap().title, "窗口重命名");
            assert_eq!(app.session(2).unwrap().messages.len(), message_count);
            assert_eq!(
                app.composer_files[binding.target.as_ref().unwrap()].references,
                reference_files
            );
            assert_eq!(app.current_files().unwrap().references, main_files);
            assert_eq!(app.input.read(cx).value(), "main draft");
            assert_eq!(app.session(1).unwrap().title, main_title);
            assert_eq!(app.selected_session, Some(1));
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn reference_close_reopen_rejects_old_callbacks_before_first_render(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-reopen");
    cx.simulate_resize(size(px(1800.), px(1000.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input
                .update(cx, |input, cx| input.set_value("main draft", window, cx));
            let mut binding = reference(app, &root, window, cx);
            binding.input.update(cx, |input, cx| {
                input.set_value("reference draft", window, cx)
            });
            // All close/reopen cycles occur before rendering or runtime polling.
            for _ in 0..3 {
                let old = binding;
                app.close_documents(&[DocumentTab::Chat(2)], window, cx);
                assert!(app.composer_editors.is_empty());
                binding = reference(app, &root, window, cx);
                assert!(old.input != binding.input);
                assert!(app.composer_binding_live(&binding));
                assert!(app.with_composer(old.clone(), cx, |_, _| ()).is_none());
                let _ = app.render_composer_for(old, window, cx);
                assert!(app.composer_binding_live(&binding));
                assert!(app.with_composer(binding.clone(), cx, |_, _| ()).is_some());
                assert_eq!(app.composer_editors.len(), 1);
                assert_eq!(binding.input.read(cx).value(), "reference draft");
                assert_eq!(app.input.read(cx).value(), "main draft");
                assert_eq!(app.selected_session, Some(1));
            }
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("reference-composer").is_some());
    assert!(cx.debug_bounds("reference-send").is_some());
}

#[gpui_kit::test]
fn reference_move_away_and_back_registers_the_new_input_before_render(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-move");
    cx.simulate_resize(size(px(1800.), px(1000.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input
                .update(cx, |input, cx| input.set_value("main draft", window, cx));
            let old = reference(app, &root, window, cx);
            old.input.update(cx, |input, cx| {
                input.set_value("reference draft", window, cx)
            });
            // Exercise the detach/select handoff used by move_chat_to_main.
            app.detach_reference_chat(2, cx);
            app.select_document(DocumentTab::Chat(2), window, cx);
            assert!(app.composer_editors.is_empty());
            assert!(app.with_composer(old.clone(), cx, |_, _| ()).is_none());
            assert_eq!(app.input.read(cx).value(), "reference draft");
            app.input.update(cx, |input, cx| {
                input.set_value("edited while in main", window, cx)
            });
            app.select_document(DocumentTab::Chat(1), window, cx);
            let binding = reference(app, &root, window, cx);
            assert!(old.input != binding.input);
            assert!(app.with_composer(old, cx, |_, _| ()).is_none());
            assert!(app.with_composer(binding.clone(), cx, |_, _| ()).is_some());
            assert_eq!(app.composer_editors.len(), 1);
            assert_eq!(binding.input.read(cx).value(), "edited while in main");
            assert_eq!(app.input.read(cx).value(), "main draft");
            assert_eq!(app.selected_session, Some(1));
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("reference-composer").is_some());
}

#[gpui_kit::test]
fn reference_released_inside_nested_scope_restores_main_editor_state(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-nested-release");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let old = reference(app, &root, window, cx);
            let main = app.composer_binding();
            app.prompt_history = Some(11);
            let replacement = app
                .with_composer(old.clone(), cx, |app, cx| {
                    app.prompt_history = Some(4);
                    let replacement = app
                        .with_composer(main, cx, |app, cx| {
                            assert_eq!(app.prompt_history, Some(11));
                            app.prompt_history = Some(12);
                            app.close_documents(&[DocumentTab::Chat(2)], window, cx);
                            let binding = reference(app, &root, window, cx);
                            app.prune_composer_editors();
                            assert!(app.with_composer(old.clone(), cx, |_, _| ()).is_none());
                            app.with_composer(binding.clone(), cx, |app, _| {
                                app.prompt_history = Some(9);
                            })
                            .unwrap();
                            assert_eq!(app.prompt_history, Some(12));
                            binding
                        })
                        .unwrap();
                    assert_eq!(app.prompt_history, Some(4));
                    replacement
                })
                .unwrap();
            assert_eq!(app.prompt_history, Some(12));
            assert_eq!(app.composer_editors.len(), 1);
            assert!(!app.composer_editors.contains_key(&old.input.entity_id()));
            app.with_composer(replacement, cx, |app, _| {
                assert_eq!(app.prompt_history, Some(9));
            })
            .unwrap();
            assert_eq!(app.prompt_history, Some(12));
            assert!(app.composer_scope.is_none());
        })
    });
}

#[gpui_kit::test]
fn reference_queue_save_clears_only_its_bound_input_and_files(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-queue");
    let file = root.join("attached.txt");
    std::fs::write(&file, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let binding = reference(app, &root, window, cx);
            let _sender = running(app, 2);
            app.input
                .update(cx, |input, cx| input.set_value("main draft", window, cx));
            app.attach_paths(std::slice::from_ref(&file), window, cx);
            let main_files = app.current_files().unwrap().references.clone();
            app.with_composer(binding.clone(), cx, |app, cx| {
                app.attach_paths(std::slice::from_ref(&file), window, cx);
                app.composer_input().update(cx, |input, cx| {
                    input.set_value("reference queued", window, cx)
                });
            });
            let (release, snapshot) = hold_save(app);
            app.send_prompt_for(binding.clone(), window, cx);
            let saved = snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(saved.queued_messages[&2].text, "reference queued");
            assert_eq!(binding.input.read(cx).value(), "reference queued");
            assert_eq!(app.input.read(cx).value(), "main draft");
            assert_eq!(app.current_files().unwrap().references, main_files);
            release.send(Ok(())).unwrap();
            finish_save(app, window, cx);
            assert!(binding.input.read(cx).value().is_empty());
            assert!(
                !app.composer_files
                    .contains_key(binding.target.as_ref().unwrap())
            );
            assert_eq!(app.input.read(cx).value(), "main draft");
            assert_eq!(app.current_files().unwrap().references, main_files);
            assert_eq!(
                (app.selected_workspace, app.selected_session),
                (Some(0), Some(1))
            );
            // Taking back the queued message uses exactly the same editor ownership.
            app.with_composer(binding.clone(), cx, |app, cx| {
                app.take_back_queued_message(2, window, cx)
            });
            assert_eq!(binding.input.read(cx).value(), "reference queued");
            assert_eq!(app.input.read(cx).value(), "main draft");
            assert_eq!(app.current_files().unwrap().references, main_files);
            app.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn reference_failed_send_keeps_draft_and_attachments_in_its_editor(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-failed-save");
    let file = root.join("attached.txt");
    std::fs::write(&file, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let binding = reference(app, &root, window, cx);
            app.input
                .update(cx, |input, cx| input.set_value("main draft", window, cx));
            app.attach_paths(std::slice::from_ref(&file), window, cx);
            let main_files = app.current_files().unwrap().references.clone();
            app.with_composer(binding.clone(), cx, |app, cx| {
                app.attach_paths(std::slice::from_ref(&file), window, cx);
                app.composer_input().update(cx, |input, cx| {
                    input.set_value("reference unsent", window, cx)
                });
            });
            let (release, snapshot) = hold_save(app);
            app.send_prompt_for(binding.clone(), window, cx);
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(
                app.session_control.runs[&2]
                    .pending_save
                    .as_ref()
                    .unwrap()
                    .composer
                    .is_some()
            );
            assert_eq!(binding.input.read(cx).value(), "reference unsent");
            release
                .send(Err("injected reference disk failure".into()))
                .unwrap();
            finish_save(app, window, cx);
            assert_eq!(binding.input.read(cx).value(), "reference unsent");
            assert_eq!(
                app.composer_files[binding.target.as_ref().unwrap()]
                    .references
                    .len(),
                1
            );
            assert!(!app.session_control.runs.contains_key(&2));
            assert!(app.session(2).unwrap().messages.is_empty());
            assert_eq!(app.input.read(cx).value(), "main draft");
            assert_eq!(app.current_files().unwrap().references, main_files);
            assert_eq!(
                (app.selected_workspace, app.selected_session),
                (Some(0), Some(1))
            );
        })
    });
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn reference_mouse_send_reads_latest_input_and_preserves_edits_during_save(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "composer-reference-mouse");
    cx.simulate_resize(size(px(1800.), px(1000.)));
    let binding = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let binding = reference(app, &root, window, cx);
            app.input
                .update(cx, |input, cx| input.set_value("main remains", window, cx));
            binding
                .input
                .update(cx, |input, cx| input.set_value("old value", window, cx));
            binding
        })
    });
    draw(cx);
    // No explicit app notification: the shared editor subscription must repaint
    // when the actual textarea emits a change between render and button click.
    cx.update(|window, cx| {
        binding
            .input
            .update(cx, |input, cx| input.set_value("current value", window, cx))
    });
    draw(cx);
    let (release, snapshot) = cx.update(|_, cx| app.update(cx, |app, _| hold_save(app)));
    let center = cx
        .debug_bounds("reference-send")
        .expect("shared reference send button")
        .center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    let saved = snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
    let session = saved
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.sessions)
        .find(|session| session.id == 2)
        .unwrap();
    assert_eq!(session.messages[0].text, "current value");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(binding.input.read(cx).value(), "current value");
            binding.input.update(cx, |input, cx| {
                input.set_value("new edit during save", window, cx)
            });
            release.send(Ok(())).unwrap();
            finish_save(app, window, cx);
            assert_eq!(binding.input.read(cx).value(), "new edit during save");
            assert_eq!(app.input.read(cx).value(), "main remains");
            assert_eq!(app.selected_session, Some(1));
            if let Some(mut run) = app.session_control.runs.remove(&2) {
                run.cancel();
            }
        })
    });
}
