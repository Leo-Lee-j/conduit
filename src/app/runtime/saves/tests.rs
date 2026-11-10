use crate::app::queue::tests::{fixture, running};
use crate::app::{ComposerSelection, ConduitApp, Event, ModelChannel, Provider, Store, queue};
use crate::app::{NoticeLevel, NoticeSource, NoticeTarget};
use crate::persistence::Persistence;
use gpui_kit::{Context, TestAppContext, Window, component::WindowExt as _};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

/// Hold exactly the send snapshot; subsequent rollback saves can complete.
fn hold_save(app: &mut ConduitApp) -> (mpsc::Sender<Result<(), String>>, mpsc::Receiver<Store>) {
    app.persistence.flush();
    let (release, wait) = mpsc::channel();
    let (entered, snapshot) = mpsc::channel();
    let first = AtomicBool::new(true);
    app.persistence = Persistence::with_writer(move |store| {
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
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while app.persistence.busy() {
        assert!(std::time::Instant::now() < deadline);
        app.poll_persistence(cx);
        std::thread::yield_now();
    }
    app.poll_pending_sends(window, cx);
    app.persistence.flush();
}

#[gpui_kit::test]
fn native_forks_preserve_drafts_and_require_a_saved_unchanged_request(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "native-fork-barrier");
    let marker = root.join("fork-launched.txt");
    let launcher = root.join("fork.cmd");
    std::fs::write(
        &launcher,
        "@echo off\r\n> \"%~dp0fork-launched.txt\" echo launched\r\nexit /b 0\r\n",
    )
    .unwrap();
    let attachment = root.join("reference.txt");
    std::fs::write(&attachment, "source attachment").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.store.cli.codex_path = Some(launcher.clone());
            let source = this.session_mut(1).unwrap();
            source.provider = Provider::Codex;
            source.messages = serde_json::from_value(serde_json::json!([
                {"key":"first-user","kind":"User","text":"first"},
                {"key":"first-reply","kind":"Assistant","text":"answer"}
            ]))
            .unwrap();
            crate::branching::record(source, "queue-first".into(), "first-turn".into());
            source.messages.push(
                serde_json::from_value(serde_json::json!({
                    "key":"later-user","kind":"User","text":"later"
                }))
                .unwrap(),
            );
            let original = serde_json::to_value(this.session(1).unwrap()).unwrap();
            for scenario in 0..4 {
                this.switch(0, Some(1), window, cx);
                this.input
                    .update(cx, |input, cx| input.set_value("source draft", window, cx));
                if scenario == 0 {
                    this.attach_paths(std::slice::from_ref(&attachment), window, cx);
                }
                let (release, snapshot) = hold_save(this);
                let id = this.create_native_fork(1, 2, window, cx).unwrap();
                let saved = snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                let fork = saved.workspaces[0]
                    .sessions
                    .iter()
                    .find(|s| s.id == id)
                    .unwrap();
                assert!(fork.native_id.is_none());
                assert_eq!(
                    fork.branching
                        .origin
                        .as_ref()
                        .unwrap()
                        .point
                        .request
                        .last_turn_id,
                    "first-turn"
                );
                assert_eq!(fork.messages[1].text, "answer");
                assert_eq!(this.drafts[&1], "source draft");
                assert!(this.session_fork_pending(id));
                this.input
                    .update(cx, |input, cx| input.set_value("branch draft", window, cx));
                this.send_prompt(window, cx);
                assert!(this.queued_messages.is_empty());
                assert_eq!(this.input.read(cx).value().as_ref(), "branch draft");
                assert!(!marker.exists());
                match scenario {
                    0 => release.send(Err("save failed".into())).unwrap(),
                    1 => {
                        this.session_mut(id).unwrap().permission =
                            crate::domain::PermissionLevel::FullAccess;
                        release.send(Ok(())).unwrap();
                    }
                    2 => {
                        std::fs::write(&launcher, "@echo off\r\nexit /b 1\r\n").unwrap();
                        release.send(Ok(())).unwrap();
                    }
                    _ => {
                        this.session_control.runs.get_mut(&id).unwrap().cancel();
                        release.send(Ok(())).unwrap();
                    }
                }
                finish_save(this, window, cx);
                assert!(!this.session_control.runs.contains_key(&id));
                assert!(crate::branching::request(this.session(id).unwrap()).is_ok());
                assert_eq!(this.session(id).unwrap().messages.len(), 2);
                assert_eq!(
                    serde_json::to_value(this.session(1).unwrap()).unwrap(),
                    original
                );
                assert!(!marker.exists());
                this.send_prompt(window, cx);
                assert!(!this.session_control.runs.contains_key(&id));
                assert_eq!(this.input.read(cx).value().as_ref(), "branch draft");
                let (release, snapshot) = hold_save(this);
                this.retry_native_fork(id, cx).unwrap();
                snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                this.session_control.runs.get_mut(&id).unwrap().cancel();
                release.send(Ok(())).unwrap();
                finish_save(this, window, cx);
                assert!(!marker.exists());
                this.switch(0, Some(1), window, cx);
                assert_eq!(this.input.read(cx).value().as_ref(), "source draft");
                assert_eq!(this.current_files().unwrap().references.len(), 1);
            }
        })
    });
}

