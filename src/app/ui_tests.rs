use super::{ChatItem, ChatTypography, file_ref, tool_content};
use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, StatefulInteractiveElement, Styled, TestAppContext, Window,
    component::{WindowExt, v_flex},
    div, point, px,
};

#[gpui_kit::test]
fn settings_dismisses_on_outside_click_and_keeps_inside_clicks(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            super::ConduitApp::with_store(
                window,
                cx,
                Ok(crate::store::Store::default()),
                false,
                false,
            )
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    for close_with_escape in [false, true] {
        cx.update(|window, cx| app.update(cx, |app, cx| app.open_settings(window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(window.has_active_dialog(cx));
        });
        let surface = cx.debug_bounds("dialog-0").unwrap();
        let inside = point(surface.left() + px(12.), surface.center().y);
        cx.simulate_mouse_down(inside, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(inside, MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| assert!(window.has_active_dialog(cx)));
        if close_with_escape {
            cx.simulate_keystrokes("escape");
        } else {
            let outside = point(px(10.), px(100.));
            cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
        }
        cx.run_until_parked();
        cx.update(|window, cx| assert!(!window.has_active_dialog(cx)));
    }
}

#[gpui_kit::test]
fn opencode_new_session_keeps_permission_selected_before_sending(cx: &mut TestAppContext) {
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
                models: vec!["step-3.5-flash".into(), "gpt-5".into()],
                source: ProviderSource::Local,
                model_metadata: vec![],
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
fn model_picker_is_available_before_the_first_session(cx: &mut TestAppContext) {
    use crate::{domain::Workspace, store::Store};
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            super::ConduitApp::with_store(
                window,
                cx,
                Ok(Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir().join("conduit-first-model-ui"),
                        sessions: vec![],
                    }],
                    ..Default::default()
                }),
                false,
                false,
            )
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let trigger = cx
        .debug_bounds("composer-model-control")
        .expect("first session model selector")
        .center();
    cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("session-engine-control").is_some());
    assert!(cx.debug_bounds("session-channel-control").is_some());
    assert!(cx.debug_bounds("session-model-list").is_some());
    assert!(cx.debug_bounds("session-effort-control").is_some());
}

#[gpui_kit::test]
fn opencode_model_picker_defaults_to_native_catalog_and_never_ccs(cx: &mut TestAppContext) {
    use crate::{
        ccs::{CcsCatalog, CcsProvider},
        domain::{Provider, Workspace},
        models::{ModelCatalog, ModelOption},
        providers::{ModelChannel, ModelProvider, ProviderSource},
        store::Store,
    };
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let cwd = std::env::temp_dir().join("conduit-native-catalog-ui");
            let mut app = super::ConduitApp::with_store(
                window,
                cx,
                Ok(Store {
                    workspaces: vec![Workspace {
                        path: cwd.clone(),
                        sessions: vec![],
                    }],
                    ..Default::default()
                }),
                false,
                false,
            );
            app.new_session_provider = Provider::OpenCode;
            let model = |id: &str| ModelOption {
                id: id.into(),
                label: id.into(),
                note: "native fixture".into(),
                context_window: None,
            };
            app.opencode_catalogs.insert(
                cwd,
                ModelCatalog {
                    models: vec![model("opencode/free"), model("native/glm-5")],
                    fetched_at: Some(std::time::SystemTime::now()),
                    ..Default::default()
                },
            );
            app.ccs = CcsCatalog {
                providers: vec![CcsProvider {
                    provider: Provider::OpenCode,
                    id: "ccs".into(),
                    name: "CCS default".into(),
                    current: true,
                    models: vec![model("ccs-default")],
                    base_url: String::new(),
                }],
                ..Default::default()
            };
            app.store.providers.push(ModelProvider {
                id: "old-ccs".into(),
                name: "CCS default".into(),
                engine: Provider::OpenCode,
                base_url: String::new(),
                api_key_env: String::new(),
                models: vec!["ccs-default".into()],
                source: ProviderSource::Ccs { id: "ccs".into() },
                model_metadata: vec![],
            });
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
    click!("composer-model-control");
    assert!(
        cx.debug_bounds("session-model-option-2").is_some(),
        "all native models visible before first send"
    );
    assert!(cx.debug_bounds("session-model-option-3").is_none());
    click!("session-model-option-2");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let selected = app.composer_selection(0);
            assert_eq!(selected.model.as_deref(), Some("native/glm-5"));
            assert_eq!(selected.channel, ModelChannel::Cli);
            assert!(
                app.opencode_model_probes.is_empty(),
                "cached directory opens without a CLI call"
            );
            let helper = app.git_generation_catalog(Provider::OpenCode);
            assert_eq!(helper.models.len(), 2);
            assert!(!helper.models.iter().any(|m| m.id == "ccs-default"));
            // Switching workspaces cannot expose the previous workspace's catalog
            // or silently fall back to CCS when the CLI query fails.
            let other = std::env::temp_dir().join("conduit-native-other-ui");
            app.store.workspaces.push(Workspace {
                path: other.clone(),
                sessions: vec![],
            });
            app.selected_workspace = Some(1);
            app.opencode_catalogs.insert(
                other,
                ModelCatalog {
                    error: Some("fixture failed".into()),
                    ..Default::default()
                },
            );
            let (error, models) = app.channel_models(Provider::OpenCode, &ModelChannel::Cli);
            assert_eq!(error.as_deref(), Some("fixture failed"));
            assert!(models.is_empty());
            app.selected_workspace = Some(0);
            app.pending_model_selection = None;
            cx.notify();
        })
    });
    click!("session-channel-control");
    assert!(cx.debug_bounds("session-provider-option-2").is_some());
    assert!(
        cx.debug_bounds("session-provider-option-3").is_none(),
        "CCS provider not offered for OpenCode"
    );
    cx.update(|_, cx| app.update(cx, |app, _| app.writable = false));
}

