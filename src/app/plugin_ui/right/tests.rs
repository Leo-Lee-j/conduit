use super::super::{
    PluginUi,
    tests::{ElementBounds as _, click, click_labeled, draw, fixture as navigation_fixture, store},
};
use super::{RightAdapter, RightBody};
use crate::app::{ConduitApp, Store};
use conduit::plugins::{
    builtins,
    context::{Scope, ScopeKey, When},
    contributions::{ContributionSet, Location, ViewDescriptor},
    ids::{Owner, PluginId, ViewId},
    registry::{HostCatalog, Registry, RendererSpec},
};
use gpui_kit::{
    Entity, InteractiveElement, IntoElement, ParentElement, Role, SharedString, TestAppContext,
    TestSupportExt, VisualTestContext, assets::IconName, div, point, px,
};

fn tool_store() -> Store {
    let mut store = store();
    let mut third = crate::app::tests::session();
    third.id = 3;
    store.workspaces[0].sessions.push(third);
    for workspace in &mut store.workspaces {
        for session in &mut workspace.sessions {
            session.messages = vec![serde_json::from_value(serde_json::json!({
                "key": "shared", "kind": "Tool", "text": "Bash",
                "input": format!("session {} input", session.id),
                "detail": (0..150).map(|i| format!("session {} output {i}\n", session.id)).collect::<String>(),
                "status": "完成"
            })).unwrap()];
        }
    }
    store
}

fn fixture(
    cx: &mut TestAppContext,
    store: Store,
    plugins: PluginUi,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    let (app, cx) = navigation_fixture(cx, store, plugins);
    // Startup sorts sessions; these cases explicitly exercise session 1 first.
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(1), window, cx)));
    draw(cx);
    (app, cx)
}

fn open(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.open_right_view(&ViewId::new(id).unwrap(), window, cx));
        })
    });
    draw(cx);
}

fn choose_tool(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, session: u64) {
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.open_tool_details(session, "shared", window, cx);
        })
    });
    draw(cx);
}

fn copied(cx: &mut VisualTestContext) -> String {
    click(cx, "copy-details");
    cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap())
}

#[gpui_kit::test]
fn session_data_opens_on_session_entry_and_respects_manual_changes(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let mut store = tool_store();
    store
        .usage_ledger
        .observations
        .push(crate::usage_ledger::Observation {
            key: "session-data-test".into(),
            session: 1,
            workspace: store.workspaces[0].path.clone(),
            provider: crate::app::Provider::Claude,
            service: "cli".into(),
            service_name: None,
            model: None,
            finished_at: 1,
            input: Some(1_000),
            cached: Some(800),
            output: Some(100),
            elapsed_ms: 1_000,
            incomplete: false,
            api_first_response_ms: Some(1_000),
            api_request_outcome: Some(crate::usage_ledger::ApiRequestOutcome::Succeeded),
        });
    let (app, cx) = fixture(cx, store, PluginUi::new());
    cx.simulate_resize(gpui_kit::size(px(1100.), px(1600.)));
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_some());
    assert!(
        cx.labeled_bounds(Role::Group, "API 连接质量 · 连接顺畅")
            .is_some()
    );
    let quality_card = cx.element_bounds("session-data-api-quality-card").unwrap();
    let quality_chart = cx.element_bounds("session-data-api-quality-chart").unwrap();
    assert!(quality_chart.size.height >= quality_card.size.height - px(2.));
    let quality_bar = cx.element_bounds("session-data-api-quality-bar-0").unwrap();
    assert!(quality_bar.size.height > px(30.));
    assert!(quality_bar.origin.y >= quality_card.origin.y);
    assert!(quality_bar.bottom() <= quality_card.bottom());
    assert!(
        cx.labeled_bounds(Role::Group, "连接顺畅 · 首响应：1.0s")
            .is_some()
    );
    assert!(cx.element_bounds("session-data-usage-summary").is_some());
    assert!(cx.element_bounds("session-data-token-details").is_some());
    click_labeled(cx, Role::Button, "Token 明细");
    draw(cx);
    assert!(cx.element_bounds("session-data-token-details").is_none());
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_usage(window, cx)));
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_document(crate::app::files::DocumentTab::Chat(1), window, cx);
        })
    });
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_some());
    click(cx, "close-session-data");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            // Refreshes, document changes, and readonly changes stay in the same session.
            app.writable = !app.writable;
            app.sync_plugin_context();
            assert!(!app.plugins.right_open());
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_none());
    cx.update(|window, cx| app.update(cx, |app, cx| app.open_usage(window, cx)));
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.select_document(crate::app::files::DocumentTab::Chat(1), window, cx);
        })
    });
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_none());
    choose_tool(&app, cx, 1);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = !app.writable;
            app.sync_plugin_context();
            assert!(app.plugins.right_is(builtins::TOOL_DETAILS));
            cx.notify();
        })
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_some());
    assert!(cx.element_bounds("session-data-token-details").is_some());
    cx.update(|_, cx| assert!(app.read(cx).plugins.right_is(builtins::SESSION_DATA)));
    assert!(cx.element_bounds("session-data-empty-state").is_some());
    assert!(
        cx.labeled_bounds(Role::Group, "API 连接质量 · 暂无请求数据")
            .is_some()
    );
    assert!(cx.element_bounds("session-data-usage-summary").is_none());
    assert!(
        cx.element_bounds("session-data-api-quality-bar-0")
            .is_none()
    );
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(1), window, cx)));
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_some());
    assert!(cx.element_bounds("session-data-token-details").is_none());
}