#[gpui_kit::test]
fn commands_revalidate_after_save_and_preserve_drafts_on_failure_or_cancellation(
    cx: &mut TestAppContext,
) {
    use crate::commands::{Arguments, CommandId, Descriptor, Execution};
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-save-barrier");
    let marker = root.join("command-launched.txt");
    std::fs::write(
        root.join("offline.cmd"),
        "@echo off\r\n> \"%~dp0command-launched.txt\" echo launched\r\nexit /b 0\r\n",
    )
    .unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.store
                .cli
                .set(Provider::OpenCode, Some(root.join("offline.cmd")));
            this.session_mut(1).unwrap().provider = Provider::OpenCode;
            this.new_session_provider = Provider::OpenCode;
            for scenario in 0..3 {
                this.remember_command_executable(Provider::OpenCode, &root.join("offline.cmd"))
                    .unwrap();
                let context = this.session_command_context(1).unwrap();
                let ticket = this.chat_commands.catalog.begin(context);
                this.chat_commands.catalog.publish(
                    &ticket,
                    Ok(vec![Descriptor {
                        revision: 1,
                        unavailable: None,
                        id: CommandId {
                            source: "opencode".into(),
                            local: "review".into(),
                        },
                        name: "review".into(),
                        aliases: vec![],
                        summary: "Review".into(),
                        arguments: Arguments::TailText {
                            required: false,
                            hint: String::new(),
                        },
                        execution: Execution::Engine,
                    }]),
                );
                let (release, snapshot) = hold_save(this);
                this.input
                    .update(cx, |input, cx| input.set_value("/review 中文", window, cx));
                this.send_prompt(window, cx);
                assert!(
                    this.session_control.runs.contains_key(&1),
                    "scenario {scenario}: {:?}; notices: {:?}",
                    this.message,
                    this.notifications.center.records()
                );
                let saved = snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                assert!(
                    saved.workspaces[0].sessions[0]
                        .messages
                        .last()
                        .unwrap()
                        .key
                        .ends_with("-command")
                );
                assert_eq!(this.input.read(cx).value().as_ref(), "/review 中文");
                assert!(!marker.exists());
                match scenario {
                    0 => release.send(Err("offline save failure".into())).unwrap(),
                    1 => {
                        this.session_mut(1).unwrap().permission =
                            crate::domain::PermissionLevel::FullAccess;
                        release.send(Ok(())).unwrap();
                    }
                    _ => {
                        this.session_control.runs.get_mut(&1).unwrap().cancel();
                        release.send(Ok(())).unwrap();
                    }
                }
                finish_save(this, window, cx);
                assert!(this.session_control.runs.is_empty());
                assert!(this.session(1).unwrap().messages.is_empty());
                assert_eq!(this.input.read(cx).value().as_ref(), "/review 中文");
                assert!(!marker.exists());
            }
        })
    });
}

