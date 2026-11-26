use crate::app::queue::tests::{fixture, running};
use crate::app::{ConduitApp, Event, ItemKind, PermissionLevel, Provider};
use crate::commands::{self as core, Arguments, CommandId, Descriptor, Execution};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    Entity, EntityInputHandler as _, Focusable as _, Modifiers, MouseButton, TestAppContext,
    VisualTestContext,
};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
}

fn draft(app: &Entity<ConduitApp>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.input.update(cx, |input, cx| {
                input.set_value(text, window, cx);
                input.set_selected_range(text.len()..text.len(), cx);
            });
            this.input.read(cx).focus_handle(cx).focus(window, cx);
        })
    });
    draw(cx);
}

fn assert_new_session_controls(cx: &mut VisualTestContext) {
    cx.update(|window, _| {
        for id in ["permission-picker-trigger", "session-data-panel"] {
            assert!(
                gpui_kit::base::test_support::find(
                    window,
                    &[],
                    &gpui_kit::SharedString::from(id).into()
                )
                .is_some_and(|element| element.visible()),
                "missing {id}"
            );
        }
    });
}

fn catalog(app: &mut ConduitApp) -> Descriptor {
    app.session_mut(1).unwrap().provider = Provider::OpenCode;
    app.new_session_provider = Provider::OpenCode;
    let item = Descriptor {
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
            hint: "target".into(),
        },
        execution: Execution::Engine,
    };
    let ticket = app
        .chat_commands
        .catalog
        .begin(app.session_command_context(1).unwrap());
    app.chat_commands
        .catalog
        .publish(&ticket, Ok(vec![item.clone()]));
    item
}

#[gpui_kit::test]
fn commands_new_creates_a_session_with_permissions_and_default_data_sidebar(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-new-session");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.startup = None;
            this.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            this.store.ui.default_session_permission = PermissionLevel::WorkspaceWrite;
            let session = this.session_mut(1).unwrap();
            session.permission = PermissionLevel::FullAccess;
            session.model = Some("workspace-model".into());
            session.reasoning_effort = crate::providers::ReasoningEffort::High;
            this.switch(0, Some(1), window, cx);
            this.close_right_view(window, cx);
        })
    });
    draft(&app, "/n", cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(app.read(cx).selected_session, Some(1));
        assert_eq!(app.read(cx).store.workspaces[0].sessions.len(), 1);
        assert_eq!(app.read(cx).input.read(cx).value().as_ref(), "/new ");
    });
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|window, cx| {
        let this = app.read(cx);
        let id = this.selected_session.expect("/new must create a session");
        assert_ne!(id, 1);
        assert_eq!(this.selected_workspace, Some(0));
        assert_eq!(this.store.workspaces[0].sessions.len(), 2);
        assert!(
            this.open_tabs
                .contains(&crate::app::files::DocumentTab::Chat(id))
        );
        let created = this.session(id).unwrap();
        assert_eq!(created.permission, PermissionLevel::WorkspaceWrite);
        assert_eq!(created.model.as_deref(), Some("workspace-model"));
        assert_eq!(
            created.reasoning_effort,
            crate::providers::ReasoningEffort::High
        );
        assert!(created.native_id.is_none() && created.messages.is_empty());
        assert_eq!(
            this.session(1).unwrap().permission,
            PermissionLevel::FullAccess
        );
        assert_eq!(
            this.session(1).unwrap().native_id.as_deref(),
            Some("queue-first")
        );
        assert!(!this.drafts.contains_key(&1));
        assert!(this.input.read(cx).value().is_empty());
        assert!(this.input.read(cx).focus_handle(cx).is_focused(window));
        assert!(this.session_control.runs.is_empty());
        assert!(
            this.plugins
                .right_is(conduit::plugins::builtins::SESSION_DATA)
        );
    });
    assert_new_session_controls(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_none());
}

#[gpui_kit::test]
fn commands_new_from_workspace_draft_creates_a_session_and_keeps_explicit_model(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-new-workspace");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.startup = None;
            this.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            this.store.ui.default_session_permission = PermissionLevel::ReadOnly;
            this.store.workspaces[0].sessions.clear();
            this.switch(0, None, window, cx);
            let mut selection = crate::app::ComposerSelection::new(Provider::Codex);
            selection.model = Some("explicit-model".into());
            this.new_session_provider = Provider::Codex;
            this.pending_model_selection = Some(selection);
            this.drafts.insert(2, "other session draft".into());
        })
    });
    draft(&app, "/new ", cx);
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.send_prompt(window, cx);
            let id = this.selected_session.expect("/new must create a session");
            let created = this.session(id).unwrap();
            assert_eq!(created.provider, Provider::Codex);
            assert_eq!(created.model.as_deref(), Some("explicit-model"));
            assert_eq!(created.permission, PermissionLevel::ReadOnly);
            assert!(created.native_id.is_none() && created.messages.is_empty());
            assert!(!this.workspace_drafts.contains_key(&root));
            assert_eq!(this.drafts[&2], "other session draft");
            assert!(this.input.read(cx).value().is_empty());
            assert!(this.session_control.runs.is_empty());
            this.persistence.flush();
        })
    });
    draw(cx);
    assert_new_session_controls(cx);
}

