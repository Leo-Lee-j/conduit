use super::bind;
use crate::app::plugin_ui::{
    PluginUi,
    tests::{ElementBounds as _, click, draw, fixture, store},
};
use conduit::plugins::{
    builtins,
    commands::Capability,
    ids::{ItemId, PluginId},
};
use gpui_kit::{
    Role, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px, size,
};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

fn plugins() -> PluginUi {
    PluginUi::from_registry(builtins::register(true).unwrap().registry)
}

#[test]
fn default_registration_follows_explicit_feature() {
    let _locale = crate::test_support::chinese();
    let ui = PluginUi::new();
    assert_eq!(
        ui.registry
            .registration(&PluginId::new(builtins::EXAMPLE).unwrap())
            .is_some(),
        cfg!(feature = "plugin-example")
    );
}

#[gpui_kit::test]
fn three_surfaces_share_commands_and_keep_independent_workspace_scrolls(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let mut data = store();
    for (i, workspace) in data.workspaces.iter_mut().enumerate() {
        workspace.sessions = (0..40)
            .map(|j| {
                let mut session = crate::app::tests::session();
                session.id = (i * 100 + j + 1) as u64;
                session.title = format!("Workspace {i} session {j}");
                session
            })
            .collect();
    }
    let (app, cx) = fixture(cx, data, plugins());
    cx.simulate_resize(size(px(1600.), px(700.)));
    draw(cx);
    click(cx, builtins::EXAMPLE_ACTIVITY);
    assert!(cx.labeled_bounds(Role::Group, "工作区概览").is_some());
    click(cx, "overview-show-summary");
    draw(cx);
    assert!(cx.labeled_bounds(Role::Group, "工作区摘要").is_some());
    assert!(cx.element_bounds(builtins::EXAMPLE_STATUS).is_some());
    let (navigation, summary, navigation_key, summary_key) = cx.update(|_, cx| {
        let app = app.read(cx);
        let nav = app.plugins.state.navigation_key().unwrap().clone();
        let right = app.plugins.right_key().unwrap().clone();
        assert_ne!(nav, right);
        assert_eq!(nav.scope, right.scope);
        (
            app.plugins
                .instances
                .get(&nav)
                .unwrap()
                .view
                .read(cx)
                .scroll
                .clone(),
            app.plugins
                .instances
                .get(&right)
                .unwrap()
                .view
                .read(cx)
                .scroll
                .clone(),
            nav,
            right,
        )
    });
    for (id, delta) in [("工作区概览", -140.), ("工作区摘要", -220.)] {
        let body = cx.labeled_bounds(Role::Group, id).unwrap();
        cx.simulate_event(ScrollWheelEvent {
            position: body.center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
            ..Default::default()
        });
        draw(cx);
    }
    let offsets = (navigation.offset(), summary.offset());
    assert!(
        offsets.0.y < px(0.) && offsets.1.y < offsets.0.y,
        "independent scroll offsets: {offsets:?}"
    );
    click(cx, builtins::EXAMPLE_ACTIVITY);
    click(cx, "close-workspace-overview");
    click(cx, builtins::EXAMPLE_STATUS);
    assert_eq!((navigation.offset(), summary.offset()), offsets);
    // A read-only native query remains usable, and it does not rebuild either instance.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            cx.notify();
        })
    });
    draw(cx);
    click(cx, "close-workspace-overview");
    click(cx, builtins::EXAMPLE_STATUS);
    assert!(cx.labeled_bounds(Role::Group, "工作区摘要").is_some());
    for (workspace, session) in [(1, 101), (0, 1)] {
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.switch(workspace, Some(session), window, cx)
            })
        });
        draw(cx);
        cx.update(|_, cx| {
            let app = app.read(cx);
            let nav = app.plugins.state.navigation_key().unwrap();
            let right = app.plugins.right_key().unwrap();
            if workspace == 1 {
                assert_ne!(nav, &navigation_key);
                assert_ne!(right, &summary_key);
                assert_eq!(
                    app.plugins
                        .instances
                        .get(nav)
                        .unwrap()
                        .view
                        .read(cx)
                        .scroll
                        .offset()
                        .y,
                    px(0.)
                );
                assert_eq!(
                    app.plugins
                        .instances
                        .get(right)
                        .unwrap()
                        .view
                        .read(cx)
                        .scroll
                        .offset()
                        .y,
                    px(0.)
                );
            } else {
                assert_eq!(nav, &navigation_key);
                assert_eq!(right, &summary_key);
            }
        });
    }
    assert_eq!((navigation.offset(), summary.offset()), offsets);
}