#[gpui_kit::test]
fn commands_discovery_binds_new_session_after_save_without_consuming_the_draft(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-connect-save");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.store
                .cli
                .set(Provider::OpenCode, Some(root.join("offline.cmd")));
            this.new_session_provider = Provider::OpenCode;
            this.selected_session = None;
            let file = root.join("reference.txt");
            std::fs::write(&file, "reference").unwrap();
            this.attach_paths(&[file], window, cx);
            this.input
                .update(cx, |input, cx| input.set_value("/review draft", window, cx));
            let (release, snapshot) = hold_save(this);
            assert!(this.send_input(
                crate::commands::TurnInput {
                    text: "Connect".into(),
                    intent: crate::commands::RequestIntent::Discover
                },
                true,
                window,
                cx
            ));
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(this.selected_session, None);
            release.send(Ok(())).unwrap();
            finish_save(this, window, cx);
            let id = this.selected_session.unwrap();
            assert_eq!(this.session(id).unwrap().provider, Provider::OpenCode);
            assert_eq!(this.input.read(cx).value().as_ref(), "/review draft");
            assert_eq!(this.current_files().unwrap().references.len(), 1);
            assert!(this.session_control.namings.is_empty());
            this.session_control.runs.get_mut(&id).unwrap().cancel();
        })
    });
}

#[gpui_kit::test]
fn slow_save_retains_draft_and_attachments_and_gates_cli(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "send-barrier");
    // This launcher only writes a marker in its temporary fixture directory.
    std::fs::write(
        root.join("offline.cmd"),
        "@echo off\r\n> \"%~dp0launched.txt\" echo launched\r\nexit /b 0\r\n",
    )
    .unwrap();
    let marker = root.join("launched.txt");
    let file = root.join("reference.txt");
    std::fs::write(&file, "attachment").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let (release, snapshot) = hold_save(this);
            this.attach_paths(&[file], window, cx);
            this.input.update(cx, |input, cx| {
                input.set_value("save before sending", window, cx)
            });
            this.send_prompt(window, cx);
            let snapshot = snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(snapshot.workspaces[0].sessions[0].messages.len(), 1);
            assert_eq!(this.input.read(cx).value().as_ref(), "save before sending");
            assert_eq!(this.current_files().unwrap().references.len(), 1);
            assert!(this.composer_save_pending());
            assert!(this.session_control.namings.is_empty());
            assert!(
                this.message.is_none(),
                "save progress must not use the error banner"
            );
            assert!(this.notifications.center.records().is_empty());
            this.send_prompt(window, cx);
            assert_eq!(this.session(1).unwrap().messages.len(), 1);
            assert!(this.queued_messages.is_empty());
            assert!(
                this.message.is_none(),
                "a repeated send during saving stays silent"
            );
            let run = &this.session_control.runs[&1];
            assert_eq!(run.activity, "正在保存消息…");
            assert!(matches!(
                run.runtime.events.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            assert!(
                !marker.exists(),
                "CLI must not start before disk acknowledgement"
            );
            release.send(Ok(())).unwrap();
            finish_save(this, window, cx);
            assert!(this.input.read(cx).value().is_empty());
            assert!(this.current_files().is_none());
            assert!(this.session_control.runs[&1].pending_save.is_none());
            assert!(matches!(
                this.session_control.runs[&1]
                    .runtime
                    .events
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap(),
                Event::Finished { .. }
            ));
            assert!(
                marker.exists(),
                "saved send must release the offline launcher"
            );
            this.session_control.runs.remove(&1);
        })
    });
}

