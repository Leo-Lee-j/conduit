use super::{ChatItem, file_ref};
use crate::app::{NoticeLevel, NoticeSource};
use gpui_kit::{TestAppContext, point, px};

#[gpui_kit::test]
fn thinking_disclosure_preserves_manual_choices_across_streaming_and_sessions(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{Modifiers, MouseButton, VisualTestContext, size};
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = super::queue::tests::fixture(cx, "thinking-disclosure");
    let draw = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    };
    let click = |cx: &mut VisualTestContext, selector: &'static str| {
        let center = cx.debug_bounds(selector).expect(selector).center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    };
    cx.simulate_resize(size(px(1100.), px(700.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.set_reasoning_collapsed(true);
            for id in [1, 2] {
                app.session_mut(id).unwrap().messages = serde_json::from_value(serde_json::json!([
                    {"key":"user","kind":"User","text":"Check the code"},
                    {"key":"turn-thought","kind":"Reasoning","text":"Checking the code"},
                    {"key":"tool","kind":"Tool","text":"Read","status":"completed"},
                    {"key":"reply","kind":"Assistant","text":"Done"}
                ]))
                .unwrap();
            }
            app.sync_chat_rows();
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-1").is_none());
    click(cx, "activity-1-1");
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-1").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.set_reasoning_collapsed(false);
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "activity-1-1");
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-1").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            super::apply_event(
                app.session_mut(1).unwrap(),
                "turn",
                crate::protocol::Event::ReasoningSummary {
                    key: "thought".into(),
                    text: " and checking again".into(),
                    summary_index: 0,
                    snapshot: false,
                },
            );
            app.sync_chat_rows();
            cx.notify();
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("reasoning-body-1-1").is_none(),
        "streaming must respect a manual collapse"
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.switch(1, Some(2), window, cx);
            app.sync_chat_rows();
            cx.notify();
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("reasoning-body-2-1").is_some(),
        "same message key in another session follows the default"
    );
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.set_reasoning_collapsed(true);
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-2-1").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.switch(0, Some(1), window, cx);
            app.set_reasoning_collapsed(false);
            app.sync_chat_rows();
            cx.notify();
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("reasoning-body-1-1").is_none(),
        "explicit choice survives switching sessions and defaults"
    );
}

#[gpui_kit::test]
fn error_retry_keeps_partial_output_when_context_validation_fails(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Workspace, protocol::Event, store::Store};
    use gpui_kit::AppContext as _;

    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            // A missing native context must remain visible to send validation;
            // removing partial replies used to hide it and attempt a fresh turn.
            let user: ChatItem = serde_json::from_value(serde_json::json!({
                "key": "turn-0-user", "kind": "User", "text": "检查项目"
            }))
            .unwrap();
            session.messages.push(user);
            super::apply_event(
                &mut session,
                "turn-0",
                Event::Delta {
                    key: "reply".into(),
                    text: "已检查部分文件。".into(),
                },
            );
            super::apply_event(
                &mut session,
                "turn-0",
                Event::ErrorBanner("API Error: 503 unavailable".into()),
            );
            let before = serde_json::to_value(&session.messages).unwrap();
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.retry_error(2, window, cx);
            assert_eq!(
                serde_json::to_value(&app.session(1).unwrap().messages).unwrap(),
                before
            );
            assert!(app.session_control.runs.is_empty());
            assert!(app.message.is_none());
            assert!(app.notifications.center.records().iter().any(|notice| {
                notice.title == "会话无法启动" && notice.message.contains("缺少可靠的 CLI 会话标识")
            }));
            app.writable = false;
            app
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}