#[gpui_kit::test]
fn session_data_preserves_agent_history_statuses_and_session_isolation(cx: &mut TestAppContext) {
    use crate::subagents::{Agent, Status};
    use gpui_kit::base::test_support::find;

    let _locale = crate::test_support::chinese();
    let mut store = store();
    let agents: Vec<_> = [
        Status::Completed,
        Status::Failed,
        Status::Stopped,
        Status::Idle,
        Status::Unknown,
        Status::Completed,
        Status::Failed,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, status)| Agent {
        id: format!("agent-{index}"),
        run: format!("history-{}", index / 4),
        name: format!("历史子代理 {index}"),
        status,
        ..Default::default()
    })
    .collect();
    store.workspaces[0].sessions[0].subagents = agents.clone();
    let (app, cx) = fixture(cx, store, PluginUi::new());
    cx.simulate_resize(gpui_kit::size(px(1100.), px(1600.)));
    draw(cx);

    // Imported history can contain agents without tool messages or usage records.
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(app.session_control.runs.is_empty());
        assert!(app.session(1).unwrap().messages.is_empty());
        assert!(app.store.usage_ledger.observations.is_empty());
    });
    let assert_agents = |cx: &mut VisualTestContext, present: bool| {
        for agent in &agents {
            let id = SharedString::from(format!("session-data-agent-{}-{}", agent.run, agent.id));
            let element = cx.update(|window, _| find(window, &[], &id.into()));
            if present {
                let element = element.expect("every recorded agent must remain inspectable");
                assert!(element.visible());
                assert!(element.label().unwrap().contains(agent.status.label()));
            } else {
                assert!(
                    element.is_none(),
                    "agents must stay scoped to their session"
                );
            }
        }
    };
    assert_agents(cx, true);
    click_labeled(cx, Role::Button, "工具与子代理");
    assert_agents(cx, false);
    click_labeled(cx, Role::Button, "工具与子代理");
    assert_agents(cx, true);

    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
    draw(cx);
    assert_agents(cx, false);
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(1), window, cx)));
    draw(cx);
    assert_agents(cx, true);
}