#[gpui_kit::test]
fn unsent_session_engine_switch_preserves_identity_draft_and_attachments(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "unsent-engine-switch");
    let file = root.join("reference.txt");
    std::fs::write(&file, "attachment").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.store.cli.codex_path = Some(root.join("offline.cmd"));
            this.store.session_defaults.insert(
                Provider::Codex,
                crate::store::SessionDefaults {
                    channel: ModelChannel::Cli,
                    model: Some("gpt-5".into()),
                },
            );
            let session = this.session_mut(1).unwrap();
            session.native_id = None;
            session.model = Some("claude-model".into());
            session.reasoning_effort = crate::providers::ReasoningEffort::High;
            session.opencode.variant = Some("old-variant".into());
            session.permission = crate::domain::PermissionLevel::FullAccess;
            let tabs = this.open_tabs.clone();
            let next_id = this.next_id;
            this.attach_paths(&[file], window, cx);
            this.input.update(cx, |input, cx| {
                input.set_value("send from the same conversation", window, cx)
            });

            this.select_engine(Provider::Codex, window, cx);
            assert_eq!(this.selected_session, Some(1));
            assert_eq!(this.send_target(true), Some(1));
            assert_eq!(this.model_picker_session_id(1), Some(1));
            assert_eq!(this.open_tabs, tabs);
            assert_eq!(this.next_id, next_id);
            let session = this.session(1).unwrap();
            assert_eq!(session.provider, Provider::Codex);
            assert_eq!(session.model.as_deref(), Some("gpt-5"));
            assert_eq!(
                session.reasoning_effort,
                crate::providers::ReasoningEffort::Medium
            );
            assert_eq!(
                session.permission,
                crate::domain::PermissionLevel::FullAccess
            );
            assert!(session.opencode.variant.is_none());
            this.update_composer_selection(1, cx, |selection| {
                selection.set_model(Some("chosen-model".into()));
            });
            assert_eq!(
                this.session(1).unwrap().model.as_deref(),
                Some("chosen-model")
            );
            assert!(this.pending_model_selection.is_none());
            assert_eq!(this.current_files().unwrap().references.len(), 1);
            assert_eq!(
                this.input.read(cx).value().as_ref(),
                "send from the same conversation"
            );

            let (release, snapshot) = hold_save(this);
            this.send_prompt(window, cx);
            let snapshot = snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(snapshot.workspaces[0].sessions.len(), 1);
            let sent = &snapshot.workspaces[0].sessions[0];
            assert_eq!(sent.id, 1);
            assert_eq!(sent.provider, Provider::Codex);
            assert_eq!(sent.model.as_deref(), Some("chosen-model"));
            assert_eq!(sent.messages.len(), 1);
            assert!(
                sent.messages[0]
                    .text
                    .contains("send from the same conversation")
            );
            assert!(sent.messages[0].text.contains("reference.txt"));
            assert_eq!(this.selected_session, Some(1));
            assert_eq!(this.next_id, next_id);
            assert_eq!(this.session_control.runs.len(), 1);
            assert!(
                this.session_control.runs[&1]
                    .pending_save
                    .as_ref()
                    .unwrap()
                    .before
                    .is_some()
            );
            release.send(Ok(())).unwrap();
            finish_save(this, window, cx);
            assert!(this.input.read(cx).value().is_empty());
            assert!(this.current_files().is_none());
            assert!(matches!(
                this.session_control.runs[&1]
                    .runtime
                    .events
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap(),
                Event::Finished { .. }
            ));
            this.session_control.runs.remove(&1);
        })
    });
}

#[gpui_kit::test]
fn failed_save_restores_existing_and_new_sessions_without_consuming_drafts(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "failed-send");
    let file = root.join("reference.txt");
    std::fs::write(&file, "attachment").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            for new_session in [false, true] {
                this.selected_session = (!new_session).then_some(1);
                let mut selection = ComposerSelection::new(Provider::Claude);
                selection.model = Some("fixture-model".into());
                this.pending_model_selection = Some(selection);
                this.store.ui.auto_rename = true;
                this.session_control.unread_completed.insert(1);
                let original = serde_json::to_value(&this.store.workspaces[0].sessions).unwrap();
                let tabs = this.open_tabs.clone();
                let (release, snapshot) = hold_save(this);
                this.attach_paths(std::slice::from_ref(&file), window, cx);
                this.input.update(cx, |input, cx| {
                    input.set_value("keep on failure", window, cx)
                });
                this.send_prompt(window, cx);
                snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                assert_eq!(this.session_control.runs.len(), 1);
                release.send(Err("injected disk full".into())).unwrap();
                finish_save(this, window, cx);
                assert!(this.session_control.runs.is_empty());
                assert!(this.session_control.namings.is_empty());
                assert_eq!(
                    serde_json::to_value(&this.store.workspaces[0].sessions).unwrap(),
                    original
                );
                assert_eq!(this.open_tabs, tabs);
                assert_eq!(this.selected_session, (!new_session).then_some(1));
                assert_eq!(
                    this.pending_model_selection
                        .as_ref()
                        .unwrap()
                        .model
                        .as_deref(),
                    Some("fixture-model")
                );
                assert_eq!(this.input.read(cx).value().as_ref(), "keep on failure");
                assert_eq!(this.current_files().unwrap().references.len(), 1);
                assert!(this.session_control.unread_completed.contains(&1));
                assert!(
                    this.message.is_none(),
                    "send failures use unified notifications"
                );
                let notices = this
                    .notifications
                    .center
                    .records()
                    .iter()
                    .filter(|notice| notice.title == "消息未发送")
                    .collect::<Vec<_>>();
                assert_eq!(notices.len(), if new_session { 2 } else { 1 });
                assert!(notices.iter().all(|notice| {
                    notice.message.contains("injected disk full")
                        && notice.message.contains("草稿已保留")
                }));
                this.poll_notifications(window, cx);
                assert!(!window.notifications(cx).is_empty());
            }
        })
    });
}