#[gpui_kit::test]
fn error_retry_clears_cards_before_repeated_failures_and_preserves_output(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Workspace, protocol::Event, store::Store};
    use gpui_kit::AppContext as _;

    let root = std::env::temp_dir().join(format!("conduit-error-retry-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let shim = root.join("offline.cmd");
    std::fs::write(&shim, "@echo off\r\nexit /b 0\r\n").unwrap();
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.native_id = Some("retry-context".into());
            session.messages = serde_json::from_value(serde_json::json!([
                {"key": "turn-0-user", "kind": "User", "text": "check project"},
                {"key": "turn-0-reply", "kind": "Assistant", "text": "checked configuration"},
                {"key": "turn-4-tool", "kind": "Tool", "text": "read file", "detail": "original output"},
                {"key": "turn-4-error", "kind": "Error", "text": "API Error: 503 unavailable"},
                {"key": "turn-5-error", "kind": "Error", "text": "API Error: 429 rate limit"},
                {"key": "turn-5-notice", "kind": "Notice", "text": "waiting to retry"}
            ]))
            .unwrap();
            let store = Store {
                version: 2,
                workspaces: vec![Workspace {
                    path: root.clone(),
                    sessions: vec![session],
                }],
                cli: crate::cli::CliSettings {
                    claude_path: Some(shim.clone()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.store.ui.auto_rename = false;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            for attempt in 0..3 {
                let session = app.session(1).unwrap();
                let kept = session
                    .messages
                    .iter()
                    .filter(|item| item.kind != super::ItemKind::Error)
                    .cloned()
                    .collect::<Vec<_>>();
                let error_index = session
                    .messages
                    .iter()
                    .rposition(|item| item.kind == super::ItemKind::Error)
                    .unwrap();
                app.retry_error(error_index, window, cx);
                assert!(app.session_control.runs.contains_key(&1), "{:?}", app.message);
                assert_eq!(
                    serde_json::to_value(&app.session(1).unwrap().messages).unwrap(),
                    serde_json::to_value(&kept).unwrap(),
                    "retry must clear old errors before any new output arrives"
                );
                app.persistence.flush();
                let saved = Store::load().unwrap();
                let saved_messages = crate::transcript::read(&saved.workspaces[0].sessions[0]).unwrap();
                // Loading a running turn also appends an interruption notice.
                assert_eq!(
                    serde_json::to_value(&saved_messages[..kept.len()]).unwrap(),
                    serde_json::to_value(&kept).unwrap(),
                    "reopening must retain all existing output"
                );
                assert!(
                    saved_messages.iter().all(|item| item.kind != super::ItemKind::Error),
                    "cleared cards must stay cleared after reopening"
                );
                let mut run = app.session_control.runs.remove(&1).unwrap();
                run.cancel();
                let session = app.session_mut(1).unwrap();
                assert!(
                    session.messages.iter().all(|item| {
                        !item.key.starts_with(&format!("{}-", run.prefix))
                    }),
                    "retry must not reuse a retained message's turn prefix"
                );
                super::apply_event(
                    session,
                    &run.prefix,
                    Event::Tool {
                        key: "tool".into(),
                        title: "read file".into(),
                        detail: format!("retry output {attempt}"),
                        status: "completed".into(),
                    },
                );
                super::apply_event(
                    session,
                    &run.prefix,
                    Event::ErrorBanner("API Error: 503 unavailable".into()),
                );
                assert_eq!(
                    serde_json::to_value(&session.messages[..kept.len()]).unwrap(),
                    serde_json::to_value(&kept).unwrap(),
                    "new output must not overwrite a tool from an earlier attempt"
                );
                assert_eq!(session.messages.len(), kept.len() + 2);
                assert_eq!(
                    session.messages.iter().filter(|item| item.kind == super::ItemKind::Error).count(),
                    1
                );
                assert_eq!(session.native_id.as_deref(), Some("retry-context"));
            }
            app.writable = false;
            app
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}

#[gpui_kit::test]
fn settings_connections_switch_engines_and_guard_service_editors(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::domain::Provider;
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::with_store(
                window,
                cx,
                Ok(crate::store::Store::default()),
                false,
                false,
            );
            app.writable = false;
            // Keep the fixture independent from local CLI/CCS background reads.
            app.connection_settings.syncing = true;
            app.store.providers.clear();
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    for (width, height) in [(850., 500.), (1280., 900.)] {
        cx.simulate_resize(gpui_kit::size(px(width), px(height)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.settings_ui.page = 3;
                app.open_settings(window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let navigation = cx
            .debug_bounds("settings-nav-3")
            .expect("connections navigation");
        cx.simulate_mouse_down(navigation.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(navigation.center(), MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let body = cx.debug_bounds("settings-workspace-body").unwrap();
        let engine_bounds = [
            "settings-engine-0",
            "settings-engine-1",
            "settings-engine-2",
            "settings-engine-3",
        ]
        .map(|selector| cx.debug_bounds(selector).expect(selector));
        for (index, bounds) in engine_bounds.iter().enumerate() {
            assert!(bounds.left() >= body.left() && bounds.right() <= body.right());
            assert!(bounds.top() >= body.top() && bounds.bottom() <= body.bottom());
            for previous in &engine_bounds[..index] {
                assert!(
                    bounds.left() >= previous.right()
                        || bounds.right() <= previous.left()
                        || bounds.top() >= previous.bottom()
                        || bounds.bottom() <= previous.top(),
                    "engine buttons must not overlap in either grid layout"
                );
            }
        }
        for (selector, provider) in [
            ("settings-engine-1", Provider::Codex),
            ("settings-engine-3", Provider::Qoder),
            ("settings-engine-2", Provider::OpenCode),
            ("settings-nav-8", Provider::OpenCode),
            ("settings-engine-0", Provider::Claude),
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            cx.update(|_, cx| assert_eq!(app.read(cx).connection_settings.engine, provider));
            if selector == "settings-nav-8" {
                cx.update(|_, cx| assert_eq!(app.read(cx).settings_ui.page, 8));
                assert!(cx.debug_bounds("settings-add-service").is_none());
            }
        }
        let add = cx
            .debug_bounds("settings-add-service")
            .expect("Claude service action");
        cx.simulate_mouse_down(add.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(add.center(), MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("provider-editor-body").is_none(),
            "read-only mode does not open an editor"
        );
        cx.update(|window, cx| {
            assert!(app.read(cx).store.providers.is_empty());
            assert_eq!(
                app.read(cx).connection_settings.engine,
                crate::domain::Provider::Claude
            );
            assert!(app.read(cx).settings_ui.open);
            app.update(cx, |app, cx| app.close_settings(window, cx));
        });
    }

    for provider in [Provider::Claude, Provider::Codex] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.writable = true;
                app.connection_settings.engine = provider;
                app.settings_ui.page = 8;
                app.open_settings(window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let add = cx.debug_bounds("settings-add-service").unwrap();
        cx.simulate_mouse_down(add.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(add.center(), MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(
            cx.debug_bounds("provider-editor-body").is_some(),
            "adding a {provider:?} service must open the editor without a cyclic app read"
        );
        for expanded in [false, true] {
            let toggle = cx.debug_bounds("provider-advanced-toggle").unwrap();
            cx.simulate_mouse_down(toggle.center(), MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(toggle.center(), MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            assert_eq!(
                cx.debug_bounds("provider-advanced-fields").is_some(),
                expanded
            );
        }
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(app.read(cx).store.providers.is_empty());
            app.update(cx, |app, cx| app.close_settings(window, cx));
        });
        assert!(cx.debug_bounds("provider-editor-body").is_none());
    }
}

#[gpui_kit::test]
fn opencode_new_session_keeps_permission_selected_before_sending(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::{PermissionLevel, Provider, Workspace},
        store::Store,
    };
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.selected_session = None;
            app.new_session_provider = Provider::OpenCode;
            app.set_session_permission(PermissionLevel::FullAccess, window, cx);
            let id = app.create_session(0, Provider::OpenCode, cx).unwrap();
            assert_eq!(
                app.session(id).unwrap().permission,
                PermissionLevel::FullAccess
            );
            app.new_session_provider = Provider::Claude;
            app.set_session_permission(PermissionLevel::ReadOnly, window, cx);
            assert_eq!(
                app.session(id).unwrap().permission,
                PermissionLevel::FullAccess
            );
            assert_eq!(app.pending_permission, PermissionLevel::ReadOnly);
            app
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn model_picker_switches_engine_and_applies_the_full_route_to_a_new_session(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::{Provider, Workspace},
        providers::{ModelChannel, ModelProvider, ProviderSource, ReasoningEffort},
        store::Store,
    };
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.native_id = Some("existing-claude-context".into());
            session.model = Some("claude-sonnet-4-6".into());
            session.reasoning_effort = ReasoningEffort::High;
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir().join("conduit-engine-route-ui"),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.new_session_provider = Provider::Claude;
            app.store.providers = vec![ModelProvider {
                id: "codex-fixture".into(),
                name: "Codex proxy".into(),
                engine: Provider::Codex,
                base_url: "https://fixture.invalid/v1".into(),
                api_key_env: String::new(),
                protected_api_key: None,
                models: vec!["step-3.5-flash".into(), "gpt-5".into()],
                source: ProviderSource::Local,
                model_metadata: vec![],
                claude_models: None,
            }];
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    macro_rules! click {
        ($selector:expr) => {{
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let point = cx.debug_bounds($selector).expect($selector).center();
            cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }};
    }
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-engine-control").is_none());
    click!("composer-model-control");
    click!("session-engine-control");
    click!("session-engine-option-1");
    assert!(
        cx.debug_bounds("session-channel-control").is_some(),
        "outer model panel stays open"
    );
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.selected_session,
            Some(1),
            "history stays selected until sending"
        );
        assert_eq!(app.send_target(true), None);
        let selection = app.composer_selection(1);
        assert_eq!(selection.engine, Provider::Codex);
        assert_eq!(selection.channel, ModelChannel::Cli);
        assert_eq!(selection.model, None);
        assert_eq!(selection.effort, ReasoningEffort::Medium);
    });
    click!("session-channel-control");
    click!("session-provider-option-1");
    click!("session-model-option-2");
    let slider = cx.debug_bounds("session-effort-control").unwrap();
    let point = point(slider.left() + slider.size.width * 0.75, slider.center().y);
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let selection = app.composer_selection(1);
            assert_eq!(
                selection.channel,
                ModelChannel::Provider("codex-fixture".into())
            );
            assert_eq!(selection.model.as_deref(), Some("gpt-5"));
            assert_eq!(selection.effort, ReasoningEffort::High);
            let id = app.create_session(0, Provider::Codex, cx).unwrap();
            let created = app.session(id).unwrap();
            assert_eq!(created.channel, selection.channel);
            assert_eq!(created.model, selection.model);
            assert_eq!(created.reasoning_effort, selection.effort);
            assert!(created.native_id.is_none());
            assert!(app.pending_model_selection.is_none());
            let old = app.session(1).unwrap();
            assert_eq!(old.provider, Provider::Claude);
            assert_eq!(old.model.as_deref(), Some("claude-sonnet-4-6"));
            assert_eq!(old.reasoning_effort, ReasoningEffort::High);
            app.writable = false;
        })
    });
}

#[gpui_kit::test]
fn opencode_model_picker_uses_session_catalog_and_saves_native_effort(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::{Provider, Workspace},
        store::Store,
    };
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.provider = Provider::OpenCode;
            session.opencode.options = serde_json::json!([
                {"id":"model","options":[
                    {"name":"Test provider","options":[{"value":"test/native","name":"Native model"},{"value":"test/next","name":"Next model"}]},
                    {"value":"other/gpt-5","name":"Other provider model"}
                ]},
                {"id":"effort","options":[{"value":"minimal","name":"Minimal"},{"value":"max","name":"Max"},{"value":"default","name":"Default"}]}
            ]);
            let store = Store { workspaces:vec![Workspace {path:std::env::temp_dir().join("conduit-opencode-picker-ui"),sessions:vec![session]}], ..Default::default() };
            let mut app = super::ConduitApp::with_store(window,cx,Ok(store),false,false);
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.new_session_provider = Provider::OpenCode;
            app.writable = true;
            app.opencode_catalogs.insert(
                app.selected_workspace_path().unwrap(),
                crate::models::ModelCatalog { models: app.session(1).unwrap().opencode.models(), ..Default::default() },
            );
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app,window,cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let trigger = cx
        .debug_bounds("composer-model-control")
        .expect("OpenCode model trigger")
        .center();
    cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("session-effort-control").is_none(),
        "fixed Codex/Claude effort slider must not be shown"
    );
    let choice = cx
        .debug_bounds("opencode-effort-2")
        .expect("native max variant")
        .center();
    cx.simulate_mouse_down(choice, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(choice, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let app = app.read(cx);
        let session = app.session(1).unwrap();
        assert_eq!(session.opencode.variant.as_deref(), Some("max"));
        assert_eq!(
            app.session_route(1).unwrap().acp_variant.as_deref(),
            Some("max")
        );
        let restored: crate::domain::SessionRecord =
            serde_json::from_value(serde_json::to_value(session).unwrap()).unwrap();
        assert_eq!(restored.opencode.variant.as_deref(), Some("max"));
        assert_eq!(restored.opencode.models()[0].id, "test/native");
    });
    macro_rules! click {
        ($selector:expr) => {{
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let point = cx.debug_bounds($selector).expect($selector).center();
            cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }};
    }
    click!("session-channel-control");
    click!("session-provider-option-2"); // CLI default, other, test
    assert!(cx.debug_bounds("session-model-option-1").is_some());
    assert!(
        cx.debug_bounds("session-model-option-2").is_none(),
        "only test's two models are shown"
    );
    click!("session-model-option-1");
    cx.update(|_, cx| {
        let app = app.read(cx);
        let session = app.session(1).unwrap();
        assert_eq!(session.channel, super::ModelChannel::Cli);
        assert_eq!(session.model.as_deref(), Some("test/next"));
        assert_eq!(
            session.opencode.variant, None,
            "old model's variant is cleared"
        );
        assert!(
            session.opencode.variants().is_empty(),
            "old model's effort catalog is invalidated"
        );
        assert_eq!(
            session.opencode.models().len(),
            3,
            "full native model catalog survives filtering"
        );
    });
    click!("session-channel-control");
    click!("session-provider-option-1");
    assert!(cx.debug_bounds("session-model-option-1").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            assert_eq!(
                app.session(1).unwrap().model.as_deref(),
                Some("other/gpt-5")
            );
            app.writable = false;
        })
    });
}