#[gpui_kit::test]
fn opencode_model_picker_uses_session_catalog_and_saves_native_effort(cx: &mut TestAppContext) {
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
            app.session_route(1).unwrap().opencode_variant.as_deref(),
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
fn git_branch_menu_opens_create_form_and_remains_usable_after_repaint(cx: &mut TestAppContext) {
    use crate::{domain::Workspace, git, store::Store};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let path = std::env::temp_dir().join("conduit-branch-menu-fixture");
            let store = Store {
                workspaces: vec![Workspace {
                    path: path.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.selected_workspace = Some(0);
            app.git_workspace = Some(path);
            app.branch_probe = None;
            app.git_probe = None;
            let info = git::BranchInfo {
                name: "main".into(),
                full_ref: "refs/heads/main".into(),
                oid: "0123456789012345678901234567890123456789".into(),
                remote: false,
                upstream: None,
                upstream_remote: None,
                upstream_ref: None,
                occupied: None,
            };
            app.git_repository = Some(git::Repository {
                current: "main".into(),
                detached: false,
                branches: vec![git::Branch {
                    name: "main".into(),
                    remote: false,
                }],
                truncated: false,
                upstream: None,
                ahead: 0,
                context: git::BranchContext {
                    current_ref: Some(info.full_ref.clone()),
                    head: Some(info.oid.clone()),
                },
                branch_info: vec![info],
                integration: None,
                conflicts: vec![],
                remotes: vec![],
            });
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let point = cx
        .debug_bounds("git-branch-trigger")
        .expect("branch trigger")
        .center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let point = cx
        .debug_bounds("git-branch-actions-0")
        .expect("visible action menu")
        .center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    for _ in 0..3 {
        cx.update(|window, cx| {
            slot.borrow()
                .as_ref()
                .unwrap()
                .update(cx, |_, cx| cx.notify());
            let _ = window.draw(cx);
        });
        cx.run_until_parked();
    }
    let item = cx.update(|window, _| {
        window
            .within("popup-menu")
            .find_by_label("New Branch from 'main'")
            .bounds()
            .center()
    });
    cx.simulate_mouse_move(item, None, Modifiers::default());
    cx.simulate_mouse_down(item, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.simulate_mouse_up(item, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("branch-form").is_some(),
        "New Branch opens a form from the nested popup"
    );
    assert!(
        cx.debug_bounds("dialog-0").is_none(),
        "simple branch input is non-modal"
    );
    cx.update(|window, _| {
        assert_eq!(
            window.find_by_label("分支名").focused(),
            Some(true),
            "inline input receives focus immediately"
        )
    });
    let row_bounds = cx.debug_bounds("git-branch-row-0").unwrap();
    let form_bounds = cx.debug_bounds("branch-form").unwrap();
    assert!(
        row_bounds.right() <= form_bounds.left() || form_bounds.right() <= row_bounds.left(),
        "input opens beside the branch list"
    );
    for (width, height) in [(1200., 800.), (850., 500.)] {
        cx.simulate_resize(gpui_kit::size(px(width), px(height)));
        cx.run_until_parked();
        for _ in 0..2 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
        let bounds = cx.debug_bounds("branch-form").unwrap();
        let row = cx.debug_bounds("git-branch-row-0").unwrap();
        assert!(
            bounds.left() >= px(0.)
                && bounds.top() >= px(0.)
                && bounds.right() <= px(width)
                && bounds.bottom() <= px(height),
            "side input stays in {width}×{height}: {bounds:?}"
        );
        assert!(
            row.right() <= bounds.left() || bounds.right() <= row.left(),
            "input flips beside the menu at {width}×{height}"
        );
    }
    let submit = cx.debug_bounds("branch-form-submit").unwrap().center();
    cx.simulate_mouse_down(submit, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(submit, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("branch-form-error").is_some(),
        "empty branch names show inline validation"
    );
    let cancel = cx.debug_bounds("branch-form-cancel").unwrap().center();
    cx.simulate_mouse_down(cancel, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(cancel, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("branch-form").is_none());
    assert!(
        cx.debug_bounds("git-branch-actions-0").is_some(),
        "cancelling inline input preserves the branch list"
    );
    cx.update(|_, cx| {
        assert!(
            slot.borrow()
                .as_ref()
                .unwrap()
                .read(cx)
                .git_operation
                .is_none(),
            "cancelling cannot start a Git mutation"
        );
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    // Both right-click and the explicit button must preserve the menu until mouse-up.
    for (label, dialog) in [
        ("Rename", "branch-form"),
        ("Show Diff with Working Tree", "git-comparison-browser"),
    ] {
        let trigger = cx.debug_bounds("git-branch-trigger").unwrap().center();
        cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let row = cx.debug_bounds("git-branch-row-0").unwrap().center();
        cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let item = cx.update(|window, _| {
            window
                .within("popup-menu")
                .find_by_label(label)
                .bounds()
                .center()
        });
        cx.simulate_mouse_move(item, None, Modifiers::default());
        cx.simulate_mouse_down(item, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_up(item, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(
            cx.debug_bounds(dialog).is_some(),
            "mouse selection opens {label}"
        );
        if label == "Rename" {
            assert!(cx.debug_bounds("git-branch-actions-0").is_some());
            assert!(cx.debug_bounds("dialog-0").is_none());
        } else {
            assert!(cx.debug_bounds("git-branch-actions-0").is_none());
            for (width, height) in [(1200., 800.), (850., 500.)] {
                cx.simulate_resize(gpui_kit::size(px(width), px(height)));
                cx.run_until_parked();
                cx.update(|window, cx| {
                    let _ = window.draw(cx);
                });
                let bounds = cx.debug_bounds("git-comparison-browser").unwrap();
                assert!(
                    (bounds.center().y - px(height * 0.5)).abs() <= px(4.),
                    "Diff dialog must be vertically centered: {bounds:?}"
                );
                assert!(
                    (bounds.center().x - px(width * 0.5)).abs() <= px(4.),
                    "Diff dialog must be horizontally centered: {bounds:?}"
                );
            }
        }
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds(dialog).is_none());
        if label == "Rename" {
            assert!(
                cx.debug_bounds("git-branch-actions-0").is_some(),
                "Escape closes only the input"
            );
            cx.simulate_keystrokes("escape");
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
        }
    }
    // Outside clicks still dismiss an ordinary branch list.
    let trigger = cx.debug_bounds("git-branch-trigger").unwrap().center();
    cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let outside = gpui_kit::point(px(5.), px(5.));
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("git-branch-actions-0").is_none());
    // A long upstream submenu must scroll independently of the branch picker.
    cx.update(|_, cx| {
        slot.borrow().as_ref().unwrap().update(cx, |app, cx| {
            let repository = app.git_repository.as_mut().unwrap();
            let template = repository.branch_info[0].clone();
            for index in 0..80 {
                let name = format!("origin/remote-{index:03}");
                repository.branch_info.push(git::BranchInfo {
                    name: name.clone(),
                    full_ref: format!("refs/remotes/{name}"),
                    remote: true,
                    ..template.clone()
                });
            }
            cx.notify();
        })
    });
    let trigger = cx.debug_bounds("git-branch-trigger").unwrap().center();
    cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let actions = cx.debug_bounds("git-branch-actions-0").unwrap().center();
    cx.simulate_mouse_down(actions, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(actions, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let tracking = cx.update(|window, _| {
        window
            .within("popup-menu")
            .find_by_label("Tracked Branch")
            .bounds()
            .center()
    });
    cx.simulate_mouse_move(tracking, None, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let submenu = cx.update(|window, _| window.find("submenu").bounds());
    assert!(
        submenu.size.height <= px(360.),
        "upstream submenu has a bounded viewport: {submenu:?}"
    );
    let first = cx.update(|window, _| {
        window
            .within("submenu")
            .find_by_label("origin/remote-000")
            .bounds()
    });
    let branch_before = cx.debug_bounds("git-branch-row-0").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: submenu.center(),
        delta: ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-3000.))),
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|window, _| {
        let last = window.within("submenu").find_by_label("origin/remote-079");
        assert!(
            last.visible(),
            "wheel reaches the final upstream branch: {last:?}"
        );
        assert!(
            window
                .within("submenu")
                .find_by_label("origin/remote-000")
                .bounds()
                .top()
                < first.top()
        );
    });
    assert_eq!(
        cx.debug_bounds("git-branch-row-0").unwrap(),
        branch_before,
        "scrolling the submenu leaves the branch list in place"
    );
    cx.simulate_event(ScrollWheelEvent {
        position: submenu.center(),
        delta: ScrollDelta::Pixels(gpui_kit::point(px(0.), px(3000.))),
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|window, _| {
        assert!(
            window
                .within("submenu")
                .find_by_label("origin/remote-000")
                .visible()
        )
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        slot.borrow().as_ref().unwrap().update(cx, |app, cx| {
            let repository = app.git_repository.as_mut().unwrap();
            repository.integration = Some(git::IntegrationState::Merge);
            repository.conflicts = vec!["src/conflicted.rs".into()];
            cx.notify();
        });
        let _ = window.draw(cx);
    });
    let point = cx.debug_bounds("git-branch-trigger").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let point = cx
        .debug_bounds("git-recovery-abort")
        .expect("persistent conflict recovery in the branch popup")
        .center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("branch-form").is_some(),
        "abort opens a reviewable confirmation"
    );
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("branch-form").is_none());
}

#[gpui_kit::test]
fn git_branch_mouse_actions_checkout_create_and_track_real_repository(cx: &mut TestAppContext) {
    use crate::{domain::Workspace, git, store::Store};
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
    use gpui_kit::{AppContext as _, Modifiers, MouseButton, VisualTestContext};
    use std::{
        path::PathBuf,
        sync::atomic::AtomicBool,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    // Only this newly-created repository is mutated by the UI actions below.
    struct Repository(PathBuf);
    impl Drop for Repository {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let repository = Repository(std::env::temp_dir().join(format!(
        "conduit-branch-click-{}-{nonce}",
        std::process::id()
    )));
    std::fs::create_dir_all(repository.0.join("empty-hooks")).unwrap();
    let git_executable = git::resolve_git().expect("Git required for branch UI integration test");
    let run = |args: &[&str]| {
        let output = std::process::Command::new(&git_executable)
            .current_dir(&repository.0)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    run(&["init", "-q", "--initial-branch=main"]);
    run(&["config", "user.name", "Conduit UI Test"]);
    run(&["config", "user.email", "conduit-ui@example.invalid"]);
    run(&["config", "commit.gpgSign", "false"]);
    run(&[
        "config",
        "core.hooksPath",
        repository.0.join("empty-hooks").to_str().unwrap(),
    ]);
    std::fs::write(repository.0.join("tracked.txt"), "seed\n").unwrap();
    run(&["add", "tracked.txt"]);
    run(&["commit", "-qm", "seed"]);
    run(&["branch", "feature"]);
    run(&["remote", "add", "origin", "."]);
    run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let metadata = git::load_repository(&repository.0, &AtomicBool::new(false)).unwrap();
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let store = Store {
                workspaces: vec![Workspace {
                    path: repository.0.clone(),
                    sessions: vec![],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.selected_workspace = Some(0);
            app.git_workspace = Some(repository.0.clone());
            app.branch_probe = None;
            app.git_probe = None;
            app.git_repository = Some(metadata);
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
    fn pointer_click(
        cx: &mut VisualTestContext,
        point: gpui_kit::Point<gpui_kit::Pixels>,
        button: MouseButton,
    ) {
        cx.simulate_mouse_move(point, None, Modifiers::default());
        cx.simulate_mouse_down(point, button, Modifiers::default());
        // A real display may repaint between the press and release.
        draw(cx);
        cx.simulate_mouse_up(point, button, Modifiers::default());
        draw(cx);
    }
    fn wait_operation(cx: &mut VisualTestContext, app: &gpui_kit::Entity<super::ConduitApp>) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let done = cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.poll_git_actions(window, cx);
                    app.git_operation.is_none()
                })
            });
            if done {
                break;
            }
            assert!(Instant::now() < deadline, "Git UI operation timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(
                app.git_feedback.values().all(|feedback| !feedback.error),
                "{}",
                app.git_feedback
                    .values()
                    .map(|feedback| feedback.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        });
    }
    draw(cx);
    let trigger = cx.debug_bounds("git-branch-trigger").unwrap().center();
    pointer_click(cx, trigger, MouseButton::Left);
    let actions = cx
        .debug_bounds("git-branch-actions-1")
        .expect("feature action button")
        .center();
    pointer_click(cx, actions, MouseButton::Left);
    let checkout = cx.update(|window, _| {
        window
            .within("popup-menu")
            .find_by_label("Checkout")
            .bounds()
            .center()
    });
    pointer_click(cx, checkout, MouseButton::Left);
    cx.update(|_, cx| {
        assert!(
            app.read(cx).git_operation.is_some(),
            "Checkout click starts a Git operation"
        )
    });
    wait_operation(cx, &app);
    assert_eq!(run(&["branch", "--show-current"]), "feature");

    let metadata = git::load_repository(&repository.0, &AtomicBool::new(false)).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.branch_probe = None;
            app.git_repository = Some(metadata);
            cx.notify();
        })
    });
    draw(cx);
    let trigger = cx.debug_bounds("git-branch-trigger").unwrap().center();
    pointer_click(cx, trigger, MouseButton::Left);
    let row = cx.debug_bounds("git-branch-row-0").unwrap().center();
    pointer_click(cx, row, MouseButton::Right);
    let create = cx.update(|window, _| {
        window
            .within("popup-menu")
            .find_by_label("New Branch from 'feature'")
            .bounds()
            .center()
    });
    pointer_click(cx, create, MouseButton::Left);
    assert!(cx.debug_bounds("branch-form").is_some());
    assert!(cx.debug_bounds("git-branch-actions-0").is_some());
    assert!(cx.debug_bounds("dialog-0").is_none());
    let input = cx.update(|window, _| window.find_by_label("分支名").bounds().center());
    pointer_click(cx, input, MouseButton::Left);
    cx.update(|window, cx| window.input("ui-created", cx));
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    cx.update(|_, cx| {
        assert!(
            app.read(cx).git_operation.is_some(),
            "form confirmation starts creation"
        )
    });
    wait_operation(cx, &app);
    assert!(cx.debug_bounds("branch-form").is_none());
    assert!(
        cx.debug_bounds("git-branch-actions-0").is_some(),
        "creation preserves the Git popup"
    );
    assert_eq!(run(&["branch", "--show-current"]), "ui-created");
    assert_eq!(
        run(&["rev-parse", "ui-created"]),
        run(&["rev-parse", "feature"])
    );

    let metadata = git::load_repository(&repository.0, &AtomicBool::new(false)).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.branch_probe = None;
            app.git_repository = Some(metadata);
            cx.notify();
        })
    });
    draw(cx);
    let actions = cx.debug_bounds("git-branch-actions-0").unwrap().center();
    pointer_click(cx, actions, MouseButton::Left);
    let tracking = cx.update(|window, _| {
        window
            .within("popup-menu")
            .find_by_label("Tracked Branch")
            .bounds()
            .center()
    });
    cx.simulate_mouse_move(tracking, None, Modifiers::default());
    draw(cx);
    let upstream = cx.update(|window, _| {
        window
            .within("submenu")
            .find_by_label("origin/main")
            .bounds()
            .center()
    });
    pointer_click(cx, upstream, MouseButton::Left);
    cx.update(|_, cx| {
        assert!(
            app.read(cx).git_operation.is_some(),
            "submenu mouse selection starts tracking update"
        )
    });
    wait_operation(cx, &app);
    assert_eq!(run(&["config", "branch.ui-created.remote"]), "origin");
    assert_eq!(
        run(&["config", "branch.ui-created.merge"]),
        "refs/heads/main"
    );
    let metadata = git::load_repository(&repository.0, &AtomicBool::new(false)).unwrap();
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.branch_probe = None;
            app.git_repository = Some(metadata);
            cx.notify();
        })
    });
    draw(cx);
    let trigger = cx.debug_bounds("git-branch-trigger").unwrap().center();
    pointer_click(cx, trigger, MouseButton::Left);
    let row = cx.debug_bounds("git-branch-row-0").unwrap().center();
    pointer_click(cx, row, MouseButton::Right);
    let rename = cx.update(|window, _| {
        window
            .within("popup-menu")
            .find_by_label("Rename")
            .bounds()
            .center()
    });
    pointer_click(cx, rename, MouseButton::Left);
    assert!(cx.debug_bounds("dialog-0").is_none());
    let input = cx.update(|window, _| window.find_by_label("分支名").bounds().center());
    pointer_click(cx, input, MouseButton::Left);
    cx.simulate_keystrokes("ctrl-a");
    cx.update(|window, cx| window.input("ui-renamed", cx));
    draw(cx);
    let save = cx.debug_bounds("branch-form-submit").unwrap().center();
    pointer_click(cx, save, MouseButton::Left);
    wait_operation(cx, &app);
    assert_eq!(run(&["branch", "--show-current"]), "ui-renamed");
    assert_eq!(run(&["config", "branch.ui-renamed.remote"]), "origin");
    assert!(
        cx.debug_bounds("git-branch-actions-0").is_some(),
        "renaming keeps the Git popup open"
    );
    cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.branch_probe = None;
            app.git_probe = None;
        })
    });
}

