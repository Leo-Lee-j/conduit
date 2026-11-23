use super::{
    ConduitApp, ModelChannel, PermissionLevel, Preset, Provider, ReasoningEffort, SessionDraft,
    Store, Workspace,
};
use gpui_kit::{AppContext as _, TestAppContext};

#[gpui_kit::test]
fn handoff_drafts_follow_the_selected_session_and_restore_on_return(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "workflow-handoff-sources");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            // Opening the workspace before selecting a session must not pin None.
            app.selected_session = None;
            app.activate_workflow(window, cx);
            let panel = app.workflow_panels[&root].clone();
            assert_eq!(panel.read(cx).source_session, None);
            app.selected_session = Some(1);
            app.activate_workflow(window, cx);
            assert_eq!(panel.read(cx).source_session, Some(1));
            panel.update(cx, |panel, cx| {
                panel
                    .through
                    .update(cx, |input, cx| input.set_value("1", window, cx));
                panel.context.update(cx, |input, cx| {
                    input.set_value("edited handoff A", window, cx)
                });
            });
            let mut second = app.session(1).unwrap().clone();
            second.id = 3;
            app.store.workspaces[0].sessions.push(second);
            app.selected_session = Some(3);
            app.activate_workflow(window, cx);
            assert_eq!(panel.read(cx).source_session, Some(3));
            assert!(panel.read(cx).context.read(cx).value().is_empty());
            panel.read(cx).context.clone().update(cx, |input, cx| {
                input.set_value("edited handoff B", window, cx)
            });
            app.selected_session = Some(1);
            app.activate_workflow(window, cx);
            assert_eq!(panel.read(cx).source_session, Some(1));
            assert_eq!(
                panel.read(cx).context.read(cx).value().as_ref(),
                "edited handoff A"
            );
            assert_eq!(panel.read(cx).through.read(cx).value().as_ref(), "1");
            app.selected_session = Some(3);
            app.activate_workflow(window, cx);
            assert_eq!(
                panel.read(cx).context.read(cx).value().as_ref(),
                "edited handoff B"
            );
        })
    });
}

#[gpui_kit::test]
fn sidebar_drafts_survive_close_and_workspace_switches(cx: &mut TestAppContext) {
    use conduit::plugins::{builtins, ids::ViewId};
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "workflow-sidebar-drafts");
    let other = root.join("other");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            let id = ViewId::new(builtins::WORKFLOW).unwrap();
            assert!(app.open_right_view(&id, window, cx));
            let panel = app.workflow_panels[&root].clone();
            panel.read(cx).title.clone().update(cx, |input, cx| {
                input.set_value("unfinished task", window, cx)
            });
            app.close_right_view(window, cx);
            assert!(app.open_right_view(&id, window, cx));
            assert_eq!(app.workflow_panels[&root], panel);
            assert_eq!(
                panel.read(cx).title.read(cx).value().as_ref(),
                "unfinished task"
            );
            app.switch(1, None, window, cx);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.plugins.right_is(builtins::WORKFLOW));
            let second = app.workflow_panels[&other].read(cx);
            assert_eq!(second.workspace, other);
            assert!(second.title.read(cx).value().is_empty());
            app.switch(0, Some(1), window, cx);
            let first = app.workflow_panels[&root].read(cx);
            assert_eq!(first.title.read(cx).value().as_ref(), "unfinished task");
            assert!(app.session_control.runs.is_empty());
        });
    });
}

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

#[gpui_kit::test]
fn reopened_panel_saves_current_session_configuration(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "review-preset-source");
    let panel = cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.persistence.flush();
            this.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            this.startup = None;
            this.session_mut(1).unwrap().model = Some("model-A".into());
            let mut second = this.session(1).unwrap().clone();
            second.id = 3;
            second.model = Some("model-B".into());
            this.store.workspaces[0].sessions.push(second);
            this.next_id = 4;
            this.activate_workflow(window, cx);
            this.close_right_view(window, cx);
            this.switch(0, Some(3), window, cx);
            this.open_right_view(
                &conduit::plugins::ids::ViewId::new(conduit::plugins::builtins::WORKFLOW).unwrap(),
                window,
                cx,
            );
            this.workflow_panels[&root].clone()
        })
    });
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel.preset_name.update(cx, |input, cx| {
                input.set_value("current configuration", window, cx)
            });
            panel
                .preset_prompt
                .update(cx, |input, cx| input.set_value("review", window, cx));
            panel.save_preset(cx);
            assert!(panel.error.is_none(), "{:?}", panel.error);
        })
    });
    cx.update(|_, cx| {
        app.update(cx, |this, _| {
            this.persistence.flush();
            this.writable = false;
            assert_eq!(
                this.store.workflow.presets.last().unwrap().model.as_deref(),
                Some("model-B")
            );
        })
    });
}

#[gpui_kit::test]
fn message_handoff_preserves_task_and_preset_drafts(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, root) = crate::app::queue::tests::fixture(cx, "review-draft-loss");
    let panel = cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.persistence.flush();
            this.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
            this.startup = None;
            this.session_mut(1).unwrap().messages = serde_json::from_value(serde_json::json!([
                {"key":"user","kind":"User","text":"question"},
                {"key":"assistant","kind":"Assistant","text":"answer"}
            ]))
            .unwrap();
            this.activate_workflow(window, cx);
            this.workflow_panels[&root].clone()
        })
    });
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel
                .title
                .update(cx, |input, cx| input.set_value("unsaved task", window, cx));
            panel.preset_prompt.update(cx, |input, cx| {
                input.set_value("unsaved template", window, cx)
            });
        })
    });
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.open_handoff_at(1, 1, window, cx);
            this.persistence.flush();
            this.writable = false;
            let panel = this.workflow_panels[&root].read(cx);
            assert_eq!(panel.title.read(cx).value().as_ref(), "unsaved task");
            assert_eq!(
                panel.preset_prompt.read(cx).value().as_ref(),
                "unsaved template"
            );
        })
    });
}