#[gpui_kit::test]
fn concurrent_sessions_isolate_events_approvals_cancellation_and_history(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::{Provider, SessionRecord, Workspace},
        protocol::{Approval, ApprovalKind, Event},
        store::Store,
    };
    use gpui_kit::AppContext as _;

    let session = |id, provider, native: &str| SessionRecord {
        branching: Default::default(),
        transcript: Default::default(),
        id,
        provider,
        native_id: Some(native.into()),
        title: format!("Chat {id}"),
        created_at: 0,
        exit_code: None,
        archived: false,
        pinned: false,
        model: None,
        messages: vec![],
        interrupted: true,
        channel: Default::default(),
        reasoning_effort: Default::default(),
        opencode: Default::default(),
        permission: Default::default(),
        usage: Default::default(),
    };
    let root = std::env::temp_dir().join("conduit-concurrent-ui");
    let other = root.join("other");
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            app.store = Store {
                workspaces: vec![
                    Workspace {
                        path: root.clone(),
                        sessions: vec![
                            session(1, Provider::Claude, "first"),
                            session(2, Provider::Codex, "second"),
                            session(4, Provider::Claude, "first"),
                        ],
                    },
                    Workspace {
                        path: other.clone(),
                        sessions: vec![session(3, Provider::Claude, "third")],
                    },
                ],
                ..Default::default()
            };
            // Event routing is read-only in this test; no real state or CLI calls.
            app.writable = false;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.new_session_provider = Provider::Claude;
            app.session_control.runs.clear();
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            let mut senders = std::collections::HashMap::new();
            for (id, path) in [(1, &root), (2, &root), (3, &other)] {
                this.selected_workspace = Some(if id == 3 { 1 } else { 0 });
                this.selected_session = Some(id);
                this.new_session_provider = this.session(id).unwrap().provider;
                assert!(
                    this.send_blocked(true).is_none(),
                    "other sessions cannot block {id}"
                );
                let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
                this.session_control.runs.insert(
                    id,
                    super::ActiveRun::new(
                        runtime,
                        path.clone(),
                        "turn-0".into(),
                        Default::default(),
                        None,
                        Default::default(),
                    ),
                );
                senders.insert(id, sender);
            }
            assert_eq!(this.session_control.runs.len(), 3);
            assert!(this.worktree_has_runs(&root));
            assert!(this.worktree_has_runs(&other));
            assert!(!this.worktree_has_runs(&root.join("unrelated")));
            assert!(
                this.session_busy(4),
                "aliases cannot resume the same native context twice"
            );
            this.selected_workspace = Some(0);
            this.selected_session = Some(1);
            this.new_session_provider = Provider::Claude;
            this.writable = true;
            assert!(!this.send_text("duplicate".into(), true, window, cx));
            this.writable = false;
            assert!(this.session(1).unwrap().messages.is_empty());
            assert_eq!(
                this.session_control.runs.len(),
                3,
                "duplicate sends must not replace a runtime"
            );
            this.delete_session(1, cx);
            assert!(
                this.session(1).is_some(),
                "active sessions remain addressable until cleanup"
            );
            this.new_session_provider = Provider::Codex;
            assert_eq!(this.send_target(true), None);
            assert!(
                this.send_blocked(true).is_none(),
                "engine switch targets a new conversation"
            );
            assert!(
                this.send_blocked(false).is_some(),
                "regeneration keeps its current conversation"
            );
            this.new_session_provider = Provider::Claude;

            for id in [1, 2, 3] {
                senders[&id]
                    .send(Event::Delta {
                        key: "reply".into(),
                        text: format!("reply-{id}"),
                    })
                    .unwrap();
                senders[&id]
                    .send(Event::Approval(Approval {
                        id: "approval-1".into(),
                        title: format!("request-{id}"),
                        detail: "{}".into(),
                        wire_id: serde_json::json!(1),
                        input: serde_json::Value::Null,
                        allow_once: true,
                        kind: ApprovalKind::Command,
                    }))
                    .unwrap();
            }
            this.poll(window, cx);
            for id in [1, 2, 3] {
                assert_eq!(
                    this.session(id).unwrap().messages[0].text,
                    format!("reply-{id}")
                );
                assert_eq!(
                    this.session_control.runs[&id]
                        .approvals
                        .front()
                        .unwrap()
                        .title,
                    format!("request-{id}")
                );
                this.toggle_approval_detail(id, "approval-1".into(), cx);
            }
            senders[&1]
                .send(Event::ApprovalResolved {
                    id: "approval-1".into(),
                    allow: true,
                })
                .unwrap();
            this.poll(window, cx);
            assert!(this.session_control.runs[&1].expanded_approvals.is_empty());
            for id in [2, 3] {
                assert!(
                    this.session_control.runs[&id]
                        .expanded_approvals
                        .contains("approval-1")
                );
                assert_eq!(this.session_control.runs[&id].approvals.len(), 1);
            }
            this.finish_history_import(
                &root,
                vec![crate::history::ImportedSession {
                    provider: Provider::Claude,
                    native_id: "first".into(),
                    created_at: 0,
                    title: "stale history".into(),
                    model: None,
                    messages: vec![],
                }],
                true,
                true,
                window,
                cx,
            );
            assert_eq!(this.session(1).unwrap().messages[0].text, "reply-1");
            this.session_control.runs.get_mut(&1).unwrap().cancel();
            assert!(this.session_control.runs[&1].runtime.is_cancelled());
            for id in [2, 3] {
                assert!(!this.session_control.runs[&id].runtime.is_cancelled());
            }
            senders[&1]
                .send(Event::Finished {
                    error: None,
                    cancelled: true,
                })
                .unwrap();
            senders[&2]
                .send(Event::Delta {
                    key: "reply".into(),
                    text: "-continued".into(),
                })
                .unwrap();
            this.poll(window, cx);
            assert!(!this.session_control.runs.contains_key(&1));
            assert!(
                !this.session_control.unread_completed.contains(&1),
                "cancellation is not a completed reply"
            );
            assert_eq!(this.session_control.runs.len(), 2);
            assert_eq!(
                this.session(2).unwrap().messages[0].text,
                "reply-2-continued"
            );
            assert!(!this.session(1).unwrap().interrupted);
            assert!(this.session(2).unwrap().interrupted);
            assert!(
                this.send_blocked(true).is_none(),
                "finished session can send while its neighbour runs"
            );
            assert!(
                this.workspace_busy(0),
                "workspace removal still waits for every task"
            );
            assert!(this.workspace_busy(1));
            senders[&2]
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            this.poll(window, cx);
            assert!(
                this.session_control.unread_completed.contains(&2),
                "successful background replies become unread"
            );
            this.switch(0, Some(2), window, cx);
            assert!(
                !this.session_control.unread_completed.contains(&2),
                "opening a session acknowledges its reply"
            );
            senders[&3]
                .send(Event::Finished {
                    error: Some("network timeout\nraw details".into()),
                    cancelled: false,
                })
                .unwrap();
            this.poll(window, cx);
            assert!(
                !this.session_control.unread_completed.contains(&3),
                "failures are not marked as successful replies"
            );
            assert_eq!(
                this.session(3).unwrap().messages.last().unwrap().detail,
                "network timeout\nraw details"
            );
            this.selected_session = Some(1);
            this.selected_workspace = Some(0);
            this.git.operation = Some(super::GitOperation {
                probe: crate::git::OperationProbe::test_pending(root.clone()),
                worktree: root.clone(),
                draft: String::new(),
                commit_push: None,
                automatic_stage: false,
            });
            assert!(this.send_blocked(true).unwrap().contains("Git 操作"));
            this.selected_workspace = Some(1);
            this.selected_session = None;
            assert!(
                this.send_blocked(true).is_none(),
                "Git writes cannot block another workspace"
            );
            this.session_control.runs.clear();
            assert!(
                !this.workspace_busy(1),
                "unrelated Git writes cannot block workspace removal"
            );
            assert!(this.workspace_busy(0));
            this.git.operation = None;
        });
    });
}