#[gpui_kit::test]
fn unregister_clears_three_surfaces_and_rebinding_cannot_revive_old_clicks(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, store(), plugins());
    click(cx, builtins::EXAMPLE_ACTIVITY);
    click(cx, "overview-show-summary");
    let saved = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let saved = app
                .plugins
                .registry
                .prepare_status_command(
                    &ItemId::new(builtins::EXAMPLE_STATUS).unwrap(),
                    &app.plugins.snapshot,
                )
                .unwrap();
            let owner = PluginId::new(builtins::EXAMPLE).unwrap();
            let registration = app.plugins.registry.registration(&owner).unwrap().clone();
            app.plugins.registry.unregister(&registration).unwrap();
            cx.notify();
            saved
        })
    });
    draw(cx);
    for id in [builtins::EXAMPLE_ACTIVITY, builtins::EXAMPLE_STATUS] {
        assert!(cx.element_bounds(id).is_none(), "remaining {id}");
    }
    for label in ["工作区概览", "工作区摘要"] {
        assert!(
            cx.labeled_bounds(Role::Group, label).is_none(),
            "remaining {label}"
        );
    }
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert!(app.plugins.is_selected(builtins::WORKSPACES));
            assert!(!app.plugins.right_open());
            assert!(app.plugins.instances.is_empty());
            assert!(!app.dispatch_plugin_command(&saved, window, cx));
            app.plugins
                .registry
                .register(
                    builtins::workspace_overview().unwrap(),
                    BTreeSet::from([Capability::WorkspaceSummary]),
                )
                .unwrap();
            app.sync_plugin_context();
            assert!(
                app.plugins
                    .host_command(builtins::EXAMPLE_COMMAND)
                    .is_none()
            );
            bind(&mut app.plugins);
            assert!(!app.dispatch_plugin_command(&saved, window, cx));
            cx.notify();
        })
    });
    draw(cx);
    click(cx, builtins::EXAMPLE_STATUS);
    assert!(cx.labeled_bounds(Role::Group, "工作区摘要").is_some());
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.store.workspaces.remove(0);
            app.selected_workspace = Some(0);
            app.selected_session = Some(2);
            cx.notify();
        })
    });
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.plugins.right_open());
        assert!(app.plugins.instances.is_empty());
    });
}

fn settle_summary(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.background_executor
        .advance_clock(Duration::from_millis(60));
    draw(cx);
}

#[gpui_kit::test]
fn metadata_coalesces_without_chat_token_renders_and_hidden_views_pause(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let _clock = crate::app::events::test_clock::Clock::freeze(1_800_000_000);
    let (app, cx) = fixture(cx, store(), plugins());
    cx.simulate_resize(size(px(1600.), px(700.)));
    draw(cx);
    click(cx, builtins::EXAMPLE_ACTIVITY);
    click(cx, "overview-show-summary");
    settle_summary(cx);
    let (key, right, index_view, right_view, renders, sidebar_renders, version) =
        cx.update(|_, cx| {
            let app = app.read(cx);
            let key = app.plugins.state.navigation_key().unwrap().clone();
            let right = app.plugins.right_key().unwrap().clone();
            let index = app.plugins.instances.get(&key).unwrap().view.clone();
            let summary = app.plugins.instances.get(&right).unwrap().view.clone();
            let renders = (index.read(cx).renders, summary.read(cx).renders);
            (
                key,
                right,
                index,
                summary,
                renders,
                app.sidebar_view.read(cx).render_count,
                app.plugins.snapshot.version(),
            )
        });
    for _ in 0..5 {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                crate::app::apply_event(
                    app.session_mut(1).unwrap(),
                    "overview-turn",
                    crate::app::Event::Delta {
                        key: "reply".into(),
                        text: " more body".into(),
                    },
                );
                cx.notify();
            })
        });
        draw(cx);
    }
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            (index_view.read(cx).renders, right_view.read(cx).renders),
            renders
        );
        assert_eq!(app.sidebar_view.read(cx).render_count, sidebar_renders);
        assert!(!app.plugins.instances.has_pending(&key));
        assert!(!app.plugins.instances.has_pending(&right));
    });
    // Several metadata updates before the timer completes produce one publication.
    for title in ["rename one", "rename two", "latest title"] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.session_mut(1).unwrap().title = title.into();
                cx.notify();
            })
        });
        draw(cx);
    }
    cx.update(|_, cx| assert_ne!(index_view.read(cx).data.sessions[0].title, "latest title"));
    settle_summary(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(index_view.read(cx).data.sessions[0].title, "latest title");
        assert_eq!(right_view.read(cx).data.sessions[0].title, "latest title");
        assert_eq!(
            (index_view.read(cx).renders, right_view.read(cx).renders),
            (renders.0 + 1, renders.1 + 1)
        );
        assert_eq!(app.plugins.snapshot.version(), version);
    });
    click(cx, "close-workspace-overview");
    click(cx, "nav-collapse");
    let hidden_renders =
        cx.update(|_, cx| (index_view.read(cx).renders, right_view.read(cx).renders));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.session_mut(1).unwrap().title = "changed while hidden".into();
            cx.notify();
        })
    });
    settle_summary(cx);
    cx.update(|_, cx| {
        assert_eq!(
            (index_view.read(cx).renders, right_view.read(cx).renders),
            hidden_renders
        );
        assert_eq!(index_view.read(cx).data.sessions[0].title, "latest title");
        let app = app.read(cx);
        assert!(!app.plugins.instances.has_pending(&key));
        assert!(!app.plugins.instances.has_pending(&right));
    });
    click(cx, "nav-collapse");
    click(cx, builtins::EXAMPLE_STATUS);
    settle_summary(cx);
    cx.update(|_, cx| {
        assert_eq!(
            index_view.read(cx).data.sessions[0].title,
            "changed while hidden"
        );
        assert_eq!(
            right_view.read(cx).data.sessions[0].title,
            "changed while hidden"
        );
    });
}