#[gpui_kit::test]
fn invalid_provider_does_not_create_session_or_submit_snapshot(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "invalid-route");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.selected_session = None;
            let mut selection = ComposerSelection::new(Provider::Claude);
            selection.channel = ModelChannel::Provider("missing-provider-fixture".into());
            selection.model = Some("fixture-model".into());
            this.pending_model_selection = Some(selection);
            let id = this.next_id;
            let tabs = this.open_tabs.clone();
            let sessions = serde_json::to_value(&this.store.workspaces[0].sessions).unwrap();
            let (_release, snapshot) = hold_save(this);
            this.input
                .update(cx, |input, cx| input.set_value("invalid route", window, cx));
            this.send_prompt(window, cx);
            assert!(snapshot.try_recv().is_err());
            assert!(!this.persistence.busy());
            assert!(this.session_control.runs.is_empty());
            assert_eq!(this.next_id, id);
            assert_eq!(this.open_tabs, tabs);
            assert_eq!(this.selected_session, None);
            assert_eq!(
                this.pending_model_selection
                    .as_ref()
                    .unwrap()
                    .model
                    .as_deref(),
                Some("fixture-model")
            );
            assert_eq!(
                serde_json::to_value(&this.store.workspaces[0].sessions).unwrap(),
                sessions
            );
            assert_eq!(this.input.read(cx).value().as_ref(), "invalid route");
            assert!(this.message.is_none());
            let notice = this
                .notifications
                .center
                .records()
                .iter()
                .find(|notice| notice.title == "会话无法启动")
                .unwrap();
            assert!(notice.message.contains("Provider"));
            this.poll_notifications(window, cx);
            assert_eq!(window.notifications(cx).len(), 1);
        })
    });
}

#[gpui_kit::test]
fn engine_switch_save_keeps_its_source_session_until_ack(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "engine-switch-save");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.new_session_provider = Provider::Codex;
            this.store.cli.codex_path = Some(root.join("offline.cmd"));
            let (release, snapshot) = hold_save(this);
            this.input.update(cx, |input, cx| {
                input.set_value("new engine draft", window, cx)
            });
            this.send_prompt(window, cx);
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(!this.session_control.runs.contains_key(&1));
            assert!(this.session_draft_save_pending(1));
            this.delete_session(1, cx);
            assert!(this.session(1).is_some());
            release
                .send(Err("injected engine-switch save failure".into()))
                .unwrap();
            finish_save(this, window, cx);
            assert_eq!(this.selected_session, Some(1));
            assert_eq!(this.new_session_provider, Provider::Codex);
            assert_eq!(this.input.read(cx).value().as_ref(), "new engine draft");
            assert_eq!(this.store.workspaces[0].sessions.len(), 1);
        })
    });
}