#[gpui_kit::test]
fn commands_keyboard_completion_is_separate_from_execution_and_escape_stays_closed(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-keyboard");
    draft(&app, "/", cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    draft(&app, "/s", cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    cx.simulate_keystrokes("z");
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    assert!(cx.debug_bounds("chat-command-empty").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_none());
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_none());
    cx.simulate_keystrokes("backspace");
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_none());
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_none());
    draft(&app, "/se", cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(app.read(cx).input.read(cx).value().as_ref(), "/settings ");
        assert!(!app.read(cx).settings_ui.open);
        assert!(app.read(cx).session_control.runs.is_empty());
    });
    assert!(cx.debug_bounds("chat-command-menu").is_none());
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        assert!(app.read(cx).settings_ui.open);
        assert!(app.read(cx).input.read(cx).value().is_empty());
    });
}

#[gpui_kit::test]
fn commands_preserve_arguments_attachments_ime_and_reject_busy_or_stale_targets(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-boundaries");
    let file = root.join("reference.txt");
    std::fs::write(&file, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            catalog(this);
            this.attach_paths(std::slice::from_ref(&file), window, cx);
        })
    });
    draft(&app, "/rev   中文 \"a b\"", cx);
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_selected_range(4..4, cx))
        })
    });
    draw(cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert_eq!(
                this.input.read(cx).value().as_ref(),
                "/review   中文 \"a b\""
            );
            assert_eq!(this.current_files().unwrap().references.len(), 1);
            this.send_prompt(window, cx);
            assert!(this.session_control.runs.is_empty());
            this.composer_files.clear();
            let _sender = running(this, 1);
            this.send_prompt(window, cx);
            assert!(this.queued_messages.is_empty());
            assert!(this.input.read(cx).value().starts_with("/review"));
            this.session_control.runs.clear();
            this.session_mut(1).unwrap().permission = PermissionLevel::FullAccess;
            this.send_prompt(window, cx);
            assert!(this.session_control.runs.is_empty());
            assert!(this.notifications.center.records().iter().any(|notice| {
                notice.message == conduit::i18n::text_static("commands.stale")
                    && notice.level == crate::notifications::Level::Warning
                    && notice.target == this.session_notice_target(1)
            }));
            this.input.update(cx, |input, cx| {
                input.set_value("/s", window, cx);
                input.set_selected_range(2..2, cx);
                input.replace_and_mark_text_in_range(None, "中", Some(1..1), window, cx);
            });
            this.send_prompt(window, cx);
            assert!(!this.settings_ui.open);
            this.input
                .update(cx, |input, cx| input.unmark_text(window, cx));
        })
    });
    let _sender = cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.attach_paths(std::slice::from_ref(&file), window, cx);
            running(this, 1)
        })
    });
    draft(&app, "/does-not-exist", cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    assert!(cx.debug_bounds("chat-command-empty").is_some());
    cx.update(|_, cx| {
        let this = app.read(cx);
        assert!(this.chat_commands.results.is_empty());
        assert!(this.queued_messages.is_empty());
        assert_eq!(this.input.read(cx).value().as_ref(), "/does-not-exist");
    });
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        let queued = &app.read(cx).queued_messages[&1];
        assert_eq!(queued.text, "/does-not-exist");
        assert_eq!(queued.references.len(), 1);
        assert_eq!(queued.references[0].path, file.canonicalize().unwrap());
    });
}

