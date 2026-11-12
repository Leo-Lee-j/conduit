use super::{
    ConduitApp, ModelChannel, PermissionLevel, Preset, Provider, ReasoningEffort, SessionDraft,
    Store, Workspace,
};
use gpui_kit::{AppContext as _, TestAppContext};

#[gpui_kit::test]
fn reply_branch_opens_handoff_without_reentering_the_app(cx: &mut TestAppContext) {
    use crate::app::queue::tests::fixture;
    use gpui_kit::{Modifiers, MouseButton};

    let _locale = crate::test_support::chinese();
    let (app, cx, _) = fixture(cx, "reply-branch-open");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().messages = serde_json::from_value(serde_json::json!([
                {"key":"user","kind":"User","text":"first"},
                {"key":"reply","kind":"Assistant","text":"answer"},
                {"key":"later","kind":"User","text":"later"}
            ]))
            .unwrap();
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let point = cx.debug_bounds("reply-branch-button-1").unwrap().center();
    cx.simulate_mouse_move(point, None, Modifiers::default());
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|window, _| {
        assert!(
            gpui_kit::base::test_support::find(window, &[], &"native-fork-create".into()).is_some()
        );
    });
}

#[gpui_kit::test]
fn native_branch_completion_binds_history_without_dispatching_work(cx: &mut TestAppContext) {
    use crate::app::queue::tests::{fixture, running};
    use crate::{branching, commands::RequestIntent, protocol::Event};
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = fixture(cx, "native-branch-completion");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.persistence.flush();
            this.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            let source = this.session_mut(1).unwrap();
            source.provider = Provider::Codex;
            source.messages = serde_json::from_value(serde_json::json!([
                {"key":"user","kind":"User","text":"first"},
                {"key":"reply","kind":"Assistant","text":"answer"}
            ]))
            .unwrap();
            let sender = running(this, 1);
            sender
                .send(Event::NativeTurnCompleted {
                    thread_id: "queue-first".into(),
                    turn_id: "completed-turn".into(),
                })
                .unwrap();
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            this.poll_streams(window, cx);
            let source = this.session(1).unwrap();
            assert_eq!(
                branching::point_at(source, 2).unwrap().request.last_turn_id,
                "completed-turn"
            );
            let fork = branching::prepare(source, 2, 3, 100).unwrap();
            let request = branching::request(&fork).unwrap();
            this.store.workspaces[0].sessions.insert(0, fork);
            let sender = running(this, 3);
            this.session_control.runs.get_mut(&3).unwrap().intent = RequestIntent::Fork(request);
            this.store
                .scheduler
                .add(
                    Preset {
                        id: 1,
                        workspace: root,
                        name: "On completion".into(),
                        prompt: "must not run on fork".into(),
                        provider: Provider::Codex,
                        channel: ModelChannel::Cli,
                        model: None,
                        effort: ReasoningEffort::Default,
                        acp_variant: None,
                        permission: PermissionLevel::ReadOnly,
                        archived: false,
                    },
                    None,
                    0,
                )
                .unwrap();
            this.queued_messages.insert(
                3,
                crate::store::QueuedMessage {
                    text: "must not automatically send".into(),
                    references: vec![],
                    paused: None,
                },
            );
            let observations = this.store.usage_ledger.observations.len();
            sender.send(Event::NativeId("native-fork".into())).unwrap();
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            this.poll_streams(window, cx);
            let fork = this.session(3).unwrap();
            assert_eq!(fork.native_id.as_deref(), Some("native-fork"));
            assert_eq!(
                branching::point_at(fork, 2).unwrap().request.thread_id,
                "native-fork"
            );
            assert!(!this.session_fork_pending(3));
            assert!(this.session_control.runs.is_empty());
            assert!(this.store.scheduler.runs.is_empty());
            assert_eq!(this.store.usage_ledger.observations.len(), observations);
            assert!(this.queued_messages.contains_key(&3));
            assert_eq!(
                this.session(1).unwrap().native_id.as_deref(),
                Some("queue-first")
            );
            this.persistence.flush();
        })
    });
}

#[gpui_kit::test]
fn preset_sessions_preserve_the_source_draft_and_never_reuse_native_context(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    let workspace = std::env::temp_dir().join("conduit-workflow-test");
    cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            let mut source = crate::app::tests::session();
            source.native_id = Some("source-native-id".into());
            let store = Store {
                workspaces: vec![Workspace {
                    path: workspace.clone(),
                    sessions: vec![source],
                }],
                ..Default::default()
            };
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            app.switch(0, Some(1), window, cx);
            app.input.update(cx, |input, cx| {
                input.set_value("original draft", window, cx)
            });
            let mut preset = Preset {
                id: 1,
                workspace: workspace.clone(),
                name: "Review".into(),
                prompt: "review this change".into(),
                provider: Provider::Codex,
                channel: ModelChannel::Cli,
                model: Some("preset-model".into()),
                effort: ReasoningEffort::High,
                acp_variant: None,
                permission: PermissionLevel::ReadOnly,
                archived: false,
            };
            let id = app
                .workflow_session(
                    SessionDraft {
                        workspace: &workspace,
                        provider: preset.provider,
                        title: &preset.name,
                        prompt: &preset.prompt,
                        preset: Some(&preset),
                    },
                    window,
                    cx,
                )
                .unwrap();
            assert_eq!(app.drafts[&1], "original draft");
            assert_eq!(app.input.read(cx).value().as_ref(), "review this change");
            let session = app.session(id).unwrap();
            assert!(session.native_id.is_none());
            assert!(session.messages.is_empty());
            assert_eq!(session.permission, PermissionLevel::ReadOnly);
            assert_eq!(session.model.as_deref(), Some("preset-model"));
            assert_eq!(session.reasoning_effort, ReasoningEffort::High);
            assert!(app.session_control.runs.is_empty());
            let count = app.store.workspaces[0].sessions.len();
            preset.workspace = workspace.join("other");
            assert!(
                app.workflow_session(
                    SessionDraft {
                        workspace: &workspace,
                        provider: preset.provider,
                        title: &preset.name,
                        prompt: &preset.prompt,
                        preset: Some(&preset)
                    },
                    window,
                    cx
                )
                .is_err()
            );
            assert_eq!(app.store.workspaces[0].sessions.len(), count);
            app.persistence.flush();
            app.writable = false;
            app
        });
        gpui_kit::component::Root::new(view, window, cx)
    });
}