/// 资源管理器把文件拖到窗口上时，gpui 会发出一个载荷为 `ExternalPaths` 的拖拽；
/// 这里从平台事件一路打到输入框，验证 composer 的 drop 接线确实生效。
#[gpui_kit::test]
fn platform_file_drop_becomes_a_reference_in_the_composer(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Workspace, store::Store};
    use gpui_kit::{AppContext as _, ExternalPaths, FileDropEvent};

    let workspace = std::env::temp_dir();
    let dropped = workspace.join(format!("conduit-drop-{}.rs", std::process::id()));
    std::fs::write(&dropped, b"fn dropped() {}\n").unwrap();
    let mention = dropped.file_name().unwrap().to_string_lossy().to_string();

    cx.update(gpui_kit::init);
    let app_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = app_slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            app.store = Store {
                workspaces: vec![Workspace {
                    path: workspace.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            app.selected_workspace = Some(0);
            app.writable = true;
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = app_slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let mut paths = ExternalPaths::default();
    paths.0.push(dropped.clone());
    let pending_prompt = |cx: &mut gpui_kit::VisualTestContext| {
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(
                app.input.read(cx).value().is_empty(),
                "adding files must leave prose untouched"
            );
            file_ref::compose_prompt(
                "",
                app.current_files()
                    .map(|files| files.references.as_slice())
                    .unwrap_or_default(),
            )
        })
    };
    // 输入区贴在对话列底部，从下往上找它的命中区域，避免写死窗口坐标。
    let height = cx.update(|window, _| f32::from(window.viewport_size().height));
    let width = cx.update(|window, _| f32::from(window.viewport_size().width));
    let mut added = false;
    for offset in [30., 70., 110., 150.] {
        let position = point(px(width / 2.), px(height - offset));
        cx.simulate_event(FileDropEvent::Entered {
            position,
            paths: paths.clone(),
        });
        cx.simulate_event(FileDropEvent::Submit { position });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        if pending_prompt(cx).contains(&mention) {
            added = true;
            break;
        }
    }
    assert!(
        added,
        "a drop inside the composer must add a file reference"
    );
    let text = pending_prompt(cx);
    assert!(
        text.contains("引用文件："),
        "the reference block must be labelled: {text}"
    );
    assert!(
        !text.contains(&workspace.display().to_string()),
        "the mention must stay workspace-relative: {text}"
    );
    let _ = std::fs::remove_file(&dropped);
}

