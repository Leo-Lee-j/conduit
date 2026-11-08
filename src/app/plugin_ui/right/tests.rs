use super::super::{
    PluginUi,
    tests::{ElementBounds as _, click, click_labeled, draw, fixture as navigation_fixture, store},
};
use super::{RightAdapter, RightBody};
use crate::{
    app::{ActiveRun, ConduitApp, Store},
    chat_runtime::ChatRuntime,
    protocol::Event,
};
use conduit::plugins::{
    builtins,
    context::{Scope, ScopeKey, When},
    contributions::{ContributionSet, Location, ViewDescriptor},
    ids::{Owner, PluginId, ViewId},
    registry::{HostCatalog, Registry, RendererSpec},
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    Entity, Focusable as _, InteractiveElement, IntoElement, Modifiers, MouseButton, ParentElement,
    Role, ScrollDelta, ScrollWheelEvent, SharedString, TestAppContext, TestSupportExt,
    VisualTestContext, assets::IconName, div, point, px, size,
};
use std::time::Duration;

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
fn tool_entry_tabs_copy_close_and_notification_focus(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    cx.simulate_resize(size(px(1400.), px(900.)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.input.read(cx).focus_handle(cx).focus(window, cx);
            app.expanded_tools.insert((1, "shared".into()));
            app.scroll.remeasure_items(0..1);
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "tool-1-shared-dock");
    assert!(cx.labeled_bounds(Role::Group, "工具调用详情").is_some());
    assert!(copied(cx).contains("session 1 output"));
    let scroll = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let session = app.plugins.session_key().unwrap();
            app.tool_details.scroll(&session, "shared")
        })
    });
    let body = cx.labeled_bounds(Role::Group, "工具调用详情").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: body.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
        ..Default::default()
    });
    draw(cx);
    let offset = scroll.offset();
    assert!(offset.y < px(0.));
    click(cx, "close-details");
    assert!(cx.element_bounds("detail-panel").is_none());
    click(cx, "toggle-sidebar");
    assert_eq!(scroll.offset(), offset);
    click_labeled(cx, Role::Tab, "通知");
    assert!(cx.element_bounds("notification-panel").is_some());
    assert!(cx.labeled_bounds(Role::Group, "工具调用详情").is_none());
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert!(window.focused(cx).is_some());
        assert_ne!(
            window.focused(cx),
            Some(app.input.read(cx).focus_handle(cx))
        );
    });
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    cx.update(|window, cx| {
        let app = app.read(cx);
        assert_eq!(
            window.focused(cx),
            Some(app.input.read(cx).focus_handle(cx))
        );
        assert_eq!(app.plugins.right().as_str(), builtins::NOTIFICATIONS);
    });
    click(cx, "toggle-sidebar");
    assert!(cx.element_bounds("notification-panel").is_some());
    click_labeled(cx, Role::Tab, "工具详情");
    assert_eq!(scroll.offset(), offset);
    assert!(copied(cx).contains("session 1 output"));
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
        assert!(cx.element_bounds("detail-empty").is_some());
        // A click queued from the old chat cannot select the new session's same message key.
        choose_tool(&app, cx, 1);
        assert!(cx.element_bounds("detail-empty").is_some());
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
fn generic_tabs_route_two_owners_and_reject_unbound_stale_revoked_views(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), plugins());
    open(&app, cx, builtins::TOOL_DETAILS);
    click_labeled(cx, Role::Tab, "View missing");
    cx.update(|_, cx| assert!(app.read(cx).plugins.right_is(builtins::TOOL_DETAILS)));
    click_labeled(cx, Role::Tab, "View alpha");
    assert!(cx.element_bounds("rendered-test.alpha.view-1").is_some());
    click_labeled(cx, Role::Tab, "View beta");
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
    assert!(cx.element_bounds("rendered-test.beta.view-2").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.dispatch_right_view(&request, window, cx));
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
    assert!(cx.labeled_bounds(Role::Tab, "工具详情").is_none());
    assert!(cx.labeled_bounds(Role::Tab, "View alpha").is_none());
    assert!(cx.labeled_bounds(Role::Tab, "通知").is_some());
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    click(cx, "toggle-sidebar");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.store.workspaces.clear();
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.element_bounds("notification-panel").is_some());
}

#[gpui_kit::test]
fn host_placeholders_cover_empty_loading_failure_and_missing_adapter(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    open(&app, cx, builtins::TOOL_DETAILS);
    assert!(cx.element_bounds("detail-empty").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().transcript.loaded = false;
            let key = app.plugins.right_key().unwrap().clone();
            // Inspect before the shell schedules the in-memory fixture's immediate load.
            assert!(matches!(
                app.render_tool_details(&key, window, cx).body,
                RightBody::Loading
            ));
        })
    });
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.transcripts
                .errors
                .insert(1, "test transcript failure".into());
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.element_bounds("right-view-error").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().transcript.loaded = true;
            app.transcripts.errors.remove(&1);
            cx.notify();
        })
    });
    choose_tool(&app, cx, 1);
    assert!(cx.labeled_bounds(Role::Group, "工具调用详情").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.plugins.right_renderers.remove(builtins::TOOL_DETAILS);
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.element_bounds("right-view-error").is_some());
    click(cx, "close-right-view");
    assert!(cx.element_bounds("detail-panel").is_none());
}