#[gpui_kit::test]
fn new_session_selects_after_ack_and_preserves_a_changed_model_choice(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "new-session-ack");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            for change_model in [false, true] {
                this.selected_session = None;
                let mut choice = ComposerSelection::new(Provider::Claude);
                choice.model = Some("original-model".into());
                this.pending_model_selection = Some(choice);
                let (release, snapshot) = hold_save(this);
                this.input.update(cx, |input, cx| {
                    input.set_value("new conversation", window, cx)
                });
                this.send_prompt(window, cx);
                snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                let id = *this.session_control.runs.keys().next().unwrap();
                assert_eq!(this.selected_session, None);
                assert!(this.pending_model_selection.is_some());
                if change_model {
                    this.pending_model_selection.as_mut().unwrap().model =
                        Some("next-model".into());
                }
                release.send(Ok(())).unwrap();
                finish_save(this, window, cx);
                assert!(this.input.read(cx).value().is_empty());
                assert_eq!(
                    this.session(id).unwrap().model.as_deref(),
                    Some("original-model")
                );
                if change_model {
                    assert_eq!(this.selected_session, None);
                    assert_eq!(
                        this.pending_model_selection
                            .as_ref()
                            .unwrap()
                            .model
                            .as_deref(),
                        Some("next-model")
                    );
                } else {
                    assert_eq!(this.selected_session, Some(id));
                    assert!(this.pending_model_selection.is_none());
                }
                this.session_control.runs.remove(&id).unwrap().cancel();
            }
        })
    });
}

#[gpui_kit::test]
fn delayed_success_preserves_edited_draft_navigation_and_model_choice(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "save-navigation");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            for navigate in [false, true] {
                this.switch(0, None, window, cx);
                this.input
                    .update(cx, |input, cx| input.set_value("original", window, cx));
                let (release, snapshot) = hold_save(this);
                this.send_prompt(window, cx);
                snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                let id = *this.session_control.runs.keys().next().unwrap();
                this.switch(0, Some(id), window, cx);
                assert_eq!(
                    this.selected_session, None,
                    "provisional session must not take another draft"
                );
                assert_eq!(this.input.read(cx).value().as_ref(), "original");
                if navigate {
                    this.switch(1, Some(2), window, cx);
                    this.input
                        .update(cx, |input, cx| input.set_value("other draft", window, cx));
                    this.message = Some("other banner".into());
                } else {
                    this.input.update(cx, |input, cx| {
                        input.set_value("edited during save", window, cx)
                    });
                    let mut selection = ComposerSelection::new(Provider::Claude);
                    selection.model = Some("next-model".into());
                    this.pending_model_selection = Some(selection);
                }
                release.send(Ok(())).unwrap();
                finish_save(this, window, cx);
                if navigate {
                    assert_eq!(this.selected_session, Some(2));
                    assert_eq!(this.selected_workspace, Some(1));
                    assert_eq!(this.input.read(cx).value().as_ref(), "other draft");
                    assert_eq!(this.message.as_deref(), Some("other banner"));
                    assert!(
                        !this
                            .workspace_drafts
                            .contains_key(&this.store.workspaces[0].path)
                    );
                } else {
                    assert_eq!(this.selected_session, None);
                    assert_eq!(this.input.read(cx).value().as_ref(), "edited during save");
                    assert_eq!(
                        this.pending_model_selection
                            .as_ref()
                            .unwrap()
                            .model
                            .as_deref(),
                        Some("next-model")
                    );
                }
                let mut run = this.session_control.runs.remove(&id).unwrap();
                run.cancel();
            }
        })
    });
}

#[gpui_kit::test]
fn attachment_only_send_preserves_files_added_while_saving(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "attachment-only-save");
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    std::fs::write(&first, "first").unwrap();
    std::fs::write(&second, "second").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.selected_session = None;
            this.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            this.attach_paths(&[first], window, cx);
            let (release, snapshot) = hold_save(this);
            this.send_prompt(window, cx);
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            let id = *this.session_control.runs.keys().next().unwrap();
            this.attach_paths(&[second], window, cx);
            release.send(Ok(())).unwrap();
            finish_save(this, window, cx);
            assert_eq!(this.selected_session, None);
            assert_eq!(this.current_files().unwrap().references.len(), 2);
            assert!(this.input.read(cx).value().is_empty());
            this.session_control.runs.remove(&id).unwrap().cancel();
        })
    });
}