/// 工作区之外的文件也必须能引用：既已证明拖拽会走到 attach_paths，这里验证它的
/// 引用形式——相对化无从谈起，就用绝对路径，两类路径可以同时挂进输入框。
#[gpui_kit::test]
fn attaching_a_file_outside_the_workspace_uses_an_absolute_mention(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Workspace, store::Store};
    use gpui_kit::AppContext as _;

    let workspace =
        std::env::temp_dir().join(format!("conduit-drop-workspace-{}", std::process::id()));
    std::fs::create_dir_all(&workspace).unwrap();
    let outside =
        std::env::temp_dir().join(format!("conduit-drop-outside-{}.rs", std::process::id()));
    std::fs::write(&outside, b"fn outside() {}\n").unwrap();
    let inside = workspace.join(format!("conduit-drop-inside-{}.rs", std::process::id()));
    std::fs::write(&inside, b"fn inside() {}\n").unwrap();

    cx.update(gpui_kit::init);
    let app_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = app_slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            app.store = Store {
                workspaces: vec![Workspace {
                    path: workspace.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            app.selected_workspace = Some(0);
            app.writable = true;
            app
        });
        *slot.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = app_slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();

    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.attach_paths(&[outside.clone(), inside.clone()], window, cx);
        });
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(
            app.message.is_none(),
            "both paths are usable, so nothing may be skipped: {:?}",
            app.message
        );
        assert!(app.input.read(cx).value().is_empty());
        let files = app
            .current_files()
            .expect("files must be staged separately");
        assert!(files.feedback.is_none());
        let text = file_ref::compose_prompt("", &files.references);
        let name = |path: &std::path::Path| path.file_name().unwrap().to_string_lossy().to_string();
        let outside_mention = text
            .lines()
            .find(|line| line.contains(&name(&outside)))
            .expect("the outside file must appear as a reference");
        let outside_mention = outside_mention
            .strip_prefix("- ")
            .expect("references are listed one per line")
            .to_string();
        assert!(
            std::path::Path::new(&outside_mention).is_absolute(),
            "an outside reference has to stay resolvable: {outside_mention}"
        );
        assert!(
            text.contains(&format!("- {}", name(&inside))),
            "the inside reference stays relative: {text}"
        );
    });
    let _ = std::fs::remove_file(&outside);
    let _ = std::fs::remove_file(&inside);
    let _ = std::fs::remove_dir(&workspace);
}
#[gpui_kit::test]
fn composer_files_paste_drafts_removal_and_failed_send(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    use crate::{
        domain::{Provider, SessionRecord, Workspace},
        store::Store,
    };
    use gpui_kit::{AppContext as _, ClipboardEntry, ClipboardItem, ExternalPaths, Focusable};

    let root = std::env::temp_dir().join(format!("conduit-composer-files-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let first = root.join("first.rs");
    let second = root.join("second.rs");
    std::fs::write(&first, b"fn first() {}").unwrap();
    std::fs::write(&second, b"fn second() {}").unwrap();
    let session = |id| SessionRecord {
        branching: Default::default(),
        transcript: Default::default(),
        id,
        provider: Provider::Claude,
        created_at: 0,
        exit_code: None,
        title: format!("Chat {id}"),
        archived: false,
        pinned: false,
        native_id: None,
        model: None,
        messages: vec![],
        interrupted: false,
        channel: Default::default(),
        reasoning_effort: Default::default(),
        opencode: Default::default(),
        permission: Default::default(),
        usage: Default::default(),
    };
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            app.store = Store {
                workspaces: vec![Workspace {
                    path: root.clone(),
                    sessions: vec![session(1), session(2)],
                }],
                ..Default::default()
            };
            app.store.ui.last_provider = Some(Provider::Claude);
            app.selected_workspace = Some(0);
            app.selected_session = None;
            app.writable = true;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_value("保留正文", window, cx));
            this.attach_paths(&[first.clone(), root.join("missing.rs")], window, cx);
            let files = this.current_files().unwrap();
            assert_eq!(files.references.len(), 1);
            assert!(files.feedback.as_ref().unwrap().contains("missing.rs"));
            assert_eq!(this.input.read(cx).value().as_ref(), "保留正文");
            cx.write_to_clipboard(ClipboardItem {
                entries: vec![ClipboardEntry::ExternalPaths(ExternalPaths(
                    [first.clone(), second.clone()].into_iter().collect(),
                ))],
            });
        })
    });
    cx.simulate_keystrokes("ctrl-v");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert_eq!(
                this.current_files().unwrap().references.len(),
                2,
                "paste must merge files without duplicates"
            );
            assert_eq!(this.input.read(cx).value().as_ref(), "保留正文");
            this.attach_paths(std::slice::from_ref(&first), window, cx);
            assert!(
                this.current_files()
                    .unwrap()
                    .feedback
                    .as_ref()
                    .unwrap()
                    .contains("无需重复")
            );
            this.switch(0, Some(1), window, cx);
            assert!(this.current_files().is_none());
            this.attach_paths(std::slice::from_ref(&second), window, cx);
            this.switch(0, Some(2), window, cx);
            assert!(this.current_files().is_none());
            this.switch(0, Some(1), window, cx);
            let path = this.current_files().unwrap().references[0].path.clone();
            this.remove_file(&path, window, cx);
            assert!(this.current_files().unwrap().references.is_empty());
            this.switch(0, None, window, cx);
            assert_eq!(this.current_files().unwrap().references.len(), 2);
            assert_eq!(this.input.read(cx).value().as_ref(), "保留正文");
            std::fs::remove_file(&first).unwrap();
            this.send_prompt(window, cx);
            assert_eq!(this.selected_session, None);
            assert!(
                this.session_control.runs.is_empty(),
                "invalid files must not start a CLI"
            );
            assert_eq!(
                this.current_files().unwrap().references.len(),
                2,
                "failed send must preserve files"
            );
            assert!(
                this.current_files()
                    .unwrap()
                    .feedback
                    .as_ref()
                    .unwrap()
                    .contains("first.rs")
            );
            assert_eq!(this.input.read(cx).value().as_ref(), "保留正文");
            // Read-only input must reject drops without changing its draft.
            this.writable = false;
            this.attach_paths(&[root.join("readonly.rs")], window, cx);
            assert_eq!(this.current_files().unwrap().references.len(), 2);
            this.writable = true;
            this.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.write_to_clipboard(ClipboardItem::new_string("纯文本".into()));
        })
    });
    cx.simulate_keystrokes("ctrl-v");
    cx.update(|_, cx| {
        assert!(
            app.read(cx).input.read(cx).value().contains("纯文本"),
            "ordinary text paste must keep working"
        );
    });
    let image = gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Png,
        include_bytes!("../../assets/app-icons/Square44x44Logo.png").to_vec(),
    );
    cx.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![
                ClipboardEntry::Image(image.clone()),
                ClipboardEntry::String(gpui_kit::ClipboardString::new("图片的替代文本".into())),
            ],
        })
    });
    cx.simulate_keystrokes("ctrl-v");
    let pasted = cx.update(|_, cx| {
        let this = app.read(cx);
        let files = &this.current_files().unwrap().references;
        assert_eq!(files.len(), 3);
        let path = files.last().unwrap().path.clone();
        assert_eq!(std::fs::read(&path).unwrap(), image.bytes);
        assert!(!this.input.read(cx).value().contains("图片的替代文本"));
        path
    });
    cx.simulate_keystrokes("ctrl-v");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            assert_eq!(this.current_files().unwrap().references.len(), 3);
            this.switch(0, Some(2), window, cx);
            assert!(this.current_files().is_none());
            this.writable = false;
            assert!(this.paste_attachments(&ClipboardItem::new_image(&image), window, cx));
            assert!(this.current_files().is_none());
            this.writable = true;
            this.switch(0, None, window, cx);
            assert_eq!(this.current_files().unwrap().references.len(), 3);
            // A clipboard containing both paths and a thumbnail adds only paths.
            this.remove_file(&pasted, window, cx);
            this.persistence.flush();
            this.poll_persistence(cx);
            assert!(this.paste_attachments(
                &ClipboardItem {
                    entries: vec![
                        ClipboardEntry::ExternalPaths(ExternalPaths(
                            [second.clone()].into_iter().collect()
                        )),
                        ClipboardEntry::Image(image.clone()),
                    ],
                },
                window,
                cx
            ));
            assert_eq!(this.current_files().unwrap().references.len(), 2);
            assert!(this.paste_attachments(
                &ClipboardItem::new_image(&gpui_kit::Image::empty()),
                window,
                cx
            ));
            assert!(
                this.current_files()
                    .unwrap()
                    .feedback
                    .as_ref()
                    .unwrap()
                    .contains("图片为空")
            );
        });
    });
    assert!(
        !pasted.exists(),
        "removing the last draft reference must delete its cached image"
    );
    std::fs::remove_file(second).unwrap();
    let _ = std::fs::remove_dir(root);
}
#[gpui_kit::test]
fn clipboard_images_are_collected_after_the_last_draft_or_history_reference(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::{domain::Workspace, store::Store};
    use gpui_kit::{AppContext as _, ClipboardItem};
    use std::os::windows::fs::OpenOptionsExt as _;

    let root = std::env::temp_dir().join(format!("conduit-image-cleanup-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let image = gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Png,
        include_bytes!("../../assets/app-icons/Square44x44Logo.png").to_vec(),
    );
    let directory = Store::path().with_file_name("attachments");
    let pasted = file_ref::save_clipboard_image(&image, &directory).unwrap();
    let orphan = file_ref::save_clipboard_image(
        &gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Bmp, b"BM\0orphan".to_vec()),
        &directory,
    )
    .unwrap();
    let original = root.join("original.png");
    std::fs::write(&original, &image.bytes).unwrap();
    let user: ChatItem = serde_json::from_value(serde_json::json!({
        "key": "user-image", "kind": "User", "text": file_ref::compose_prompt("查看图片", &[
            file_ref::FileReference { path: pasted.clone(), mention: file_ref::display_path(&pasted).replace('\\', "/") }
        ])
    })).unwrap();
    let session = |id, messages| {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": "Claude", "created_at": 0, "exit_code": null,
            "title": "Image cleanup", "messages": messages
        }))
        .unwrap()
    };
    let store = Store {
        workspaces: vec![Workspace {
            path: root.clone(),
            sessions: vec![session(1, vec![user.clone()]), session(2, vec![])],
        }],
        ..Default::default()
    };
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let app = cx.new(|cx| super::ConduitApp::with_store(window, cx, Ok(store), false, false));
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    assert!(
        !orphan.exists(),
        "startup must collect leftovers from a discarded draft"
    );
    assert!(pasted.exists(), "startup must preserve sent images");
    cx.update(|window, cx| {
        app.update(cx, |this, cx| {
            this.switch(0, Some(2), window, cx);
            this.paste_attachments(&ClipboardItem::new_image(&image), window, cx);
            this.remove_file(&pasted.canonicalize().unwrap(), window, cx);
            assert!(
                pasted.exists(),
                "removing a draft must preserve another session's history image"
            );
            this.paste_attachments(&ClipboardItem::new_image(&image), window, cx);
            this.delete_session(1, cx);
            assert!(
                pasted.exists(),
                "a second session's unsent draft still owns the image"
            );

            this.session_mut(2).unwrap().messages.push(user.clone());
            this.remove_file(&pasted.canonicalize().unwrap(), window, cx);
            assert!(
                pasted.exists(),
                "sending transfers ownership from the draft to history"
            );
            assert!(this.persist());
            this.persistence.flush();
            let locked = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(Store::path())
                .unwrap();
            this.delete_session(2, cx);
            this.persistence.flush();
            assert!(
                pasted.exists(),
                "failed state writes must not delete an image referenced on disk"
            );
            drop(locked);
            this.persist_and_cleanup_attachments();
            this.persistence.flush();
            this.poll_persistence(cx);
            assert!(
                !pasted.exists(),
                "deleting the last saved reference must collect the image"
            );

            this.switch(0, None, window, cx);
            this.paste_attachments(&ClipboardItem::new_image(&image), window, cx);
            assert!(pasted.exists());
            this.discard_composer_attachments();
            assert!(
                !pasted.exists(),
                "closing must collect unsent workspace drafts"
            );

            this.paste_attachments(&ClipboardItem::new_image(&image), window, cx);
            this.attach_paths(std::slice::from_ref(&original), window, cx);
            this.remove_workspace(0, cx);
            this.persistence.flush();
            this.poll_persistence(cx);
            assert!(
                !pasted.exists(),
                "removing a workspace must collect its last draft image"
            );
            assert!(
                original.exists(),
                "user-selected originals must never be deleted"
            );

            file_ref::save_clipboard_image(&image, &directory).unwrap();
            this.writable = false;
            this.cleanup_attachments();
            assert!(
                pasted.exists(),
                "read-only or corrupt state must disable collection"
            );
            this.writable = true;
            this.cleanup_attachments();
            assert!(!pasted.exists());
        })
    });
    std::fs::remove_file(original).unwrap();
    let _ = std::fs::remove_dir(root);
}