#[gpui_kit::test]
fn model_picker_renders_search_provider_and_effort_without_overlapping(cx: &mut TestAppContext) {
    use crate::{
        domain::Workspace,
        models::{ModelCatalog, ModelOption},
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
            session.title = "模型面板".into();
            session.reasoning_effort = ReasoningEffort::High;
            session.channel = ModelChannel::Cli;
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir().join("conduit-model-picker-ui"),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.store.providers = vec![ModelProvider {
                id: "synced-fixture".into(),
                name: "Synced proxy".into(),
                engine: super::Provider::Claude,
                base_url: "https://fixture.invalid".into(),
                api_key_env: String::new(),
                models: vec!["synced-model".into()],
                source: ProviderSource::Ccs {
                    id: "fixture".into(),
                },
                model_metadata: vec![],
            }];
            app.catalogs.insert(
                super::Provider::Claude,
                ModelCatalog {
                    models: (0..12)
                        .map(|i| ModelOption {
                            id: format!("model-{i}"),
                            label: format!("Model {i}"),
                            note: "上下文 200K".into(),
                            context_window: Some(200_000),
                        })
                        .collect(),
                    ..Default::default()
                },
            );
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let trigger = cx.debug_bounds("composer-model-control").unwrap().center();
    cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    // An open panel must remain usable while the parent receives repeated
    // runtime notifications, rather than recursively invalidating itself.
    for _ in 0..20 {
        cx.update(|window, cx| {
            app.update(cx, |_, cx| cx.notify());
            let _ = window.draw(cx);
        });
        cx.run_until_parked();
    }
    let provider = cx
        .debug_bounds("session-channel-control")
        .expect("Provider selector");
    let engine = cx
        .debug_bounds("session-engine-control")
        .expect("engine inside model panel");
    let models = cx
        .debug_bounds("session-model-list")
        .expect("scrollable model list");
    let footer = cx
        .debug_bounds("session-model-footer")
        .expect("visible footer");
    assert!(engine.bottom() <= provider.top());
    assert!(provider.bottom() <= models.top());
    assert!(models.size.height <= px(250.));
    assert!(models.bottom() < footer.top());
    assert!(footer.bottom() < cx.debug_bounds("composer-model-control").unwrap().top());
    for (width, height) in [(850., 500.), (1200., 800.)] {
        cx.simulate_resize(gpui_kit::size(px(width), px(height)));
        cx.run_until_parked();
        for _ in 0..2 {
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            cx.run_until_parked();
        }
        let panel = cx.debug_bounds("session-model-panel").unwrap();
        assert!(
            panel.top() >= px(0.),
            "panel above viewport at {width}x{height}: {panel:?}"
        );
        assert!(panel.left() >= px(0.) && panel.right() <= px(width));
        let footer = cx.debug_bounds("session-model-footer").unwrap();
        assert!(
            footer.bottom() < cx.debug_bounds("composer-model-control").unwrap().top(),
            "footer overlaps composer: {footer:?}"
        );
    }
    for _ in 0..4 {
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("session-model-list").is_none());
        let trigger = cx.debug_bounds("composer-model-control").unwrap().center();
        cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("session-model-list").is_some());
    }
    // Synced and local entries share the same list. Test builds persist to
    // their own temporary directory, never to the user's application state.
    let provider = cx.debug_bounds("session-channel-control").unwrap().center();
    cx.simulate_mouse_down(provider, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(provider, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let choice = cx
        .debug_bounds("session-provider-option-1")
        .unwrap()
        .center();
    cx.simulate_mouse_down(choice, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(choice, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let selected = &app.read(cx).store.workspaces[0].sessions[0];
        assert_eq!(
            selected.channel,
            ModelChannel::Provider("synced-fixture".into())
        );
        assert_eq!(selected.reasoning_effort, ReasoningEffort::Medium);
        let _ = window.draw(cx);
    });
    let effort = cx.debug_bounds("session-effort-control").unwrap();
    let position = point(effort.left() + effort.size.width * 0.75, effort.center().y);
    cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    let search = cx.debug_bounds("session-model-search").unwrap().center();
    cx.simulate_mouse_down(search, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(search, MouseButton::Left, Modifiers::default());
    cx.simulate_keystrokes("n o m a t c h");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|_, cx| app.update(cx, |app, _| app.writable = false));
}

#[gpui_kit::test]
fn live_activity_shares_stats_row_and_tools_collapse_when_reply_starts(cx: &mut TestAppContext) {
    use crate::{domain::Workspace, protocol::Event, store::Store};
    use gpui_kit::AppContext as _;
    use gpui_kit::{Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.title = "状态布局回归".into();
            let item = |key: &str, kind, text: &str, status: &str| ChatItem {
                elapsed_ms: None,
                key: key.into(),
                created_at: 0,
                kind,
                text: text.into(),
                input: String::new(),
                detail: String::new(),
                status: status.into(),
                approval: None,
                model: None,
            };
            session.messages = vec![
                item("user", super::ItemKind::User, "检查代码", ""),
                item(
                    "turn-before",
                    super::ItemKind::Assistant,
                    "先检查代码。",
                    "",
                ),
                item("turn-a", super::ItemKind::Tool, "Read", "completed"),
                item("turn-b", super::ItemKind::Tool, "Read", "completed"),
            ];
            let path = std::env::temp_dir().join("conduit-status-ui");
            let store = Store {
                workspaces: vec![Workspace {
                    path: path.clone(),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.runs.insert(
                1,
                super::ActiveRun::new(
                    runtime,
                    path,
                    "turn".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app.runs.get_mut(&1).unwrap().model = Some("running-model".into());
            app.runs.get_mut(&1).unwrap().reasoning_effort = super::ReasoningEffort::High;
            app.session_mut(1).unwrap().model = Some("next-turn-model".into());
            app.session_mut(1).unwrap().reasoning_effort = super::ReasoningEffort::Low;
            app.plans.insert(
                1,
                vec![super::PlanStep {
                    text: "检查代码".into(),
                    status: super::PlanStatus::InProgress,
                }],
            );
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    let turn_start = std::time::Instant::now() - std::time::Duration::from_secs(12);
    cx.update(|_, cx| {
        app.update(cx, |this, _| {
            let run = this.runs.get_mut(&1).unwrap();
            run.turn_started = turn_start;
            run.usage_started = turn_start;
            run.turn_tokens_at_start = 100;
        })
    });
    sender
        .send(Event::Usage {
            input_tokens: 1,
            cached_input_tokens: 0,
            output_tokens: 1,
            total_tokens: 2,
            context_window: 0,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.update(|_, cx| {
        let run = app.read(cx).runs.get(&1).unwrap();
        assert_eq!(
            run.turn_started, turn_start,
            "token reset must not reset wall time"
        );
        assert!(run.usage_started > turn_start);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let activity = cx.debug_bounds("engine-activity").unwrap();
    let stats = cx.debug_bounds("turn-stats").unwrap();
    let text = cx.debug_bounds("engine-activity-text").unwrap();
    let spinner = cx.debug_bounds("engine-activity-spinner").unwrap();
    let elapsed = cx.debug_bounds("engine-elapsed").unwrap();
    assert!(elapsed.left() >= spinner.right());
    let model = cx.debug_bounds("engine-model").unwrap();
    let effort = cx.debug_bounds("engine-reasoning-effort").unwrap();
    assert!(model.left() >= elapsed.right());
    assert!(effort.left() >= model.right());
    cx.update(|_, cx| {
        let run = app.read(cx).runs.get(&1).unwrap();
        assert_eq!(run.model.as_deref(), Some("running-model"));
        assert_eq!(run.reasoning_effort, super::ReasoningEffort::High);
    });
    assert!((activity.center().y - stats.center().y).abs() < px(1.));
    assert!(
        stats.left() >= activity.right(),
        "step and file tags dock right of the activity strip"
    );
    assert!(spinner.left() >= text.right());
    assert!(spinner.left() - text.right() <= px(10.));
    // Tags reveal the step and file details above them; a click elsewhere
    // dismisses the panel.
    let tags = cx.debug_bounds("turn-stats").unwrap().center();
    cx.simulate_mouse_down(tags, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(tags, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("turn-stats-details").is_some());
    cx.simulate_mouse_down(
        point(px(6.), px(6.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        point(px(6.), px(6.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("turn-stats-details").is_none());
    assert!(cx.debug_bounds("activity-1-2").is_some());
    assert!(cx.debug_bounds("activity-1-3").is_some());
    assert!(cx.debug_bounds("reply-actions-1").is_none());
    let before_height = cx.debug_bounds("message-row-1").unwrap().size.height;

    sender
        .send(Event::Delta {
            key: "reply".into(),
            text: "检查完成".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("tool-group-header-1-2").is_some());
    assert!(cx.debug_bounds("activity-1-2").is_none());
    assert!(cx.debug_bounds("activity-1-3").is_none());
    assert!(cx.debug_bounds("assistant-message-1").is_some());
    assert!(cx.debug_bounds("reply-actions-1").is_none());
    assert!(cx.debug_bounds("reply-actions-4").is_none());
    assert_eq!(
        cx.debug_bounds("message-row-1").unwrap().size.height,
        before_height
    );
    assert!(
        cx.update(|_, cx| app.read(cx).runs.contains_key(&1)),
        "collapse happens while the reply is streaming"
    );

    // Further segments must keep every footer hidden throughout this run;
    // only the final visible reply gains a footer when Finished arrives.
    let reply_height = cx.debug_bounds("message-row-4").unwrap().size.height;
    sender
        .send(Event::Delta {
            key: "followup".into(),
            text: "补充检查结果。".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("reply-actions-4").is_none());
    assert!(cx.debug_bounds("reply-actions-5").is_none());
    assert_eq!(
        cx.debug_bounds("message-row-4").unwrap().size.height,
        reply_height
    );

    sender
        .send(Event::Text {
            key: "followup".into(),
            text: " \n".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("reply-actions-4").is_none());
    assert!(cx.debug_bounds("reply-actions-5").is_none());
    assert_eq!(
        cx.debug_bounds("message-row-4").unwrap().size.height,
        reply_height
    );

    sender
        .send(Event::Finished {
            error: None,
            cancelled: false,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("engine-activity").is_none());
    assert!(cx.debug_bounds("reply-elapsed-4").is_some());
    assert!(cx.debug_bounds("reply-actions-4").is_some());
    assert!(cx.debug_bounds("reply-actions-1").is_none());
    assert!(cx.debug_bounds("reply-actions-5").is_none());
    cx.update(|_, cx| {
        let session = app.read(cx).session(1).unwrap();
        assert!(session.messages[4].elapsed_ms.unwrap() >= 12_000);
        assert!(session.messages[0].elapsed_ms.is_none());
        let json = serde_json::to_string(session).unwrap();
        let restored: crate::domain::SessionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(
            restored.messages[4].elapsed_ms,
            session.messages[4].elapsed_ms
        );
        let mut legacy = serde_json::to_value(&session.messages[4]).unwrap();
        legacy.as_object_mut().unwrap().remove("elapsed_ms");
        assert!(
            serde_json::from_value::<ChatItem>(legacy)
                .unwrap()
                .elapsed_ms
                .is_none()
        );
    });
}

#[gpui_kit::test]
fn empty_chat_blocks_have_no_chrome_and_streaming_replies_reflow_tool_groups(
    cx: &mut TestAppContext,
) {
    use crate::{domain::Workspace, protocol::Event, store::Store};
    use gpui_kit::AppContext as _;

    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            let item = |key: &str, kind, text: &str| ChatItem {
                elapsed_ms: None,
                key: key.into(),
                created_at: 0,
                kind,
                text: text.into(),
                input: String::new(),
                detail: String::new(),
                status: "completed".into(),
                approval: None,
                model: None,
            };
            session.messages = vec![
                item("user", super::ItemKind::User, "检查聊天排版"),
                item("turn-before", super::ItemKind::Assistant, " \n"),
                item("turn-a", super::ItemKind::Tool, "Read"),
                item("turn-between", super::ItemKind::Assistant, ""),
                item("turn-b", super::ItemKind::Tool, "exec_command"),
                item("turn-reply", super::ItemKind::Assistant, ""),
            ];
            session.messages[2].text = "WebFetch".into();
            session.messages[2].input = r#"{"url":"https://example.com"}"#.into();
            session.record_tool_approval(
                "turn",
                "fetch-approval",
                "请求使用 WebFetch",
                r#"{"tool_use_id":"a"}"#,
                "已允许",
            );
            let path = std::env::temp_dir().join("conduit-empty-chat-ui");
            let store = Store {
                workspaces: vec![Workspace {
                    path: path.clone(),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.runs.insert(
                1,
                super::ActiveRun::new(
                    runtime,
                    path,
                    "turn".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    for (index, message, actions) in [
        (1, "assistant-message-1", "reply-actions-1"),
        (3, "assistant-message-3", "reply-actions-3"),
        (5, "assistant-message-5", "reply-actions-5"),
    ] {
        assert!(cx.debug_bounds(message).is_none());
        assert!(cx.debug_bounds(actions).is_none());
        let height = cx.update(|_, cx| {
            app.read(cx)
                .scroll
                .bounds_for_item(index)
                .map_or(px(0.), |bounds| bounds.size.height)
        });
        assert_eq!(
            height,
            px(0.),
            "empty stream blocks must not reserve row spacing"
        );
    }
    let first = cx.debug_bounds("activity-1-2").unwrap();
    let second = cx.debug_bounds("activity-1-4").unwrap();
    assert!(second.top() - first.bottom() < px(10.));
    assert_eq!(first.left(), second.left());
    assert_eq!(first.size.height, second.size.height);
    assert!(cx.debug_bounds("activity-approval-1-2").is_some());
    let category_left = cx.debug_bounds("activity-category-1-2").unwrap().left();
    assert_eq!(
        category_left,
        cx.debug_bounds("activity-category-1-4").unwrap().left()
    );
    assert!(cx.debug_bounds("tool-group-header-1-2").is_some());

    sender
        .send(Event::Delta {
            key: "reply".into(),
            text: "## 检查结果\n\n- 间距恢复正常\n- 工具记录保持紧凑".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("assistant-message-5").is_some());
    assert!(cx.debug_bounds("activity-1-2").is_none());
    assert!(cx.debug_bounds("activity-1-4").is_none());
    assert!(cx.debug_bounds("tool-group-header-1-2").is_some());

    // A delayed text delta splits the group into two singleton groups, each
    // still collapsed once its following reply is visible.
    sender
        .send(Event::Delta {
            key: "between".into(),
            text: "读取文件完成。".into(),
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("assistant-message-3").is_some());
    assert!(cx.debug_bounds("tool-group-header-1-2").is_some());
    assert!(cx.debug_bounds("activity-1-2").is_none());
    assert!(cx.debug_bounds("activity-1-4").is_none());
    assert!(cx.debug_bounds("tool-group-header-1-4").is_some());

    let header = cx.debug_bounds("tool-group-header-1-4").unwrap();
    assert!(header.size.height > px(20.));
    // A single command must remain inspectable, and collapse again on demand.
    for expanded in [true, false] {
        let position = cx.debug_bounds("tool-group-header-1-4").unwrap().center();
        cx.simulate_mouse_down(
            position,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            position,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(cx.debug_bounds("activity-1-4").is_some(), expanded);
    }
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.session(1).unwrap().messages.len(), 6);
    });
}

#[gpui_kit::test]
fn session_naming_applies_success_and_reports_failure_without_starting_cli(
    cx: &mut TestAppContext,
) {
    use crate::{domain::Workspace, protocol::Event, store::Store};
    use gpui_kit::AppContext as _;

    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.title = "原始标题".into();
            session.messages.push(ChatItem {
                elapsed_ms: None,
                key: "user".into(),
                created_at: 0,
                kind: super::ItemKind::User,
                text: "优化聊天界面".into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            });
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir().join("conduit-naming-ui"),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            for (reply, error, expected) in [
                ("聊天界面优化", None, None),
                ("", Some("模拟网络失败"), Some("模拟网络失败")),
                ("", None, Some("模型未返回有效标题")),
            ] {
                let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
                this.naming_errors.remove(&1);
                this.session_namings.insert(
                    1,
                    super::SessionNaming {
                        automatic: true,
                        runtime,
                        started: std::time::Instant::now(),
                        parts: vec![],
                        error: None,
                    },
                );
                this.poll_session_namings(cx);
                assert!(this.session_namings.contains_key(&1));
                sender
                    .send(Event::Text {
                        key: "title".into(),
                        text: reply.into(),
                    })
                    .unwrap();
                sender
                    .send(Event::Finished {
                        error: error.map(String::from),
                        cancelled: false,
                    })
                    .unwrap();
                this.poll_session_namings(cx);
                assert!(!this.session_namings.contains_key(&1));
                assert_eq!(this.naming_errors.get(&1).map(String::as_str), expected);
                assert_eq!(this.session(1).unwrap().title, "聊天界面优化");
            }
        })
    });
}

#[gpui_kit::test]
fn concurrent_sessions_isolate_events_approvals_cancellation_and_history(cx: &mut TestAppContext) {
    use crate::{
        domain::{Provider, SessionRecord, Workspace},
        protocol::{Approval, ApprovalKind, Event},
        store::Store,
    };
    use gpui_kit::AppContext as _;

    let session = |id, provider, native: &str| SessionRecord {
        id,
        provider,
        native_id: Some(native.into()),
        title: format!("Chat {id}"),
        created_at: 0,
        exit_code: None,
        archived: false,
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
            app.runs.clear();
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
                this.runs.insert(
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
            assert_eq!(this.runs.len(), 3);
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
                this.runs.len(),
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
                    this.runs[&id].approvals.front().unwrap().title,
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
            assert!(this.runs[&1].expanded_approvals.is_empty());
            for id in [2, 3] {
                assert!(this.runs[&id].expanded_approvals.contains("approval-1"));
                assert_eq!(this.runs[&id].approvals.len(), 1);
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
                window,
                cx,
            );
            assert_eq!(this.session(1).unwrap().messages[0].text, "reply-1");
            this.runs.get_mut(&1).unwrap().cancel();
            assert!(this.runs[&1].runtime.is_cancelled());
            for id in [2, 3] {
                assert!(!this.runs[&id].runtime.is_cancelled());
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
            assert!(!this.runs.contains_key(&1));
            assert!(
                !this.unread_completed.contains(&1),
                "cancellation is not a completed reply"
            );
            assert_eq!(this.runs.len(), 2);
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
                this.unread_completed.contains(&2),
                "successful background replies become unread"
            );
            this.switch(0, Some(2), window, cx);
            assert!(
                !this.unread_completed.contains(&2),
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
                !this.unread_completed.contains(&3),
                "failures are not marked as successful replies"
            );
            assert_eq!(
                this.session(3).unwrap().messages.last().unwrap().detail,
                "network timeout\nraw details"
            );
            this.selected_session = Some(1);
            this.selected_workspace = Some(0);
            this.git_operation = Some(super::GitOperation {
                probe: crate::git::OperationProbe::test_pending(root.clone()),
                worktree: root.clone(),
                draft: String::new(),
            });
            assert!(this.send_blocked(true).unwrap().contains("Git 操作"));
            this.selected_workspace = Some(1);
            this.selected_session = None;
            assert!(
                this.send_blocked(true).is_none(),
                "Git writes cannot block another workspace"
            );
            this.runs.clear();
            assert!(
                !this.workspace_busy(1),
                "unrelated Git writes cannot block workspace removal"
            );
            assert!(this.workspace_busy(0));
            this.git_operation = None;
        });
    });
}

#[gpui_kit::test]
fn long_chat_virtualizes_real_rows_and_keeps_history_jump_on_append(cx: &mut TestAppContext) {
    use crate::{
        domain::{ItemKind, PermissionLevel, Provider, SessionRecord, Usage, Workspace},
        store::Store,
    };
    use gpui_kit::AppContext;
    cx.update(gpui_kit::init);
    let app_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = app_slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            let messages = (0..10_000)
                .map(|index| ChatItem {
                    elapsed_ms: None,
                    key: format!("message-{index}"),
                    created_at: 0,
                    kind: if index % 2 == 0 {
                        ItemKind::User
                    } else {
                        ItemKind::Notice
                    },
                    text: format!("历史消息 {index}"),
                    input: String::new(),
                    detail: String::new(),
                    status: String::new(),
                    approval: None,
                    model: None,
                })
                .collect();
            app.store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions: vec![SessionRecord {
                        id: 1,
                        provider: Provider::Claude,
                        title: "Long chat".into(),
                        created_at: 0,
                        exit_code: None,
                        archived: false,
                        native_id: None,
                        model: None,
                        messages,
                        interrupted: false,
                        channel: Default::default(),
                        reasoning_effort: Default::default(),
                        opencode: Default::default(),
                        permission: PermissionLevel::default(),
                        usage: Usage::default(),
                    }],
                }],
                ..Default::default()
            };
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
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
    cx.update(|_, cx| {
        let app = app.read(cx);
        let monitor = app.performance.read(cx);
        assert_eq!(monitor.message_count, 10_000);
        assert_eq!(app.outline.entries.len(), 5_000);
        assert!(app.outline.scroll.0.borrow().last_item_size.is_some());
        assert!(monitor.mounted_messages() > 0);
        assert!(
            monitor.mounted_messages() < 100,
            "offscreen rows must not be constructed"
        );
    });
    cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "message-5000", cx)));
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let before = cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.scroll.bounds_for_item(5000).is_some());
        assert!(!app.at_bottom);
        app.scroll.logical_scroll_top()
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            super::apply_event(
                app.session_mut(1).unwrap(),
                "turn-new",
                crate::protocol::Event::Delta {
                    key: "reply".into(),
                    text: "new streaming content".into(),
                },
            );
            cx.notify();
        })
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        let after = app.scroll.logical_scroll_top();
        assert_eq!(
            (after.item_ix, after.offset_in_item),
            (before.item_ix, before.offset_in_item),
            "new tail content must not pull readers away from history"
        );
        assert!(app.performance.read(cx).mounted_messages() < 100);
    });

    // Scrolling inside the same outline section keeps session navigation cached.
    let render_count = cx.update(|_, cx| app.read(cx).sidebar_view.read(cx).render_count);
    let position = cx.debug_bounds("user-message-5000").unwrap().center();
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(-8.))),
        ..Default::default()
    });
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.sidebar_view.read(cx).render_count, render_count);
        let after = app.scroll.logical_scroll_top();
        assert_ne!(
            (after.item_ix, after.offset_in_item),
            (before.item_ix, before.offset_in_item),
            "the transcript must still scroll while session navigation stays cached"
        );
        assert!(app.performance.read(cx).mounted_messages() < 100);
    });
}

struct ToolScrollFixture {
    outer: ScrollHandle,
    inner: ScrollHandle,
    sibling: ScrollHandle,
    item: ChatItem,
    typography: ChatTypography,
}

impl Render for ToolScrollFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Exercise the production tool body inside another scrolling list.
        div().size_full().child(
            div()
                .id("outer")
                .w(px(600.))
                .h(px(320.))
                .overflow_y_scroll()
                .track_scroll(&self.outer)
                .child(
                    v_flex()
                        .gap_4()
                        .child(tool_content("first", &self.item, &self.inner, &self.typography).0)
                        .child(
                            tool_content("second", &self.item, &self.sibling, &self.typography).0,
                        ),
                ),
        )
    }
}

