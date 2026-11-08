use super::{NavigationAdapter, PluginUi};
use crate::app::{ConduitApp, DocumentTab, Store, Workspace};
use conduit::plugins::{
    builtins,
    commands::{ArgumentPolicy, Capability, CommandDescriptor},
    context::{Scope, When},
    contributions::{ActivityAction, ActivityItem, ContributionSet, Location, ViewDescriptor},
    ids::{CommandId, ItemId, Owner, PluginId, ViewId},
    registry::{HostCatalog, Registry, RendererSpec},
};
use gpui_kit::{
    AppContext as _, Bounds, Entity, InteractiveElement, IntoElement, Modifiers, MouseButton,
    ParentElement, Pixels, Role, SharedString, TestAppContext, TestSupportExt, VisualTestContext,
    base::test_support, div, px,
};
use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

pub(super) fn fixture(
    cx: &mut TestAppContext,
    store: Store,
    plugins: PluginUi,
) -> (Entity<ConduitApp>, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let slot = Rc::new(RefCell::new(None));
    let output = slot.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
            app.startup = None;
            app.writable = true;
            app.selected_workspace = (!app.store.workspaces.is_empty()).then_some(0);
            app.selected_session = app
                .store
                .workspaces
                .first()
                .and_then(|w| w.sessions.first())
                .map(|s| s.id);
            app.open_tabs = app
                .selected_session
                .into_iter()
                .map(DocumentTab::Chat)
                .collect();
            app.plugins = plugins;
            app
        });
        *output.borrow_mut() = Some(app.clone());
        gpui_kit::component::Root::new(app, window, cx)
    });
    let app = slot.borrow().as_ref().unwrap().clone();
    cx.simulate_resize(gpui_kit::size(px(1100.), px(800.)));
    draw(cx);
    (app, cx)
}

pub(super) fn store() -> Store {
    Store {
        workspaces: (0..2)
            .map(|index| {
                let mut session = crate::app::tests::session();
                session.id = index + 1;
                Workspace {
                    path: std::env::temp_dir().join(format!("conduit-plugin-navigation-{index}")),
                    sessions: vec![session],
                }
            })
            .collect(),
        ..Default::default()
    }
}

// Two independent owners use the same generic adapter; neither needs a branch
// in the production navigation renderer. One renderer and one command are unbound.
fn plugins(count: usize) -> PluginUi {
    let mut extra =
        ContributionSet::empty(Owner::external(PluginId::new("test.navigation").unwrap()).unwrap());
    for index in 0..=count {
        let name = if index == count {
            "missing".into()
        } else {
            index.to_string()
        };
        let id = format!("test.navigation.view-{name}");
        extra.views.push(ViewDescriptor {
            id: ViewId::new(&id).unwrap(),
            title: format!("Navigation {name}"),
            location: Location::Navigation,
            scope: Scope::Workspace,
            renderer: id.clone(),
            when: When::default(),
        });
        extra.activity_items.push(ActivityItem {
            id: ItemId::new(format!("test.navigation.entry-{name}")).unwrap(),
            title: format!("Navigation {name}"),
            icon: "folder".into(),
            action: ActivityAction::OpenView(ViewId::new(id).unwrap()),
            order: 200 + index as i32,
            when: When::default(),
        });
    }
    let command = CommandId::new("test.navigation.command").unwrap();
    extra.commands.push(CommandDescriptor {
        id: command.clone(),
        title: "Unbound command".into(),
        scope: Scope::Global,
        arguments: ArgumentPolicy::None,
        required_capabilities: BTreeSet::new(),
        requires_writable: false,
    });
    extra.activity_items.push(ActivityItem {
        id: ItemId::new("test.navigation.command-entry").unwrap(),
        title: "Unbound command".into(),
        icon: "folder".into(),
        action: ActivityAction::ExecuteCommand(command),
        order: 300,
        when: When::default(),
    });
    let mut hidden = extra.activity_items[0].clone();
    hidden.id = ItemId::new("test.navigation.hidden").unwrap();
    hidden.when = When::parse(Some("session.running")).unwrap();
    extra.activity_items.push(hidden);
    let sets = [
        builtins::core().unwrap(),
        builtins::workspace_overview().unwrap(),
        extra,
    ];
    let mut catalog = HostCatalog::default();
    for icon in ["folder", "file", "git-fork", "layout-dashboard"] {
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
        registry
            .register(
                set,
                BTreeSet::from([Capability::SessionSummary, Capability::WorkspaceSummary]),
            )
            .unwrap();
    }
    let mut plugins = PluginUi::from_registry(registry);
    for id in std::iter::once(builtins::EXAMPLE_INDEX.to_owned())
        .chain((0..count).map(|i| format!("test.navigation.view-{i}")))
    {
        plugins.renderers.insert(
            id,
            NavigationAdapter {
                render: |app, _| {
                    div()
                        .id("plugin-navigation-page")
                        .test_support()
                        .debug_selector(|| "plugin-navigation-page".into())
                        .child(app.plugins.selected().to_string())
                        .into_any_element()
                },
                activate: |_, _| {},
                element_id: "",
                task_badge: false,
                refresh_on_notify: true,
            },
        );
    }
    plugins
}