#[gpui_kit::test]
fn commit_generation_recovers_from_transient_errors_and_preserves_edited_drafts(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::protocol::Event;
    use gpui_kit::AppContext as _;
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = crate::store::Store {
                workspaces: vec![crate::domain::Workspace {
                    path: std::env::temp_dir().join("conduit-commit-generation-ui"),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.selected_workspace = Some(0);
            app.ensure_commit_draft(window, cx);
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    for (reply, diagnostic, edited) in [
        (
            "```text\nfeat: staged changes\n\nDetailed body\n```",
            Some("503 unavailable"),
            false,
        ),
        ("", Some("401 unauthorized"), false),
        ("feat: do not overwrite", None, true),
    ] {
        let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                let path = this.selected_workspace_path().unwrap();
                let original = this.git.drafts[&path].input.read(cx).value().to_string();
                this.git.generation = Some(super::CommitGeneration {
                    cancelled: false,
                    path: path.clone(),
                    runtime,
                    started: std::time::Instant::now(),
                    original,
                    parts: vec![],
                    error: None,
                    last_diagnostic: None,
                    cleanup_error: None,
                });
                if edited {
                    this.git.drafts[&path].input.update(cx, |input, cx| {
                        input.set_value("manual message", window, cx)
                    });
                }
            })
        });
        if let Some(error) = diagnostic {
            sender.send(Event::ErrorBanner(error.into())).unwrap();
        }
        sender
            .send(Event::Text {
                key: "reply".into(),
                text: reply.into(),
            })
            .unwrap();
        sender
            .send(Event::Finished {
                error: None,
                cancelled: false,
            })
            .unwrap();
        cx.update(|window, cx| app.update(cx, |this, cx| this.poll_git_actions(window, cx)));
        cx.update(|_, cx| {
            let this = app.read(cx);
            let path = this.selected_workspace_path().unwrap();
            assert!(this.git.generation.is_none());
            let text = this.git.drafts[&path].input.read(cx).value().to_string();
            let feedback = this
                .notifications
                .center
                .records()
                .iter()
                .find(|notice| notice.source == NoticeSource::Git)
                .unwrap();
            if edited {
                assert_eq!(text, "manual message");
                assert!(feedback.message.contains("未覆盖"));
            } else if reply.is_empty() {
                assert_eq!(feedback.level, NoticeLevel::Error);
                assert!(feedback.message.contains("401"));
            } else {
                assert_eq!(text, "feat: staged changes\n\nDetailed body");
                assert_eq!(feedback.level, NoticeLevel::Success);
            }
        });
    }
}