#[gpui_kit::test]
fn expanded_tool_long_text_scrolls_inside_its_fixed_viewport(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let outer = ScrollHandle::new();
    let inner = ScrollHandle::new();
    let sibling = ScrollHandle::new();
    let (_, cx) = cx.add_window_view(|_, _| ToolScrollFixture {
        outer: outer.clone(),
        inner: inner.clone(),
        sibling: sibling.clone(),
        typography: ChatTypography {
            body_font: None,
            body_size: 15.0,
            line_height: 1.5,
            code_font: "Consolas".into(),
            code_size: 12.0,
            code_copy: true,
            syntax_highlight: true,
            code_line_numbers: false,
        },
        item: serde_json::from_value(serde_json::json!({
            "key":"tool", "kind":"Tool", "text":"Bash", "input":"{\"command\":\"test\"}",
            "detail": (0..200).map(|i| format!("line {i}: 中文 output\n")).collect::<String>(),
            "status":"完成"
        }))
        .unwrap(),
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(inner.bounds().size.height, px(240.));
    assert!(
        inner.max_offset().y > px(1000.),
        "long output must retain its natural height"
    );
    let outer_before = outer.offset();
    let sibling_before = sibling.offset();
    cx.simulate_event(ScrollWheelEvent {
        position: inner.bounds().center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(inner.offset().y < px(0.), "wheel must scroll tool text");
    assert_eq!(
        outer.offset(),
        outer_before,
        "tool scrolling must leave chat in place"
    );
    assert_eq!(
        sibling.offset(),
        sibling_before,
        "other tools have independent scroll state"
    );
    let inner_before = inner.offset();
    cx.simulate_event(ScrollWheelEvent {
        // The gap between cards belongs to the outer chat viewport.
        position: point(px(30.), px(248.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-80.))),
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(outer.offset().y < outer_before.y);
    assert_eq!(inner.offset(), inner_before);
}

/// Drives a real pointer drag across an assistant reply inside the
/// production virtual list and reports what the selection state and the
/// painted highlight do at each pointer position.
#[gpui_kit::test]
fn drag_across_assistant_reply_tracks_the_pointer(cx: &mut TestAppContext) {
    const REPLY: &str = "第一段回复内容用于验证拖拽选中是否跟随指针移动，这里需要足够长的中文来覆盖两行以上的文本区域。第二段内容继续保持长度以便跨行拖拽选中。";
    check_assistant_drag_selection(cx, REPLY, REPLY, false);
}

const SELECTION_CODE: &str = "let selected_text = \"CODE_SELECTION enough words to verify that dragging across a code block updates the painted selection immediately and keeps following the pointer while this deliberately long code line wraps across the block width for the regression test\";\nlet second_line = \"中文代码选区\";\nlet third_line = \"another long line keeps the selection endpoint visible when the pointer moves farther right\";";

#[gpui_kit::test]
fn drag_across_numbered_code_tracks_the_pointer(cx: &mut TestAppContext) {
    let reply = format!("```rust\n{SELECTION_CODE}\n```");
    check_assistant_drag_selection(cx, &reply, SELECTION_CODE, true);
}

#[gpui_kit::test]
fn drag_across_stock_code_tracks_the_pointer(cx: &mut TestAppContext) {
    let reply = format!("```rust\n{SELECTION_CODE}\n```");
    check_assistant_drag_selection(cx, &reply, SELECTION_CODE, false);
}

fn check_assistant_drag_selection(
    cx: &mut TestAppContext,
    reply: &str,
    selectable_text: &str,
    code_line_numbers: bool,
) {
    use crate::{
        domain::{ItemKind, PermissionLevel, Provider, SessionRecord, Usage, Workspace},
        store::Store,
    };
    use gpui_kit::{AppContext as _, Modifiers, MouseButton, base::TextSelection};

    const USER_PROMPT: &str = "USER_PROMPT_TEXT enough words to make a wide selectable user bubble for checking repaint during dragging across the text.";

    cx.update(gpui_kit::init);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            let item = |key: &str, kind: ItemKind, text: &str| ChatItem {
                elapsed_ms: None,
                key: key.into(),
                created_at: 0,
                kind,
                text: text.into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            };
            app.store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions: vec![SessionRecord {
                        id: 1,
                        provider: Provider::Claude,
                        title: "选中".into(),
                        created_at: 0,
                        exit_code: None,
                        archived: false,
                        native_id: None,
                        model: None,
                        messages: vec![
                            item("user-1", ItemKind::User, USER_PROMPT),
                            item("reply-1", ItemKind::Assistant, reply),
                        ],
                        interrupted: false,
                        channel: Default::default(),
                        reasoning_effort: Default::default(),
                        opencode: Default::default(),
                        permission: PermissionLevel::default(),
                        usage: Usage::default(),
                    }],
                }],
                ..Default::default()
            };
            app.store.ui.code_line_numbers = code_line_numbers;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    // The virtual list reports no item bounds until it has been scrolled,
    // so find a painted text line by probing short drags instead.
    let probe = |cx: &mut gpui_kit::VisualTestContext, x, y| {
        cx.simulate_mouse_down(point(x, y), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            point(x + px(30.), y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(x + px(30.), y),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(TextSelection::selected_text).trim().to_string()
    };
    let mut line = None;
    'probe: for y in (120..1000).step_by(12) {
        for x in [360, 560, 760] {
            let selected = probe(cx, px(x as f32), px(y as f32));
            if selected.is_empty() {
                continue;
            }
            if selectable_text.contains(&selected) {
                line = Some((px(x as f32), px(y as f32), selected));
                break 'probe;
            }
        }
    }
    let (anchor_x, y, found) = line.expect("a selectable reply text line");
    assert!(found.len() > 3, "probe must land inside a word: {found}");

    let read = |cx: &mut gpui_kit::VisualTestContext| {
        let selected = cx.update(TextSelection::selected_text).trim().to_string();
        let painted = cx.update(|window, cx| {
            let color: gpui_kit::Background = gpui_kit::base::Theme::global(cx)
                .tokens
                .colors
                .selection
                .into();
            let scale = window.scale_factor();
            let mut right = px(0.);
            let mut count = 0usize;
            for quad in window.painted_quads() {
                if quad.background != color {
                    continue;
                }
                let bounds = quad.bounds.map(|value| px(value.0 / scale));
                count += 1;
                right = right.max(bounds.right());
            }
            (count, right)
        });
        (selected, painted)
    };

    cx.simulate_mouse_down(
        point(anchor_x + px(10.), y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let move_to = |cx: &mut gpui_kit::VisualTestContext, x: gpui_kit::Pixels| {
        // Let selection changes schedule the frame. Forcing window.draw here
        // would hide a participant that forgets to request a repaint.
        cx.simulate_mouse_move(point(x, y), Some(MouseButton::Left), Modifiers::default());
        cx.run_until_parked();
        read(cx)
    };
    let (at_mid, painted_mid) = move_to(cx, anchor_x + px(150.));
    let (at_far, painted_far) = move_to(cx, anchor_x + px(330.));
    cx.simulate_mouse_up(
        point(anchor_x + px(330.), y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();

    assert!(!at_mid.is_empty(), "drag must select reply text");
    assert!(
        selectable_text.contains(&at_mid),
        "selection must stay inside the reply: {at_mid}"
    );
    assert!(
        at_far.chars().count() > at_mid.chars().count(),
        "selection must grow with the pointer: {at_mid} -> {at_far}"
    );
    assert!(
        painted_mid.0 > 0,
        "the highlight must be painted while dragging"
    );
    assert!(
        painted_far.1 > painted_mid.1,
        "the painted highlight must follow the pointer: {:?} -> {:?}",
        painted_mid.1,
        painted_far.1
    );

    // Ctrl+C only reaches the window root's copy handler because the
    // selection drag handed keyboard focus back to the conversation.
    cx.update(|window, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("SENTINEL".into()));
        window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx);
    });
    cx.run_until_parked();
    let copied = cx.update(|_, cx| {
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    });
    assert_eq!(copied, at_far, "Ctrl+C must copy the chat selection");

    let mut user_line = None;
    let bubble = cx
        .debug_bounds("user-bubble-0")
        .expect("a visible user bubble");
    'probe_user: for user_y in
        ((f32::from(bubble.top()) + 8.) as i32..f32::from(bubble.bottom()) as i32).step_by(8)
    {
        for user_x in
            ((f32::from(bubble.left()) + 16.) as i32..f32::from(bubble.right()) as i32).step_by(40)
        {
            let selected = probe(cx, px(user_x as f32), px(user_y as f32));
            if !selected.is_empty() && USER_PROMPT.contains(&selected) {
                user_line = Some((px(user_x as f32), px(user_y as f32)));
                break 'probe_user;
            }
        }
    }
    let (user_x, user_y) = user_line.expect("a selectable user text line");
    cx.simulate_mouse_down(
        point(user_x + px(10.), user_y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    let drag_user = |cx: &mut gpui_kit::VisualTestContext, x: f32| {
        cx.simulate_mouse_move(
            point(user_x + px(x), user_y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        read(cx)
    };
    let (user_mid, user_painted_mid) = drag_user(cx, 100.);
    let (user_far, user_painted_far) = drag_user(cx, 220.);
    assert!(
        !user_mid.is_empty(),
        "user selection should update during drag"
    );
    assert!(
        USER_PROMPT.contains(&user_mid),
        "unexpected user selection: {user_mid}"
    );
    assert!(user_far.chars().count() > user_mid.chars().count());
    assert!(
        user_painted_mid.0 > 0,
        "user highlight should paint during drag"
    );
    assert!(user_painted_far.1 > user_painted_mid.1);
    cx.simulate_mouse_up(
        point(user_x + px(220.), user_y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
}

#[gpui_kit::test]
fn sidebar_bottom_bar_matches_the_toolbar_and_opens_about(cx: &mut TestAppContext) {
    use gpui_kit::AppContext;
    cx.update(gpui_kit::init);
    let app_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let slot = app_slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| super::ConduitApp::new(window, cx, false, false));
        *slot.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = app_slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        // Both bottom bars are 34px strips pinned to the window edge. The
        // rail used to carry its own bottom padding, which lifted the left
        // bar above the conversation toolbar.
        let scale = window.scale_factor();
        let height = f32::from(window.viewport_size().height);
        let mut bars: Vec<(f32, f32, f32)> = Vec::new();
        for quad in window.painted_quads() {
            let bounds = quad.bounds.map(|value| px(value.0 / scale));
            let top = f32::from(bounds.top());
            let bar_height = f32::from(bounds.size.height);
            if top < height - 60.
                || (bar_height - 34.).abs() > 4.
                || f32::from(bounds.size.width) < 60.
            {
                continue;
            }
            bars.push((top, f32::from(bounds.left()), bar_height));
        }
        let rail = bars
            .iter()
            .find(|(_, left, _)| *left < 100.)
            .expect("the sidebar bottom bar must be painted");
        let toolbar = bars
            .iter()
            .find(|(_, left, _)| *left > 200.)
            .expect("the conversation toolbar must be painted");
        assert!(
            (rail.0 - toolbar.0).abs() < 1. && (rail.2 - toolbar.2).abs() < 1.,
            "both bottom bars must be the same height on the same line: {bars:?}"
        );
        assert!(
            (rail.0 + rail.2 - height).abs() < 1.,
            "the bottom bars must reach the window edge: {bars:?} / {height}"
        );
    });
    cx.update(|window, cx| {
        app.update(cx, |this, cx| this.open_about_dialog(window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(
            window.has_active_dialog(cx),
            "about icon must open a dialog"
        );
        let _ = window.draw(cx);
    });
    cx.update(|window, cx| {
        window.close_dialog(cx);
        assert!(!window.has_active_dialog(cx));
        let _ = window.draw(cx);
    });
}

/// 资源管理器把文件拖到窗口上时，gpui 会发出一个载荷为 `ExternalPaths` 的拖拽；
/// 这里从平台事件一路打到输入框，验证 composer 的 drop 接线确实生效。
#[gpui_kit::test]
fn platform_file_drop_becomes_a_reference_in_the_composer(cx: &mut TestAppContext) {
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
        id,
        provider: Provider::Claude,
        created_at: 0,
        exit_code: None,
        title: format!("Chat {id}"),
        archived: false,
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
            assert!(this.runs.is_empty(), "invalid files must not start a CLI");
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
    std::fs::remove_file(second).unwrap();
    let _ = std::fs::remove_dir(root);
}
#[gpui_kit::test]
fn attachment_tags_size_to_names_and_stay_inside_the_user_bubble(cx: &mut TestAppContext) {
    use crate::{
        domain::{ItemKind, Provider, SessionRecord, Workspace},
        file_ref::FileReference,
        store::Store,
    };
    use gpui_kit::AppContext as _;

    let workspace = std::env::temp_dir();
    let references = [
        "a.rs".to_string(),
        "a_descriptive_filename.rs".to_string(),
        format!("{}.rs", "long_name_".repeat(18)),
    ]
    .into_iter()
    .map(|name| FileReference {
        path: workspace.join(&name),
        mention: name,
    })
    .collect::<Vec<_>>();
    let prompt = file_ref::compose_prompt("这个文档里有什么内容", &references);
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            app.store = Store {
                workspaces: vec![Workspace {
                    path: workspace.clone(),
                    sessions: vec![SessionRecord {
                        id: 1,
                        provider: Provider::Claude,
                        created_at: 0,
                        exit_code: None,
                        title: "Files".into(),
                        archived: false,
                        native_id: None,
                        model: None,
                        messages: vec![ChatItem {
                            elapsed_ms: None,
                            key: "user-files".into(),
                            created_at: 0,
                            kind: ItemKind::User,
                            text: prompt.clone(),
                            input: String::new(),
                            detail: String::new(),
                            status: String::new(),
                            approval: None,
                            model: None,
                        }],
                        interrupted: false,
                        channel: Default::default(),
                        reasoning_effort: Default::default(),
                        opencode: Default::default(),
                        permission: Default::default(),
                        usage: Default::default(),
                    }],
                }],
                ..Default::default()
            };
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.writable = true;
            app.composer_files.insert(
                app.composer_target().unwrap(),
                super::ComposerFiles {
                    references: references.clone(),
                    feedback: None,
                },
            );
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let short = cx
        .debug_bounds("composer-file-0")
        .expect("short composer tag");
    let medium = cx
        .debug_bounds("composer-file-1")
        .expect("medium composer tag");
    let long = cx
        .debug_bounds("composer-file-2")
        .expect("long composer tag");
    assert!(
        short.size.width < medium.size.width,
        "short names must shrink: {short:?} / {medium:?}"
    );
    assert!(medium.size.width <= long.size.width);
    assert!(
        long.size.width <= px(220.),
        "long names must be bounded: {long:?}"
    );
    assert!(short.size.height <= px(26.));
    let row = cx.debug_bounds("user-attachments-0").unwrap();
    let bubble = cx.debug_bounds("user-bubble-0").unwrap();
    let body = cx.debug_bounds("user-body-0").unwrap();
    let first = cx.debug_bounds("user-file-0-0").unwrap();
    let second = cx.debug_bounds("user-file-0-1").unwrap();
    let last = cx.debug_bounds("user-file-0-2").unwrap();
    assert!(row.left() >= bubble.left() + px(15.));
    assert!(row.right() <= bubble.right() - px(15.));
    assert!(row.bottom() <= bubble.bottom() - px(11.));
    assert!(
        (first.right() - body.right()).abs() < px(1.),
        "attachment tags must align with the bubble content: {first:?} / {body:?}"
    );
    assert!(
        row.top() >= body.bottom(),
        "attachments belong below the message body"
    );
    assert!(
        second.right() < first.left(),
        "attachments must flow right to left"
    );
    assert!(first.size.width < second.size.width);
    assert!(last.size.width <= px(200.));
    assert!(first.size.height <= px(24.));
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).store.workspaces[0].sessions[0].messages[0].text,
            prompt,
            "display changes must preserve engine and retry context"
        );
    });
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            this.store.workspaces[0].sessions[0].messages[0].text =
                file_ref::compose_prompt("", &references);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(
        cx.debug_bounds("user-bubble-0").is_some(),
        "file-only messages keep their attachments inside the bubble"
    );
    assert!(cx.debug_bounds("user-body-0").is_none());
    assert!(cx.debug_bounds("user-file-0-0").is_some());
}
#[gpui_kit::test]
fn cli_error_details_expand_copy_raw_diagnostics_and_collapse(cx: &mut TestAppContext) {
    use crate::{domain::Workspace, store::Store};
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let raw = "connection closed\n".repeat(40);
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            session.messages = vec![ChatItem {
                elapsed_ms: None,
                key: "error".into(),
                created_at: 0,
                kind: super::ItemKind::Error,
                text: "执行失败：connection closed".into(),
                detail: raw.clone(),
                input: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            }];
            let store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir().join("conduit-error-ui"),
                    sessions: vec![session],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("cli-error-details-0").is_none());
    let collapsed = cx.debug_bounds("cli-error-0").unwrap();
    let toggle = cx.debug_bounds("cli-error-toggle-0").unwrap().center();
    cx.simulate_mouse_down(toggle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(toggle, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let details = cx
        .debug_bounds("cli-error-details-0")
        .expect("expanded details");
    assert!(details.size.height <= px(200.));
    assert!(cx.debug_bounds("cli-error-0").unwrap().size.height > collapsed.size.height);
    let copy = cx.debug_bounds("cli-error-copy-0").unwrap().center();
    cx.simulate_mouse_down(copy, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(copy, MouseButton::Left, Modifiers::default());
    cx.update(|_, cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(raw.as_str())
        );
    });
    let toggle = cx.debug_bounds("cli-error-toggle-0").unwrap().center();
    cx.simulate_mouse_down(toggle, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(toggle, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("cli-error-details-0").is_none());
    assert_eq!(
        cx.debug_bounds("cli-error-0").unwrap().size.height,
        collapsed.size.height
    );
}

#[gpui_kit::test]
fn deleting_blank_session_skips_confirmation(cx: &mut TestAppContext) {
    use crate::{
        domain::{ItemKind, Provider, SessionRecord, Workspace},
        store::Store,
    };
    use gpui_kit::AppContext as _;
    use gpui_kit::test::TestWindowExt as _;

    let session = |id, messages| SessionRecord {
        id,
        provider: Provider::Claude,
        created_at: 0,
        exit_code: None,
        title: "新会话".into(),
        archived: false,
        native_id: None,
        model: None,
        messages,
        interrupted: false,
        channel: Default::default(),
        reasoning_effort: Default::default(),
        opencode: Default::default(),
        permission: Default::default(),
        usage: Default::default(),
    };
    let used = session(
        2,
        vec![ChatItem {
            elapsed_ms: None,
            key: "user".into(),
            created_at: 0,
            kind: ItemKind::User,
            text: "hello".into(),
            input: String::new(),
            detail: String::new(),
            status: String::new(),
            approval: None,
            model: None,
        }],
    );
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = super::ConduitApp::new(window, cx, false, false);
            app.store = Store {
                workspaces: vec![Workspace {
                    path: std::env::temp_dir(),
                    sessions: vec![session(1, vec![]), used],
                }],
                ..Default::default()
            };
            app.writable = false;
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            this.request_delete_session(1, cx);
            assert!(
                this.session(1).is_none(),
                "a session with no messages must delete without confirmation"
            );
            assert_eq!(this.delete_target, None);

            this.request_delete_session(2, cx);
            assert!(
                this.session(2).is_some(),
                "a session with messages still asks for confirmation"
            );
            assert_eq!(this.delete_target, Some(2));
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("delete-session-confirmation").is_some());
    assert!(cx.debug_bounds("dialog-0").is_none());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("delete-session-confirmation").is_none());
    assert_eq!(cx.update(|_, cx| app.read(cx).delete_target), None);
    for confirm in [false, true] {
        let row = cx.debug_bounds("session-row-2").unwrap().center();
        cx.simulate_mouse_down(
            row,
            gpui_kit::MouseButton::Right,
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            row,
            gpui_kit::MouseButton::Right,
            gpui_kit::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let delete = cx.update(|window, _| {
            window
                .within("popup-menu")
                .find_by_label("删除")
                .bounds()
                .center()
        });
        cx.simulate_mouse_move(delete, None, gpui_kit::Modifiers::default());
        cx.simulate_mouse_down(
            delete,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            delete,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let selector = if confirm {
            "delete-confirm"
        } else {
            "delete-cancel"
        };
        let position = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_down(
            position,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            position,
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("delete-session-confirmation").is_none());
        assert_eq!(
            cx.update(|_, cx| app.read(cx).session(2).is_none()),
            confirm
        );
    }
}

#[gpui_kit::test]
fn historical_reply_elapsed_footer_is_available_on_hover(cx: &mut TestAppContext) {
    use gpui_kit::{AppContext as _, Modifiers};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut session = super::tests::session();
            let item = |kind, created_at, elapsed_ms, text: &str| ChatItem {
                key: format!("history-{created_at}"),
                created_at,
                elapsed_ms,
                kind,
                text: text.into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
                model: None,
            };
            session.model =
                Some("provider-with-a-very-long-model-name-that-must-not-overlap-actions".into());
            session.messages = vec![
                item(super::ItemKind::User, 100, None, "旧版问题"),
                item(super::ItemKind::Assistant, 112, None, "旧版回答"),
                item(super::ItemKind::User, 200, None, "导入问题"),
                item(super::ItemKind::Assistant, 207, Some(8250), "导入回答"),
            ];
            // Exercise a restored session without an ActiveRun.
            let json = serde_json::to_string(&session).unwrap();
            let restored = serde_json::from_str(&json).unwrap();
            let store = crate::store::Store {
                workspaces: vec![crate::domain::Workspace {
                    path: std::env::temp_dir().join("conduit-history-footer"),
                    sessions: vec![restored],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    // Include the real two-button case; no send/save actions are invoked.
    cx.update(|_, cx| {
        app.update(cx, |this, cx| {
            this.writable = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    for width in [1200., 850., 700.] {
        cx.simulate_resize(gpui_kit::size(px(width), px(900.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        for (message_selector, elapsed_selector) in [
            ("assistant-message-1", "reply-elapsed-1"),
            ("assistant-message-3", "reply-elapsed-3"),
        ] {
            let message = cx.debug_bounds(message_selector).unwrap();
            cx.simulate_mouse_move(message.center(), None, Modifiers::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                let _ = window.draw(cx);
            });
            let footer = cx.debug_bounds(elapsed_selector).unwrap();
            let (actions, controls, model) = if message_selector == "assistant-message-1" {
                ("reply-actions-1", "reply-controls-1", "reply-model-1")
            } else {
                ("reply-actions-3", "reply-controls-3", "reply-model-3")
            };
            let actions = cx.debug_bounds(actions).unwrap();
            let controls = cx.debug_bounds(controls).unwrap();
            let model = cx.debug_bounds(model).unwrap();
            assert_eq!(actions.left(), message.left());
            assert_eq!(
                controls.left(),
                message.left(),
                "buttons start at the prose's left edge"
            );
            let copy_selector = if message_selector == "assistant-message-1" {
                "reply-copy-button-1"
            } else {
                "reply-copy-button-3"
            };
            let copy_button = cx.debug_bounds(copy_selector).unwrap();
            assert_eq!(
                copy_button.size.width,
                px(24.),
                "keep the original hit target"
            );
            assert_eq!(
                copy_button.center().x - px(7.),
                message.left(),
                "the centered 14px copy glyph aligns with prose"
            );
            assert!(
                model.left() >= controls.right(),
                "metadata follows the buttons"
            );
            assert!(footer.size.width > px(0.));
            assert!(footer.size.height > px(0.));
            assert!(footer.top() >= message.top());
            assert!(footer.bottom() <= message.bottom());
            let timestamp_selector = if message_selector == "assistant-message-1" {
                "reply-timestamp-1"
            } else {
                "reply-timestamp-3"
            };
            let timestamp = cx.debug_bounds(timestamp_selector).unwrap();
            let mut parts = vec![copy_button, model, timestamp, footer];
            if message_selector == "assistant-message-3" {
                let retry = cx
                    .debug_bounds("reply-retry-button-3")
                    .expect("latest reply offers retry");
                assert!(retry.left() >= copy_button.right());
                assert!(retry.right() <= controls.right());
                parts.push(retry);
            }
            for (index, left) in parts.iter().enumerate() {
                for right in &parts[index + 1..] {
                    let overlaps = left.left() < right.right()
                        && left.right() > right.left()
                        && left.top() < right.bottom()
                        && left.bottom() > right.top();
                    assert!(
                        !overlaps,
                        "footer parts overlap at width {width}: {left:?} / {right:?}"
                    );
                }
            }
        }
    }
    cx.update(|_, cx| app.update(cx, |this, _| this.writable = false));
}

#[gpui_kit::test]
fn tabs_share_running_and_unread_dots_and_clear_on_open(cx: &mut TestAppContext) {
    use crate::protocol::Event;
    use gpui_kit::{AppContext as _, Modifiers, MouseButton};
    cx.update(gpui_kit::init);
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let output = slot.clone();
    let (runtime, sender) = crate::chat_runtime::ChatRuntime::test_channel();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let first = super::tests::session();
            let mut second = first.clone();
            second.id = 2;
            second.title = "后台会话".into();
            let path = std::env::temp_dir().join("conduit-tab-dots");
            let store = crate::store::Store {
                workspaces: vec![crate::domain::Workspace {
                    path: path.clone(),
                    sessions: vec![first, second],
                }],
                ..Default::default()
            };
            let mut app = super::ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.writable = false;
            app.selected_session = Some(1);
            app.open_tabs = vec![1, 2];
            app.runs.insert(
                2,
                super::ActiveRun::new(
                    runtime,
                    path,
                    "turn".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let dot = cx.debug_bounds("tab-activity-2-running").unwrap();
    let close = cx.debug_bounds("session-tab-close-2").unwrap();
    assert!(dot.right() <= close.left());
    assert!(cx.debug_bounds("session-activity-2-running").is_some());
    sender
        .send(Event::Finished {
            error: None,
            cancelled: false,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |this, cx| this.poll(window, cx)));
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert!(cx.debug_bounds("tab-activity-2-running").is_none());
    assert!(cx.debug_bounds("tab-activity-2-unread").is_some());
    assert!(cx.debug_bounds("session-activity-2-unread").is_some());
    let tab = cx.debug_bounds("session-tab-2").unwrap();
    let point = gpui_kit::point(tab.left() + px(15.), tab.center().y);
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
    assert_eq!(cx.update(|_, cx| app.read(cx).selected_session), Some(2));
    assert!(cx.debug_bounds("tab-activity-2-unread").is_none());
    assert!(cx.debug_bounds("session-activity-2-unread").is_none());
}