#[gpui_kit::test]
fn background_tool_and_notification_events_preserve_active_view_and_focus(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    choose_tool(&app, cx, 1);
    open(&app, cx, builtins::NOTIFICATIONS);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let focus = window.focused(cx);
            let key = app.plugins.right_key().cloned();
            let (runtime, sender) = ChatRuntime::test_channel();
            let path = app.store.workspaces[0].path.clone();
            app.session_control.runs.insert(
                1,
                ActiveRun::new(
                    runtime,
                    path,
                    "background".into(),
                    Default::default(),
                    None,
                    Default::default(),
                ),
            );
            sender
                .send(Event::Tool {
                    key: "late".into(),
                    title: "Bash".into(),
                    detail: "background output".into(),
                    status: "completed".into(),
                })
                .unwrap();
            app.poll_streams(window, cx);
            assert!(
                app.session(1)
                    .unwrap()
                    .messages
                    .iter()
                    .any(|m| m.detail == "background output")
            );
            app.notify_user(
                crate::notifications::Notice::new(
                    crate::notifications::Source::App,
                    crate::notifications::Level::Info,
                    "后台通知",
                    "完成",
                ),
                cx,
            );
            app.poll_notifications(window, cx);
            assert_eq!(app.plugins.right_key(), key.as_ref());
            assert_eq!(window.focused(cx), focus);
            sender
                .send(Event::Finished {
                    error: None,
                    cancelled: false,
                })
                .unwrap();
            app.poll_streams(window, cx);
            assert_eq!(app.plugins.right_key(), key.as_ref());
            assert_eq!(window.focused(cx), focus);
        })
    });
    draw(cx);
    assert!(cx.element_bounds("notification-panel").is_some());
    // Toasts occlude the tool panel. Let their lifetime and exit animation finish
    // before testing the copy button underneath them with a real mouse click.
    for _ in 0..9 {
        cx.executor().advance_clock(Duration::from_secs(1));
        draw(cx);
    }
    cx.update(|window, cx| assert!(window.notifications(cx).is_empty()));
    click_labeled(cx, Role::Tab, "工具详情");
    assert!(copied(cx).contains("session 1 output"));
}

#[gpui_kit::test]
fn right_width_drag_and_view_switch_preserve_layout(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    cx.simulate_resize(size(px(1600.), px(900.)));
    choose_tool(&app, cx, 1);
    let width = cx.element_bounds("detail-panel").unwrap().size.width;
    let band = cx
        .element_bounds("detail-resize")
        .expect("right resize handle");
    let start = band.center();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(start.x - px(10.), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    cx.simulate_mouse_move(
        point(start.x - px(80.), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    cx.simulate_mouse_up(
        point(start.x - px(80.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    let resized = cx.element_bounds("detail-panel").unwrap().size.width;
    assert!(resized > width + px(50.));
    open(&app, cx, builtins::NOTIFICATIONS);
    assert_eq!(
        cx.element_bounds("detail-panel").unwrap().size.width,
        resized
    );
    click(cx, "close-notifications");
    open(&app, cx, builtins::TOOL_DETAILS);
    assert_eq!(
        cx.element_bounds("detail-panel").unwrap().size.width,
        resized
    );
    cx.simulate_resize(size(px(850.), px(500.)));
    draw(cx);
    let panel = cx.element_bounds("detail-panel").unwrap();
    assert!(panel.right() <= px(850.) && panel.left() >= px(38.));
    assert_eq!(
        cx.element_bounds("detail-header").unwrap().size.height,
        cx.element_bounds("document-tab-strip").unwrap().size.height
    );
}

#[gpui_kit::test]
fn branch_navigation_only_offers_visible_views_and_rejects_hidden_requests(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, tool_store(), PluginUi::new());
    choose_tool(&app, cx, 1);
    let request = cx.update(|_, cx| {
        let app = app.read(cx);
        app.plugins
            .registry
            .prepare_view(
                &ViewId::new(builtins::TOOL_DETAILS).unwrap(),
                &app.plugins.snapshot,
            )
            .unwrap()
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.show_branch_navigation(window, cx);
            assert!(!app.dispatch_right_view(&request, window, cx));
        })
    });
    draw(cx);
    assert!(cx.element_bounds("detail-panel").is_none());
    click(cx, "notification-bell");
    assert!(cx.element_bounds("notification-panel").is_some());
    assert!(cx.labeled_bounds(Role::Tab, "工具详情").is_none());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.leave_branch_navigation();
            cx.notify();
        })
    });
    draw(cx);
    click_labeled(cx, Role::Tab, "工具详情");
    assert!(copied(cx).contains("session 1 output"));
}