mod history_and_search {
    //! History import, outline invalidation, and search freshness; no real CLI is started.
    use super::super::{ChatItem, ConduitApp, Provider, Store, Workspace};
    use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use std::path::PathBuf;

    fn fixture(cx: &mut TestAppContext) -> (Entity<ConduitApp>, &mut VisualTestContext, PathBuf) {
        cx.update(gpui_kit::init);
        let path = std::env::temp_dir().canonicalize().unwrap();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut session = super::super::tests::session();
                session.native_id = Some("review-native".into());
                session.title = "手动命名".into();
                let store = Store {
                    version: 2,
                    workspaces: vec![Workspace {
                        path: path.clone(),
                        sessions: vec![session],
                    }],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        (app, cx, path)
    }
    fn item(key: &str, text: &str) -> ChatItem {
        serde_json::from_value(serde_json::json!({"key":key,"kind":"User","text":text})).unwrap()
    }
    fn imported(messages: Vec<ChatItem>) -> crate::history::ImportedSession {
        crate::history::ImportedSession {
            provider: Provider::Claude,
            native_id: "review-native".into(),
            created_at: 0,
            title: "原生标题".into(),
            model: None,
            messages,
        }
    }

    #[gpui_kit::test]
    fn stale_history_cannot_replace_messages_after_a_turn_completes(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, path) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.session_mut(1).unwrap().messages = vec![item("user", "first")];
                app.session_control.begin_import(&app.store.workspaces[0]);
                // A started and completed turn changes the revision even though no
                // runtime remains when the scan returns.
                app.session_mut(1)
                    .unwrap()
                    .messages
                    .push(item("new-user", "completed turn"));
                app.finish_history_import(
                    &path,
                    vec![imported(vec![item("import-0", "first")])],
                    false,
                    true,
                    window,
                    cx,
                );
                assert_eq!(app.session(1).unwrap().messages.len(), 2);
                assert_eq!(app.session(1).unwrap().messages[1].text, "completed turn");
                assert_eq!(app.session(1).unwrap().title, "手动命名");
            })
        });
    }

    #[gpui_kit::test]
    fn search_discards_results_after_transcript_changes(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = fixture(cx);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.session_mut(1).unwrap().messages = vec![item("user", "old prompt")];
                app.search_query = "prompt".into();
                app.refresh_search(cx, false);
                app.session_mut(1).unwrap().messages = vec![item("user", "new prompt")];
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                assert_eq!(app.search_ui.hits[0].snippet, "new prompt");
                app.refresh_search(cx, false);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(app.search_ui.hits.len(), 1);
            assert_eq!(app.search_ui.hits[0].snippet, "new prompt");
        });
    }

    #[gpui_kit::test]
    fn history_batches_keep_the_original_revision_until_the_import_finishes(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, path) = fixture(cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.session_mut(1).unwrap().messages = vec![item("user", "first")];
                app.session_control.begin_import(&app.store.workspaces[0]);
                app.history_loading.insert(path.clone());
                let initial = app.session_control.imports[&path].clone();
                app.finish_history_import(
                    &path,
                    vec![imported(vec![item("import-0", "first")])],
                    false,
                    false,
                    window,
                    cx,
                );
                assert!(app.history_loading.contains(&path));
                assert_eq!(app.session_control.imports[&path], initial);
                app.session_mut(1)
                    .unwrap()
                    .messages
                    .push(item("new-user", "completed turn"));
                app.finish_history_import(
                    &path,
                    vec![imported(vec![item("import-0", "first")])],
                    false,
                    true,
                    window,
                    cx,
                );
                assert!(!app.history_loading.contains(&path));
                assert!(!app.session_control.imports.contains_key(&path));
                assert_eq!(app.session(1).unwrap().messages.len(), 2);
                assert_eq!(app.session(1).unwrap().messages[1].text, "completed turn");
            });
        });
    }
}