pub(super) fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });
}

/// Query the same observed native elements in dev and release. Never fall back
/// to debug selectors: their absence in release would invalidate hidden checks.
pub(super) trait ElementBounds {
    fn element_bounds(&mut self, id: &str) -> Option<Bounds<Pixels>>;
    fn labeled_bounds(&mut self, role: Role, label: &str) -> Option<Bounds<Pixels>>;
}

impl ElementBounds for VisualTestContext {
    fn element_bounds(&mut self, id: &str) -> Option<Bounds<Pixels>> {
        self.update(|window, _| {
            test_support::find(window, &[], &SharedString::from(id.to_owned()).into())
                .filter(|element| element.visible())
                .map(|element| element.bounds())
        })
    }

    fn labeled_bounds(&mut self, role: Role, label: &str) -> Option<Bounds<Pixels>> {
        self.update(|window, _| {
            let matches: Vec<_> = test_support::snapshots(window)
                .into_iter()
                .filter(|element| {
                    element.visible()
                        && element.role() == Some(role)
                        && element.label() == Some(label)
                })
                .collect();
            assert!(
                matches.len() <= 1,
                "ambiguous {role:?} labeled {label:?}: {matches:?}"
            );
            matches.first().map(|element| element.bounds())
        })
    }
}

pub(super) fn click(cx: &mut VisualTestContext, id: &str) {
    let bounds = cx
        .element_bounds(id)
        .unwrap_or_else(|| panic!("missing {id}"));
    click_bounds(cx, bounds);
}

pub(super) fn click_labeled(cx: &mut VisualTestContext, role: Role, label: &str) {
    let bounds = cx
        .labeled_bounds(role, label)
        .unwrap_or_else(|| panic!("missing {role:?} labeled {label:?}"));
    click_bounds(cx, bounds);
}

fn click_bounds(cx: &mut VisualTestContext, bounds: Bounds<Pixels>) {
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
}

fn selected(app: &Entity<ConduitApp>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|_, cx| {
        assert!(
            app.read(cx).plugins.is_selected(id),
            "expected {id}, got {}",
            app.read(cx).plugins.selected()
        );
    });
}

#[gpui_kit::test]
fn context_changes_and_revocation_reject_saved_navigation_requests(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, store(), plugins(1));
    click(cx, builtins::EXAMPLE_ACTIVITY);
    let stale = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            let item = app
                .plugins
                .registry
                .activities(&app.plugins.snapshot)
                .into_iter()
                .find(|i| i.id.as_str() == builtins::EXAMPLE_ACTIVITY)
                .unwrap();
            app.plugins.request(item).unwrap()
        })
    });
    cx.update(|window, cx| app.update(cx, |app, cx| app.switch(1, Some(2), window, cx)));
    draw(cx);
    click(cx, "nav-workspaces");
    cx.update(|_, cx| app.update(cx, |app, cx| app.dispatch_navigation(&stale, cx)));
    selected(&app, cx, builtins::WORKSPACES);
    click(cx, builtins::EXAMPLE_ACTIVITY);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.plugins.set_collapsed(true);
            let owner = PluginId::new(builtins::EXAMPLE).unwrap();
            let handle = app.plugins.registry.registration(&owner).unwrap().clone();
            app.plugins.registry.unregister(&handle).unwrap();
            cx.notify();
        })
    });
    draw(cx);
    selected(&app, cx, builtins::WORKSPACES);
    cx.update(|_, cx| assert!(app.read(cx).plugins.collapsed()));
    assert!(cx.element_bounds(builtins::EXAMPLE_ACTIVITY).is_none());
    click(cx, "test.navigation.entry-0");
    selected(&app, cx, "test.navigation.view-0");
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = None;
            app.sync_plugin_context();
            assert!(app.plugins.snapshot.input().session.is_none());
            cx.notify();
        })
    });
    draw(cx);
    selected(&app, cx, builtins::WORKSPACES);
}