#[gpui_kit::test]
fn commands_empty_search_keeps_discovery_and_caret_navigation_available(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-empty-search");
    cx.update(|_, cx| {
        app.update(cx, |this, _| {
            this.session_mut(1).unwrap().provider = Provider::OpenCode;
            this.new_session_provider = Provider::OpenCode;
        })
    });
    draft(&app, "/review", cx);
    assert!(cx.debug_bounds("chat-command-empty").is_some());
    assert!(cx.debug_bounds("connect-chat-commands").is_some());
    cx.update(|window, cx| {
        let this = app.read(cx);
        assert!(this.chat_command_menu_open(window, cx));
        assert!(!this.chat_commands_open(window, cx));
        assert!(this.session_control.runs.is_empty());
    });
    cx.simulate_keystrokes("up");
    draw(cx);
    cx.update(|_, cx| {
        // The fixture can wrap the text; Up must move the caret rather than
        // get swallowed by an empty command list, regardless of line width.
        let selection = app.read(cx).input.read(cx).selected_range();
        assert!(selection.is_empty() && selection.start < "/review".len());
        assert_eq!(app.read(cx).input.read(cx).value().as_ref(), "/review");
    });
    draft(&app, "/review", cx);
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            catalog(this);
            this.refresh_chat_commands(cx);
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("chat-command-empty").is_none());
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(app.read(cx).input.read(cx).value().as_ref(), "/review ");
        assert!(app.read(cx).session_control.runs.is_empty());
    });
}

#[gpui_kit::test]
fn commands_short_names_keep_host_and_engine_collisions_bound_to_the_selected_source(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-short-name-collision");
    let (ticket, native) = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let mut native = catalog(app);
            native.name = "help".into();
            native.aliases = vec!["skill".into()];
            let ticket = app
                .chat_commands
                .catalog
                .begin(app.session_command_context(1).unwrap());
            app.chat_commands
                .catalog
                .publish(&ticket, Ok(vec![native.clone()]));
            (ticket, native)
        })
    });
    for (name, action) in [
        ("help", core::HostAction::Help),
        ("skill", core::HostAction::Skill),
    ] {
        draft(&app, &format!("/{name}"), cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.chat_commands.accepted = None;
                let text = app.input.read(cx).value().to_string();
                assert_eq!(app.chat_commands.results.len(), 2);
                assert_eq!(
                    app.prepare_chat_submission(&text, &core::parse(&text).unwrap())
                        .unwrap_err(),
                    conduit::i18n::text("commands.ambiguous"),
                );
                app.send_prompt(window, cx);
                assert_eq!(app.input.read(cx).value().as_ref(), text);
                assert!(app.session_control.runs.is_empty());

                for selected in [
                    native.id.clone(),
                    CommandId {
                        source: "conduit".into(),
                        local: name.into(),
                    },
                ] {
                    app.input.update(cx, |input, cx| {
                        input.set_value(&text, window, cx);
                        input.set_selected_range(text.len()..text.len(), cx);
                    });
                    app.chat_commands.dismissed = None;
                    app.refresh_chat_commands(cx);
                    app.chat_commands.selected = Some(selected.clone());
                    assert!(app.accept_chat_command(window, cx));
                    let completed = app.input.read(cx).value().to_string();
                    let (command, context) = app
                        .prepare_chat_submission(&completed, &core::parse(&completed).unwrap())
                        .unwrap()
                        .unwrap();
                    assert_eq!(command.id, selected);
                    if selected == native.id {
                        assert_eq!(command.execution, Execution::Engine);
                        assert_eq!(completed, "/help ");
                        assert!(
                            app.chat_commands
                                .catalog
                                .prepare(&context, &command, completed.clone())
                                .is_ok()
                        );
                        app.chat_commands.catalog.publish(&ticket, Ok(vec![]));
                        app.send_prompt(window, cx);
                        assert_eq!(app.input.read(cx).value().as_ref(), completed);
                        assert!(app.session_control.runs.is_empty());
                        app.chat_commands
                            .catalog
                            .publish(&ticket, Ok(vec![native.clone()]));
                    } else {
                        assert_eq!(command.execution, Execution::Host(action));
                        assert_eq!(completed, format!("/{name} "));
                    }
                }
                assert!(app.queued_messages.is_empty());
            })
        });
    }
}