#[gpui_kit::test]
fn late_results_after_switch_hide_readonly_and_close_cannot_publish(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, store(), plugins());
    click(cx, builtins::EXAMPLE_ACTIVITY);
    // Deliberately invoke saved completions even after cancellation, like a worker
    // that finished just before its delivery task was dropped.
    for action in 0..5 {
        let (ticket, data, old_view) = cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.writable = true;
                app.closing = false;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.sync_plugin_context();
                app.plugins
                    .select(&conduit::plugins::ids::ViewId::new(builtins::EXAMPLE_INDEX).unwrap())
                    .unwrap();
                let key = app.plugins.state.navigation_key().unwrap().clone();
                app.render_overview(&key, false, cx);
                let data = Arc::new(app.overview_data(&key).unwrap());
                let ticket = app
                    .plugins
                    .instances
                    .begin(&app.plugins.registry, &app.plugins.snapshot, &key)
                    .unwrap();
                let view = app.plugins.instances.get(&key).unwrap().view.clone();
                (ticket, data, view)
            })
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                match action {
                    0 => {
                        app.selected_workspace = Some(1);
                        app.selected_session = Some(2);
                    }
                    1 => app.plugins.set_collapsed(true),
                    2 => app.writable = false,
                    3 => app.closing = true,
                    _ => app.active_file = Some(123),
                }
                app.sync_plugin_context();
                assert!(ticket.is_cancelled());
                assert!(!app.complete_overview(&ticket, data.clone(), cx));
                assert_eq!(*old_view.read(cx).data, *data);
                if action == 3 {
                    assert!(app.plugins.instances.is_empty());
                }
                app.active_file = None;
            })
        });
    }
}

#[gpui_kit::test]
fn immediate_rebind_deletion_and_exit_release_entities_and_tasks(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, store(), plugins());
    click(cx, builtins::EXAMPLE_ACTIVITY);
    let (old, ticket, data) = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let key = app.plugins.state.navigation_key().unwrap().clone();
            app.session_mut(1).unwrap().title = "pending".into();
            app.refresh_plugin_views(cx);
            assert!(app.plugins.instances.has_pending(&key));
            let old = app.plugins.instances.get(&key).unwrap().view.downgrade();
            let ticket = app
                .plugins
                .instances
                .begin(&app.plugins.registry, &app.plugins.snapshot, &key)
                .unwrap();
            let data = Arc::new(app.overview_data(&key).unwrap());
            let registration = app
                .plugins
                .registry
                .registration(&key.owner)
                .unwrap()
                .clone();
            app.plugins.registry.unregister(&registration).unwrap();
            app.plugins
                .registry
                .register(
                    builtins::workspace_overview().unwrap(),
                    BTreeSet::from([Capability::WorkspaceSummary]),
                )
                .unwrap();
            // Without a render between unregister/register, old bindings are unusable.
            assert!(app.plugins.adapter(&key.view).is_none());
            bind(&mut app.plugins);
            assert!(ticket.is_cancelled());
            assert!(app.plugins.instances.is_empty());
            assert!(!app.complete_overview(&ticket, data.clone(), cx));
            cx.notify();
            (old, ticket, data)
        })
    });
    draw(cx);
    draw(cx);
    assert!(
        old.upgrade().is_none(),
        "unregistered Entity must be released after frame cleanup"
    );
    click(cx, builtins::EXAMPLE_ACTIVITY);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            assert!(!app.complete_overview(&ticket, data, cx));
            let key = app.plugins.state.navigation_key().unwrap().clone();
            let pending = app
                .plugins
                .instances
                .begin(&app.plugins.registry, &app.plugins.snapshot, &key)
                .unwrap();
            app.store.workspaces.remove(0);
            app.selected_workspace = Some(0);
            app.selected_session = Some(2);
            app.sync_plugin_context();
            assert!(pending.is_cancelled());
            assert!(app.plugins.instances.is_empty());
            app.plugins
                .select(&conduit::plugins::ids::ViewId::new(builtins::EXAMPLE_INDEX).unwrap())
                .unwrap();
            let key = app.plugins.state.navigation_key().unwrap().clone();
            app.render_overview(&key, false, cx);
            let pending = app
                .plugins
                .instances
                .begin(&app.plugins.registry, &app.plugins.snapshot, &key)
                .unwrap();
            app.plugins.stop_instances();
            assert!(pending.is_cancelled());
            assert!(app.plugins.instances.is_empty());
            // The updater returns early while exiting; cleanup must run first.
            app.render_overview(&key, false, cx);
            app.closing = true;
            app.updater.exiting = true;
        })
    });
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.poll(window, cx);
            assert!(app.plugins.instances.is_empty());
        })
    });
}