#[gpui_kit::test]
fn cancel_or_close_while_saving_rolls_back_and_ignores_late_success(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "cancel-save");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            for closing in [false, true] {
                let (release, snapshot) = hold_save(this);
                this.input
                    .update(cx, |input, cx| input.set_value("unsent", window, cx));
                this.send_prompt(window, cx);
                snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                if closing {
                    this.closing = true;
                } else {
                    this.session_control.runs.get_mut(&1).unwrap().cancel();
                }
                this.poll_pending_sends(window, cx);
                assert!(!this.closing);
                assert!(this.session_control.runs.is_empty());
                assert!(this.session(1).unwrap().messages.is_empty());
                assert_eq!(this.input.read(cx).value().as_ref(), "unsent");
                release.send(Ok(())).unwrap();
                finish_save(this, window, cx);
                assert!(this.session_control.runs.is_empty());
                assert!(this.session(1).unwrap().messages.is_empty());
                assert!(this.session_control.namings.is_empty());
            }
        })
    });
}

#[gpui_kit::test]
fn retry_and_regenerate_restore_original_output_when_save_fails(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "retry-save");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.session_mut(1).unwrap().messages = serde_json::from_value(serde_json::json!([
                {"key":"turn-0-user","kind":"User","text":"original prompt"},
                {"key":"turn-0-reply","kind":"Assistant","text":"partial reply"},
                {"key":"turn-0-error","kind":"Error","text":"original error"}
            ]))
            .unwrap();
            let before = serde_json::to_value(&this.session(1).unwrap().messages).unwrap();
            for retry in [true, false] {
                let (release, snapshot) = hold_save(this);
                if retry {
                    this.retry_error(2, window, cx);
                } else {
                    this.regenerate(1, window, cx);
                }
                snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
                assert!(this.session_control.runs.contains_key(&1));
                release.send(Err("injected disk failure".into())).unwrap();
                finish_save(this, window, cx);
                assert_eq!(
                    serde_json::to_value(&this.session(1).unwrap().messages).unwrap(),
                    before
                );
                assert!(this.session_control.runs.is_empty());
            }
        })
    });
}

#[gpui_kit::test]
fn queued_dispatch_failure_restores_paused_message_and_background_banner(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "queue-save-failure");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.queued_messages.insert(
                1,
                queue::QueuedMessage {
                    text: "queued content".into(),
                    references: vec![],
                    paused: None,
                },
            );
            this.selected_workspace = Some(1);
            this.selected_session = Some(2);
            this.message = Some("other banner".into());
            let (release, snapshot) = hold_save(this);
            this.dispatch_queued_message(1, cx);
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(this.queued_messages.is_empty());
            release
                .send(Err("injected queue dispatch failure".into()))
                .unwrap();
            finish_save(this, window, cx);
            assert_eq!(this.queued_messages[&1].text, "queued content");
            assert!(this.queued_messages[&1].paused.is_some());
            assert!(this.session(1).unwrap().messages.is_empty());
            assert_eq!(this.message.as_deref(), Some("other banner"));
            assert_eq!(this.selected_session, Some(2));
        })
    });
}

#[gpui_kit::test]
fn pending_queue_save_blocks_deletion_and_failure_preserves_composer(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "queue-enqueue-failure");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let sender = running(this, 1);
            let (release, snapshot) = hold_save(this);
            this.input
                .update(cx, |input, cx| input.set_value("unsaved queue", window, cx));
            this.send_prompt(window, cx);
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            sender
                .send(Event::Finished {
                    error: Some("previous turn failed".into()),
                    cancelled: false,
                })
                .unwrap();
            this.poll_streams(window, cx);
            assert!(this.session_control.runs.is_empty());
            this.delete_session(1, cx);
            this.remove_workspace(0, cx);
            assert!(this.session(1).is_some());
            assert_eq!(this.store.workspaces.len(), 2);
            let feedback = this.message.clone();
            release
                .send(Err("injected queue save failure".into()))
                .unwrap();
            finish_save(this, window, cx);
            assert!(this.queued_messages.is_empty());
            assert!(this.session_control.queue_saves.is_empty());
            assert_eq!(this.input.read(cx).value().as_ref(), "unsaved queue");
            assert_eq!(
                this.message, feedback,
                "save failures must not overwrite inline feedback"
            );
            let notices = this
                .notifications
                .center
                .records()
                .iter()
                .filter(|notice| notice.title == "消息未加入队列")
                .collect::<Vec<_>>();
            assert_eq!(notices.len(), 1);
            assert!(notices[0].message.contains("草稿已保留"));
            assert_eq!(notices[0].target, this.session_notice_target(1));
            this.poll_notifications(window, cx);
            assert!(!window.notifications(cx).is_empty());
        })
    });
}