#[gpui_kit::test]
fn commands_query_changes_select_best_match_and_completed_calls_enforce_size_limit(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-selection");
    let (ticket, items) = cx.update(|_, cx| {
        app.update(cx, |this, _| {
            let review = catalog(this);
            let mut reviewer = review.clone();
            reviewer.name = "reviewer".into();
            reviewer.id.local = "reviewer".into();
            let ticket = this
                .chat_commands
                .catalog
                .begin(this.session_command_context(1).unwrap());
            let items = vec![review, reviewer];
            this.chat_commands
                .catalog
                .publish(&ticket, Ok(items.clone()));
            (ticket, items)
        })
    });
    draft(&app, "/rev", cx);
    cx.simulate_keystrokes("down");
    draw(cx);
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            assert_eq!(this.chat_commands.selected.as_ref(), Some(&items[1].id));
            let mut refreshed = items.clone();
            refreshed[1].summary = "Updated command description".into();
            this.chat_commands.catalog.publish(&ticket, Ok(refreshed));
            this.refresh_chat_commands(cx);
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).chat_commands.selected.as_ref(),
            Some(&items[1].id)
        );
    });
    draft(&app, "/review", cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|_, cx| {
        let this = app.read(cx);
        assert_eq!(this.input.read(cx).value().as_ref(), "/review ");
        assert!(this.chat_commands.accepted.is_some());
        let text = format!("/review {}", "x".repeat(core::MAX_INPUT_BYTES));
        assert_eq!(
            this.prepare_chat_submission(&text, &core::parse(&text).unwrap())
                .unwrap_err(),
            conduit::i18n::text("commands.arguments"),
        );
        assert!(this.session_control.runs.is_empty());
        assert!(this.queued_messages.is_empty());
    });
}

#[gpui_kit::test]
fn commands_help_is_local_for_every_engine_and_preserves_attachments(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-help");
    let file = root.join("reference.txt");
    std::fs::write(&file, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.attach_paths(std::slice::from_ref(&file), window, cx);
            let files = this.current_files().unwrap().references.clone();
            for provider in Provider::ALL {
                this.session_mut(1).unwrap().provider = provider;
                this.new_session_provider = provider;
                this.input.update(cx, |input, cx| {
                    input.set_value("/help ", window, cx);
                });
                this.send_prompt(window, cx);
                assert_eq!(this.input.read(cx).value().as_ref(), "/");
                assert!(this.chat_commands_open(window, cx));
                assert!(
                    this.chat_commands
                        .results
                        .iter()
                        .any(|item| item.name == "rename")
                );
                assert_eq!(this.current_files().unwrap().references, files);
                assert!(this.session_control.runs.is_empty());
                assert!(this.queued_messages.is_empty());
            }
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
}

#[gpui_kit::test]
fn commands_extension_actions_are_local_and_preserve_attachments(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-extension-actions");
    let file = root.join("reference.txt");
    std::fs::write(&file, "reference").unwrap();
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            // This provider reports native-only discovery, without scanning user config files.
            app.new_session_provider = Provider::Gemini;
            app.attach_paths(std::slice::from_ref(&file), window, cx);
            let files = app.current_files().unwrap().references.clone();
            let messages = app.session(1).unwrap().messages.len();
            for command in ["/skill", "/mcp"] {
                let invalid = format!("{command} unexpected");
                app.input
                    .update(cx, |input, cx| input.set_value(invalid.clone(), window, cx));
                app.send_prompt(window, cx);
                assert_eq!(app.input.read(cx).value().as_ref(), invalid);
                assert!(!window.has_active_dialog(cx));

                app.input
                    .update(cx, |input, cx| input.set_value(command, window, cx));
                assert!(app.host_chat_command_ready(cx));
                app.send_prompt(window, cx);
                assert!(window.has_active_dialog(cx));
                assert!(app.input.read(cx).value().is_empty());
                assert_eq!(app.current_files().unwrap().references, files);
                assert_eq!(app.session(1).unwrap().messages.len(), messages);
                assert_eq!(app.store.workspaces[0].sessions.len(), 1);
                assert!(app.session_control.runs.is_empty());
                assert!(app.queued_messages.is_empty());
                assert!(!app.settings_ui.open);
                assert!(app.extensions_ui.probe.is_none());
                assert!(app.extensions_ui.connection_probe.is_none());
                window.close_dialog(cx);
            }
        });
    });
}

#[gpui_kit::test]
fn commands_host_actions_can_be_clicked_during_a_run_with_a_full_queue(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "commands-host-send-button");
    let file = root.join("reference.txt");
    std::fs::write(&file, "reference").unwrap();
    let _sender = cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.startup = None;
            this.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            this.attach_paths(std::slice::from_ref(&file), window, cx);
            this.queued_messages.insert(
                1,
                crate::store::QueuedMessage {
                    text: "follow-up".into(),
                    references: vec![],
                    paused: None,
                },
            );
            running(this, 1)
        })
    });
    for text in ["/pin ", "/stop "] {
        draft(&app, text, cx);
        assert!(cx.debug_bounds("stop").is_none());
        let center = cx.debug_bounds("send").unwrap().center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            let this = app.read(cx);
            assert!(this.input.read(cx).value().is_empty());
            assert_eq!(this.current_files().unwrap().references.len(), 1);
            assert_eq!(this.queued_messages[&1].text, "follow-up");
        });
    }
    cx.update(|_, cx| {
        let this = app.read(cx);
        assert!(this.session(1).unwrap().pinned);
        assert!(this.session_control.runs[&1].runtime.is_cancelled());
        assert!(this.queued_messages[&1].paused.is_some());
    });
}