#[gpui_kit::test]
fn session_data_plan_stays_visible_without_usage_or_timing_details(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    cx.simulate_resize(gpui_kit::size(px(1100.), px(1600.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_control.plans.insert(
                1,
                vec![
                    crate::app::state::PlanStep {
                        text: "读取 CLI 配置".into(),
                        status: crate::app::state::PlanStatus::Completed,
                    },
                    crate::app::state::PlanStep {
                        text: "更新会话数据面板".into(),
                        status: crate::app::state::PlanStatus::InProgress,
                    },
                ],
            );
            cx.notify();
        })
    });
    draw(cx);

    assert!(cx.element_bounds("session-data-plan-list").is_some());
    assert!(cx.element_bounds("session-data-empty-state").is_some());
    click_labeled(cx, Role::Button, "耗时与轮次");
    assert!(cx.element_bounds("session-data-plan-list").is_some());
    assert!(cx.element_bounds("session-data-timing-toggle").is_some());
}

#[gpui_kit::test]
fn tool_state_isolated_across_sessions_workspaces_and_stale_clicks(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    choose_tool(&app, cx, 1);
    let scroll = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let session = app.plugins.session_key().unwrap();
            let scroll = app.tool_details.scroll(&session, "shared");
            scroll.set_offset(point(px(0.), px(-150.)));
            scroll
        })
    });
    for (workspace, session) in [(0, 3), (1, 2)] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.switch(workspace, Some(session), window, cx)
            })
        });
        draw(cx);
        assert!(cx.element_bounds("session-data-panel").is_some());
        // A click queued from the old chat cannot select the new session's same message key.
        choose_tool(&app, cx, 1);
        assert!(cx.element_bounds("session-data-panel").is_some());
        choose_tool(&app, cx, session);
        assert!(copied(cx).contains(&format!("session {session} output")));
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                let key = app.plugins.session_key().unwrap();
                assert_eq!(app.tool_details.scroll(&key, "shared").offset().y, px(0.));
            })
        });
    }
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(0, Some(1), window, cx)));
    draw(cx);
    assert_eq!(scroll.offset().y, px(-150.));
    choose_tool(&app, cx, 1);
    assert!(copied(cx).contains("session 1 output"));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let key = app.plugins.right_key().unwrap().clone();
            let version = app.plugins.snapshot.version();
            let source = app.store.workspaces[0].path.clone();
            let target = app.store.workspaces[1].path.clone();
            app.reorder_workspace(&source, &target, cx);
            app.sync_plugin_context();
            assert_eq!(app.plugins.right_key(), Some(&key));
            assert_eq!(app.plugins.snapshot.version(), version);
        })
    });
    draw(cx);
    assert!(copied(cx).contains("session 1 output"));
}

fn plugins() -> PluginUi {
    let mut sets = vec![builtins::core().unwrap()];
    for name in ["alpha", "beta", "missing"] {
        let owner = format!("test.{name}");
        let mut set =
            ContributionSet::empty(Owner::external(PluginId::new(&owner).unwrap()).unwrap());
        let id = format!("{owner}.view");
        set.views.push(ViewDescriptor {
            id: ViewId::new(&id).unwrap(),
            title: format!("View {name}"),
            location: Location::Right,
            scope: Scope::Session,
            renderer: id,
            when: When::default(),
        });
        sets.push(set);
    }
    let mut catalog = HostCatalog::default();
    for icon in ["folder", "file", "git-fork"] {
        catalog.allow_icon(icon).unwrap();
    }
    for set in &sets {
        for view in &set.views {
            catalog
                .allow_renderer(
                    &view.renderer,
                    RendererSpec {
                        owner: set.owner.id().clone(),
                        location: view.location,
                        scope: view.scope,
                    },
                )
                .unwrap();
        }
    }
    let mut registry = Registry::new(catalog);
    for set in sets {
        registry.register(set, Default::default()).unwrap();
    }
    let mut plugins = PluginUi::from_registry(registry);
    for name in ["alpha", "beta"] {
        plugins.right_renderers.insert(
            format!("test.{name}.view"),
            RightAdapter {
                render: |_, key, _, _| {
                    let ScopeKey::Session { id, .. } = &key.scope else {
                        panic!("session adapter")
                    };
                    let selector = format!("rendered-{}-{id}", key.view);
                    RightBody::Content(
                        div()
                            .id(SharedString::from(selector.clone()))
                            .test_support()
                            .debug_selector(move || selector.clone())
                            .child(key.view.to_string())
                            .into_any_element(),
                    )
                    .into()
                },
                icon: IconName::PanelRight,
                close_id: "close-test-view",
                with_branches: true,
                activate: |_, _, _| {},
                deactivate: |_, _, _| {},
            },
        );
    }
    plugins
}