#[gpui_kit::test]
fn cleanup_fault_cancels_closing_and_keeps_alias_exclusion_until_finished(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "cleanup-fault");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.session_mut(2).unwrap().native_id = this.session(1).unwrap().native_id.clone();
            let sender = running(this, 1);
            this.closing = true;
            sender
                .send(Event::CleanupBlocked("injected cleanup timeout".into()))
                .unwrap();
            this.poll_streams(window, cx);
            assert!(!this.closing);
            assert!(this.session_busy(1));
            assert!(this.session_busy(2));
            assert!(this.session_control.runs[&1].runtime.is_cancelled());
            assert!(this.session_control.runs[&1].cleanup_error.is_some());
            this.closing = true;
            this.poll(window, cx);
            assert!(!this.closing, "a repeated close must remain recoverable");
            sender
                .send(Event::Finished {
                    error: Some("injected cleanup timeout".into()),
                    cancelled: true,
                })
                .unwrap();
            this.poll_streams(window, cx);
            assert!(!this.session_busy(1));
            assert!(!this.session_busy(2));
            this.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn helper_cleanup_faults_remain_visible_until_verified_completion(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "helper-cleanup");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            for naming in [true, false] {
                let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
                if naming {
                    this.session_control.namings.insert(
                        1,
                        crate::app::SessionNaming {
                            automatic: false,
                            runtime,
                            started: std::time::Instant::now(),
                            parts: vec![],
                            error: None,
                            cleanup_error: None,
                        },
                    );
                } else {
                    this.git.generation = Some(crate::app::CommitGeneration {
                        cancelled: false,
                        path: this.store.workspaces[0].path.clone(),
                        runtime,
                        started: std::time::Instant::now(),
                        original: String::new(),
                        last_diagnostic: None,
                        parts: vec![],
                        error: None,
                        cleanup_error: None,
                    });
                }
                this.closing = true;
                sender
                    .send(Event::CleanupBlocked(
                        "injected helper cleanup timeout".into(),
                    ))
                    .unwrap();
                if naming {
                    this.poll_session_namings(cx);
                } else {
                    this.poll_git_actions(window, cx);
                }
                assert!(!this.closing);
                if naming {
                    assert!(this.session_control.namings[&1].cleanup_error.is_some());
                    assert!(this.session_control.naming_errors.contains_key(&1));
                } else {
                    assert!(
                        this.git
                            .generation
                            .as_ref()
                            .unwrap()
                            .cleanup_error
                            .is_some()
                    );
                    assert!(
                        this.notifications
                            .center
                            .records()
                            .iter()
                            .any(|notice| notice.source == NoticeSource::Git
                                && notice.level == NoticeLevel::Error
                                && notice.target
                                    == Some(NoticeTarget::Workspace(
                                        this.store.workspaces[0].path.clone()
                                    )))
                    );
                }
                this.closing = true;
                this.poll(window, cx);
                assert!(!this.closing);
                sender
                    .send(Event::Finished {
                        error: Some("injected helper cleanup timeout".into()),
                        cancelled: true,
                    })
                    .unwrap();
                if naming {
                    this.poll_session_namings(cx);
                } else {
                    this.poll_git_actions(window, cx);
                }
                assert!(this.session_control.namings.is_empty());
                assert!(this.git.generation.is_none());
            }
            this.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn failed_save_cancels_closing_even_without_a_pending_send(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "close-save-failure");
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            let (release, snapshot) = hold_save(this);
            this.persist();
            snapshot.recv_timeout(Duration::from_secs(3)).unwrap();
            this.closing = true;
            release
                .send(Err("injected close save failure".into()))
                .unwrap();
            // Poll the actual completion, rather than consuming it with test flush.
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while this.persistence.busy() {
                assert!(std::time::Instant::now() < deadline);
                this.poll_persistence(cx);
                std::thread::yield_now();
            }
            assert!(!this.closing);
            assert!(this.dirty);
            assert!(
                this.message
                    .as_deref()
                    .unwrap()
                    .contains("injected close save failure")
            );
        })
    });
}
