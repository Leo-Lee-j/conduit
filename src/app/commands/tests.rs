use crate::app::queue::tests::{fixture, running};
use crate::app::{ConduitApp, Event, ItemKind, PermissionLevel, Provider};
use crate::commands::{self as core, Arguments, CommandId, Descriptor, Execution};
use gpui_kit::{
    Entity, EntityInputHandler as _, Focusable as _, TestAppContext, VisualTestContext,
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
fn commands_keyboard_completion_is_separate_from_execution_and_escape_stays_closed(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "commands-keyboard");
    draft(&app, "/", cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    draft(&app, "/conduit:s", cx);
    assert!(cx.debug_bounds("chat-command-menu").is_some());
    cx.simulate_keystrokes("z");
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
    draft(&app, "/conduit:se", cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).input.read(cx).value().as_ref(),
            "/conduit:settings "
        );
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
            assert_eq!(
                this.message.as_deref(),
                Some(conduit::i18n::text_static("commands.stale"))
            );
            this.input.update(cx, |input, cx| {
                input.set_value("/conduit:s", window, cx);
                input.set_selected_range(10..10, cx);
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
    assert!(cx.debug_bounds("chat-command-menu").is_none());
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