#[gpui_kit::test]
fn qoder_picker_persists_session_catalog_and_resets_effort_when_model_changes(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    use crate::domain::Provider;
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.provider = Provider::Qoder;
            session.native_id = Some("qoder-context".into());
            session.reasoning_effort = crate::providers::ReasoningEffort::Default;
            session.opencode.options = serde_json::json!([
                {"id":"model","options":[{"value":"model-a","name":"Alpha"},{"value":"model-b","name":"Beta"}]},
                {"id":"reasoning_effort","options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}
            ]);
            let store = crate::store::Store {
                workspaces: vec![crate::domain::Workspace {
                    path: std::env::temp_dir().join("conduit-qoder-picker-ui"), sessions: vec![session]
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window,cx,Ok(store),false,false);
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.new_session_provider = Provider::Qoder;
            app.writable = true;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app,window,cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    macro_rules! click {
        ($selector:expr) => {{
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let point = cx.debug_bounds($selector).expect($selector).center();
            cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }};
    }
    click!("composer-model-control");
    assert!(cx.debug_bounds("session-effort-control").is_none());
    click!("opencode-effort-2");
    cx.update(|_, cx| {
        let app = app.read(cx);
        let session = app.session(1).unwrap();
        assert_eq!(
            app.session_route(1).unwrap().acp_variant.as_deref(),
            Some("high")
        );
        assert!(
            app.channel_route(Provider::Qoder, &super::ModelChannel::Ccs("test".into()))
                .is_err()
        );
        let restored: crate::domain::SessionRecord =
            serde_json::from_value(serde_json::to_value(session).unwrap()).unwrap();
        assert_eq!(restored.provider, Provider::Qoder);
        assert_eq!(restored.native_id.as_deref(), Some("qoder-context"));
        assert_eq!(restored.opencode.variant.as_deref(), Some("high"));
        assert_eq!(restored.opencode.models().len(), 2);
    });
    click!("session-model-option-2");
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let session = app.session(1).unwrap();
            assert_eq!(session.model.as_deref(), Some("model-b"));
            assert!(session.opencode.variant.is_none());
            assert!(session.opencode.variants().is_empty());
            assert_eq!(session.opencode.models().len(), 2);
            app.writable = false;
        });
    });
}

#[gpui_kit::test]
fn cli_retry_progress_keeps_native_details_and_recovers_with_output(cx: &mut TestAppContext) {
    use crate::protocol::{Event, Protocol};
    use gpui_kit::{Modifiers, MouseButton};
    use serde_json::json;
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = super::queue::tests::fixture(cx, "cli-retry-status");
    let sender = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().messages.clear();
            super::queue::tests::running(app, 1)
        })
    });
    let mut protocol = Protocol::new(
        super::Provider::Codex,
        "C:/work".into(),
        "hello".into(),
        None,
    );
    let mut expected_detail = String::new();
    for attempt in 1..=2 {
        let step = protocol.accept(json!({"method":"error", "params":{
            "threadId":"retry-thread", "turnId":"turn",
            "error":{"message":format!("Reconnecting... {attempt}/5: stream disconnected before completion"),
                "codexErrorInfo":{"responseStreamDisconnected":{"httpStatusCode":502}}},
            "willRetry":true
        }}));
        for event in step.events {
            if let Event::ErrorBanner(raw) = &event {
                expected_detail = raw.clone();
            }
            sender.send(event).unwrap();
        }
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    }
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session_control.runs[&1].activity_label(),
            "正在自动重试（2/5）…"
        );
        assert_eq!(app.session(1).unwrap().messages.len(), 1);
        assert_eq!(app.session(1).unwrap().messages[0].detail, expected_detail);
    });
    assert!(cx.debug_bounds("cli-error-0").is_some());
    assert!(cx.debug_bounds("cli-error-details-0").is_none());
    let point = cx.debug_bounds("cli-error-toggle-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("cli-error-details-0").is_some());
    let point = cx.debug_bounds("cli-error-copy-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap()),
        expected_detail
    );
    sender
        .send(Event::Delta {
            key: "reply".into(),
            text: "Recovered response".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).session_control.runs[&1].activity_label(),
            conduit::i18n::text_static("stream_events.generating_response")
        );
    });
    sender
        .send(Event::Finished {
            error: None,
            cancelled: false,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.session_control.runs.contains_key(&1));
        assert_eq!(app.session(1).unwrap().messages.len(), 2);
        assert_eq!(
            app.session(1).unwrap().messages[1].text,
            "Recovered response"
        );
    });
}