#[gpui_kit::test]
fn rail_routes_two_owners_and_rejects_unbound_stale_revoked_views(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), plugins());
    click(cx, "right-choice-conduit.core.tool-details");
    click_labeled(cx, Role::Button, "View missing");
    cx.update(|_, cx| assert!(app.read(cx).plugins.right_is(builtins::TOOL_DETAILS)));
    click_labeled(cx, Role::Button, "View alpha");
    assert!(cx.element_bounds("rendered-test.alpha.view-1").is_some());
    click_labeled(cx, Role::Button, "View beta");
    assert!(cx.element_bounds("rendered-test.beta.view-1").is_some());
    click_labeled(cx, Role::Button, "View beta");
    assert!(cx.element_bounds("detail-panel").is_none());
    click_labeled(cx, Role::Button, "View beta");
    assert!(cx.element_bounds("rendered-test.beta.view-1").is_some());
    let request = cx.update(|_, cx| {
        let app = app.read(cx);
        app.plugins
            .registry
            .prepare_view(
                &ViewId::new("test.beta.view").unwrap(),
                &app.plugins.snapshot,
            )
            .unwrap()
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
    draw(cx);
    assert!(cx.element_bounds("session-data-panel").is_some());
    open(&app, cx, "test.beta.view");
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.dispatch_right_view(&request, window, cx));
            assert!(!app.toggle_right_view(&request, window, cx));
            assert!(app.plugins.right_is("test.beta.view"));
        })
    });
    let revoked = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let request = app
                .plugins
                .registry
                .prepare_view(
                    &ViewId::new("test.beta.view").unwrap(),
                    &app.plugins.snapshot,
                )
                .unwrap();
            let owner = PluginId::new("test.beta").unwrap();
            let registration = app.plugins.registry.registration(&owner).unwrap().clone();
            app.plugins.registry.unregister(&registration).unwrap();
            cx.notify();
            request
        })
    });
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.dispatch_right_view(&revoked, window, cx));
            assert!(!app.toggle_right_view(&revoked, window, cx));
            assert!(!app.plugins.right_open());
            assert_eq!(app.plugins.right().as_str(), builtins::TOOL_DETAILS);
        })
    });
    open(&app, cx, "test.alpha.view");
    assert!(cx.element_bounds("rendered-test.alpha.view-2").is_some());
    click(cx, "close-test-view");
    cx.update(|_, cx| assert_eq!(app.read(cx).plugins.right().as_str(), "test.alpha.view"));
}

#[gpui_kit::test]
fn scope_removal_closes_dock_even_when_host_selects_another_session(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), plugins());
    choose_tool(&app, cx, 1);
    let session = cx.update(|_, cx| app.read(cx).plugins.session_key().unwrap());
    cx.update(|_, cx| app.update(cx, |app, cx| app.delete_session(1, cx)));
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    cx.update(|_, cx| {
        assert!(app.read(cx).tool_details.selected(&session).is_none());
        assert_eq!(app.read(cx).selected_session, Some(3));
    });
    open(&app, cx, "test.alpha.view");
    cx.update(|window, cx| app.update(cx, |app, cx| app.clear_workspace_selection(window, cx)));
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).plugins.right().as_str(),
            builtins::TOOL_DETAILS
        )
    });
    click(cx, "notification-bell");
    assert!(cx.labeled_bounds(Role::Button, "工具详情").is_none());
    assert!(cx.labeled_bounds(Role::Button, "View alpha").is_none());
    assert!(cx.element_bounds("project-workflow").is_none());
    assert!(cx.element_bounds("project-automations").is_none());
    assert!(cx.element_bounds("notification-bell").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    click(cx, "notification-bell");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.store.workspaces.clear();
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.element_bounds("notification-panel").is_some());
}