#[gpui_kit::test]
fn commands_stop_rechecks_live_availability_and_missing_sessions_preserve_drafts(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-stop-availability");
    draft(&app, "/stop", cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert!(
                this.chat_commands
                    .accepted
                    .as_ref()
                    .unwrap()
                    .1
                    .unavailable
                    .is_some()
            );
            let _sender = running(this, 1);
            this.send_prompt(window, cx);
            assert!(this.session_control.runs[&1].runtime.is_cancelled());
            assert!(this.input.read(cx).value().is_empty());
            this.session_control.runs.clear();
        })
    });
    let _sender = cx.update(|_, cx| app.update(cx, |this, _| running(this, 1)));
    draft(&app, "/stop", cx);
    cx.simulate_keystrokes("tab");
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert!(
                this.chat_commands
                    .accepted
                    .as_ref()
                    .unwrap()
                    .1
                    .unavailable
                    .is_none()
            );
            this.session_control.runs.clear();
            this.send_prompt(window, cx);
            assert_eq!(this.input.read(cx).value().as_ref(), "/stop ");
            assert!(this.notifications.center.records().iter().any(|notice| {
                notice.message == conduit::i18n::text_static("commands.nothing_running")
            }));
            this.switch(0, None, window, cx);
            for text in ["/rename title", "/pin", "/unpin", "/stop"] {
                this.input
                    .update(cx, |input, cx| input.set_value(text, window, cx));
                this.send_prompt(window, cx);
                assert_eq!(this.input.read(cx).value().as_ref(), text);
                assert!(this.session_control.runs.is_empty());
                assert!(this.queued_messages.is_empty());
            }
            assert_eq!(this.store.workspaces[0].sessions.len(), 1);
            assert!(this.notifications.center.records().iter().any(|notice| {
                notice.message == conduit::i18n::text_static("commands.requires_session")
            }));
        })
    });
}

#[gpui_kit::test]
fn commands_catalog_and_terminal_results_stay_bound_to_the_original_run(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-results");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let item = catalog(this);
            let context = this.session_command_context(1).unwrap();
            let call = this
                .chat_commands
                .catalog
                .prepare(&context, &item, "/review".into())
                .unwrap();
            let sender = running(this, 1);
            let ticket = this.chat_commands.catalog.begin(context.clone());
            let run = this.session_control.runs.get_mut(&1).unwrap();
            run.intent = core::RequestIntent::Command(Box::new(call));
            run.command_ticket = Some(ticket);
            this.switch(1, Some(2), window, cx);
            this.input
                .update(cx, |input, cx| input.set_value("other draft", window, cx));
            sender.send(Event::CommandCatalog(Ok(vec![]))).unwrap();
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            this.poll_streams(window, cx);
            assert!(
                this.chat_commands
                    .catalog
                    .get(&context)
                    .unwrap()
                    .commands
                    .is_empty()
            );
            assert_eq!(this.selected_session, Some(2));
            assert_eq!(this.input.read(cx).value().as_ref(), "other draft");
            assert!(
                this.session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .any(|item| item.kind == ItemKind::Notice && item.text.contains("命令已完成"))
            );
            assert!(this.session(2).unwrap().messages.is_empty());
            this.switch(0, Some(1), window, cx);
            let item = catalog(this);
            let context = this.session_command_context(1).unwrap();
            let call = this
                .chat_commands
                .catalog
                .prepare(&context, &item, "/review".into())
                .unwrap();
            this.input
                .update(cx, |input, cx| input.set_value("/review", window, cx));
            let sent = this.sent_composer(cx);
            let sender = running(this, 1);
            let run = this.session_control.runs.get_mut(&1).unwrap();
            run.intent = core::RequestIntent::Command(Box::new(call));
            run.command_draft = sent;
            run.cancel();
            this.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            sender
                .send(Event::Text {
                    key: "late".into(),
                    text: "late output".into(),
                })
                .unwrap();
            for _ in 0..2 {
                sender
                    .send(Event::Finished {
                        error: None,
                        cancelled: true,
                    })
                    .unwrap();
            }
            this.poll_streams(window, cx);
            assert_eq!(this.input.read(cx).value().as_ref(), "/review");
            assert!(
                !this
                    .session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .any(|item| item.text == "late output")
            );
            assert_eq!(
                this.session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .filter(|item| item.text.contains("已取消"))
                    .count(),
                1
            );
        })
    });
}
