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
    AppContext as _, Entity, Focusable as _, InteractiveElement, IntoElement, Modifiers,
    MouseButton, ParentElement, SharedString, TestAppContext, VisualTestContext,
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

pub(super) fn click(cx: &mut VisualTestContext, id: &'static str) {
    let bounds = cx
        .debug_bounds(id)
        .or_else(|| {
            cx.update(|window, _| {
                test_support::find(window, &[], &SharedString::from(id.to_owned()).into())
                    .map(|element| element.bounds())
            })
        })
        .unwrap_or_else(|| panic!("missing {id}"));
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
fn empty_workspace_navigation_collapse_and_git_shortcut(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, Store::default(), PluginUi::new());
    click(cx, "nav-files");
    selected(&app, cx, builtins::FILES);
    click(cx, "files-pick-workspace");
    selected(&app, cx, builtins::WORKSPACES);
    click(cx, "nav-git");
    selected(&app, cx, builtins::GIT);
    assert!(cx.debug_bounds("git-navigation").is_some());
    click(cx, "nav-collapse");
    assert!(cx.debug_bounds("navigation-panel").is_none());
    cx.update(|window, cx| {
        app.read(cx)
            .input
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_keystrokes("ctrl-k");
    draw(cx);
    selected(&app, cx, builtins::GIT);
    assert!(cx.debug_bounds("navigation-panel").is_some());
    cx.simulate_keystrokes("ctrl-k");
    draw(cx);
    selected(&app, cx, builtins::WORKSPACES);
    cx.update(|_, cx| assert!(app.read(cx).plugins.snapshot.input().workspace.is_none()));
}

#[gpui_kit::test]
fn overflow_routes_owners_and_disables_unbound_actions(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, store(), plugins(2));
    for id in [
        "test.navigation.entry-missing",
        "test.navigation.command-entry",
    ] {
        cx.update(|window, _| {
            assert_eq!(
                test_support::find(window, &[], &SharedString::from(id).into())
                    .unwrap()
                    .disabled(),
                Some(true)
            );
        });
        click(cx, id);
        selected(&app, cx, builtins::WORKSPACES);
    }
    cx.simulate_resize(gpui_kit::size(px(900.), px(300.)));
    draw(cx);
    click(cx, "nav-more");
    assert!(cx.debug_bounds("nav-more-test.navigation.hidden").is_none());
    for id in [
        "nav-more-test.navigation.entry-missing",
        "nav-more-test.navigation.command-entry",
    ] {
        click(cx, id);
        selected(&app, cx, builtins::WORKSPACES);
    }
    click(cx, "nav-more-example.workspace-overview.activity");
    selected(&app, cx, builtins::EXAMPLE_INDEX);
    assert!(cx.debug_bounds("plugin-navigation-page").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        let item = app
            .plugins
            .registry
            .activities(&app.plugins.snapshot)
            .into_iter()
            .find(|i| i.id.as_str() == builtins::EXAMPLE_ACTIVITY)
            .unwrap();
        assert!(app.activity_selected(item));
    });
    click(cx, "nav-more");
    click(cx, "nav-more-test.navigation.entry-0");
    selected(&app, cx, "test.navigation.view-0");
    click(cx, "nav-collapse");
    click(cx, "nav-more");
    click(cx, "nav-more-example.workspace-overview.activity");
    selected(&app, cx, builtins::EXAMPLE_INDEX);
    assert!(cx.debug_bounds("navigation-panel").is_some());
}

#[gpui_kit::test]
fn resizing_reflows_overflow_and_keeps_fixed_controls_visible(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, store(), plugins(2));
    assert!(cx.debug_bounds("nav-more").is_none());
    for (width, height) in [
        (850., 300.),
        (900., 350.),
        (850., 450.),
        (1100., 800.),
        (850., 300.),
    ] {
        cx.simulate_resize(gpui_kit::size(px(width), px(height)));
        draw(cx);
        let bottom = cx.debug_bounds("nav-collapse").unwrap();
        assert!(
            bottom.bottom() <= px(height),
            "collapse outside {height}: {bottom:?}"
        );
        let mut previous = cx
            .debug_bounds("nav-more")
            .or_else(|| cx.debug_bounds("nav-git"))
            .unwrap();
        for id in ["git-branch-trigger", "nav-collapse"] {
            let bounds = cx.debug_bounds(id).unwrap();
            assert!(
                bounds.top() >= previous.bottom(),
                "overlapping {id} at {height}"
            );
            previous = bounds;
        }
        assert_eq!(cx.debug_bounds("nav-more").is_some(), height < 450.);
        let settings = cx.debug_bounds("title-settings").unwrap();
        let about = cx.debug_bounds("title-about").unwrap();
        assert!(settings.left() > px(width / 2.));
        assert!(settings.right() <= about.left());
        assert_eq!(settings.top(), about.top());
        cx.update(|window, _| {
            let title = test_support::find(window, &[], &"title-bar".into()).unwrap();
            let controls = test_support::find(window, &[], &"window-controls".into()).unwrap();
            assert!(title.bounds().contains(&settings.origin));
            assert!(settings.bottom() <= title.bounds().bottom());
            assert!(about.right() <= controls.bounds().left());
        });
        assert!(cx.debug_bounds("nav-settings").is_none());
        assert!(cx.debug_bounds("nav-about").is_none());
    }
    // A resize is a sidebar invalidation even when neither context nor data changed.
    selected(&app, cx, builtins::WORKSPACES);
}

#[gpui_kit::test]
fn overflowing_menu_stays_in_window_and_reaches_last_action(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, store(), plugins(24));
    cx.simulate_resize(gpui_kit::size(px(900.), px(350.)));
    draw(cx);
    click(cx, "nav-more");
    for height in [350., 300., 350.] {
        cx.simulate_resize(gpui_kit::size(px(900.), px(height)));
        draw(cx);
        cx.update(|window, _| {
            let menus: Vec<_> = test_support::snapshots(window)
                .into_iter()
                .filter(|e| e.role() == Some(gpui_kit::Role::Menu))
                .collect();
            assert_eq!(menus.len(), 1);
            let bounds = menus[0].bounds();
            assert!(
                bounds.top() >= px(0.) && bounds.bottom() <= px(height),
                "menu outside {height}px window: {bounds:?}"
            );
        });
    }
    // The four built-in entries now fit in the rail at this height. Keyboard
    // navigation must scroll through the 24 remaining enabled actions and route
    // the last one through the same validated request.
    cx.simulate_mouse_move(
        gpui_kit::point(px(850.), px(340.)),
        None,
        Modifiers::default(),
    );
    draw(cx);
    for _ in 0..24 {
        cx.simulate_keystrokes("down");
        draw(cx);
    }
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    selected(&app, cx, "test.navigation.view-23");
}

#[gpui_kit::test]
fn context_changes_and_revocation_reject_saved_navigation_requests(cx: &mut TestAppContext) {
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
    assert!(cx.debug_bounds(builtins::EXAMPLE_ACTIVITY).is_none());
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

#[gpui_kit::test]
fn workspace_reorder_preserves_plugin_scope_and_context_version(cx: &mut TestAppContext) {
    let (app, cx) = fixture(cx, store(), plugins(1));
    click(cx, builtins::EXAMPLE_ACTIVITY);
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let key = app.plugins.state.navigation_key().unwrap().clone();
            let version = app.plugins.snapshot.version();
            let source = app.store.workspaces[0].path.clone();
            let target = app.store.workspaces[1].path.clone();
            app.reorder_workspace(&source, &target, cx);
            app.sync_plugin_context();
            assert_eq!(app.selected_workspace, Some(1));
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.plugins.snapshot.version(), version);
            assert_eq!(app.plugins.state.navigation_key(), Some(&key));
            app.writable = false;
        })
    });
    draw(cx);
    selected(&app, cx, builtins::EXAMPLE_INDEX);
}

struct Repositories(std::path::PathBuf);
impl Repositories {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "conduit-plugin-navigation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        for name in ["alpha", "beta"] {
            let path = root.join(name);
            std::fs::create_dir_all(path.join("folder")).unwrap();
            std::fs::write(path.join(format!("{name}.txt")), name).unwrap();
            std::fs::write(path.join("folder/child.txt"), name).unwrap();
            assert!(
                std::process::Command::new(crate::git::resolve_git().unwrap())
                    .args(["init", "-q"])
                    .current_dir(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        Self(root)
    }
    fn store(&self) -> Store {
        let mut store = store();
        store.workspaces[0].path = self.0.join("alpha");
        store.workspaces[1].path = self.0.join("beta");
        // Exercise a workspace with no session through the same picker.
        store.workspaces[1].sessions.clear();
        store
    }
}
impl Drop for Repositories {
    fn drop(&mut self) {
        let path = self.0.canonicalize().unwrap();
        assert!(path.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        for attempt in 0..100 {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if attempt == 99 => panic!("repository cleanup failed: {error}"),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
    }
}

fn wait_for(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        draw(cx);
        if ready(cx) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "navigation update timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn choose_workspace(cx: &mut VisualTestContext, picker: &'static str, name: &str) {
    click(cx, picker);
    let bounds = cx.update(|window, _| {
        test_support::snapshots(window)
            .into_iter()
            .find(|e| e.role() == Some(gpui_kit::Role::MenuItem) && e.label() == Some(name))
            .unwrap()
            .bounds()
    });
    cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn file_navigation_refreshes_and_preserves_expansion_across_workspace_picker(
    cx: &mut TestAppContext,
) {
    let repositories = Repositories::new();
    let (app, cx) = fixture(cx, repositories.store(), PluginUi::new());
    click(cx, "nav-files");
    wait_for(cx, |cx| {
        cx.debug_bounds("file-tree-entry-alpha.txt").is_some()
    });
    click(cx, "file-tree-entry-folder");
    wait_for(cx, |cx| {
        cx.debug_bounds("file-tree-entry-folder\\child.txt")
            .is_some()
    });
    std::fs::write(repositories.0.join("alpha/new.txt"), "new").unwrap();
    click(cx, "nav-files");
    wait_for(cx, |cx| {
        cx.debug_bounds("file-tree-entry-new.txt").is_some()
    });
    assert!(
        cx.debug_bounds("file-tree-entry-folder\\child.txt")
            .is_some()
    );
    choose_workspace(cx, "files-workspace", "beta");
    selected(&app, cx, builtins::FILES);
    wait_for(cx, |cx| {
        cx.debug_bounds("file-tree-entry-beta.txt").is_some()
    });
    assert!(cx.debug_bounds("file-tree-entry-alpha.txt").is_none());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(app.selected_workspace, Some(1));
        assert!(app.selected_session.is_none());
        assert!(app.plugins.snapshot.input().session.is_none());
    });
    choose_workspace(cx, "files-workspace", "alpha");
    wait_for(cx, |cx| {
        cx.debug_bounds("file-tree-entry-folder\\child.txt")
            .is_some()
    });
    selected(&app, cx, builtins::FILES);
}

#[gpui_kit::test]
fn git_navigation_refreshes_on_click_and_routes_the_workspace_picker(cx: &mut TestAppContext) {
    let repositories = Repositories::new();
    let (app, cx) = fixture(cx, repositories.store(), PluginUi::new());
    click(cx, "nav-git");
    let wait_git = |cx: &mut VisualTestContext, expected: &str| {
        wait_for(cx, |cx| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.poll_branches(cx);
                    app.poll_git(window, cx);
                    app.git.probe.is_none()
                        && app
                            .git
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| s.files.iter().any(|f| f.path == expected))
                })
            })
        });
    };
    wait_git(cx, "alpha.txt");
    std::fs::write(repositories.0.join("alpha/new.txt"), "new").unwrap();
    click(cx, "nav-git");
    wait_git(cx, "new.txt");
    choose_workspace(cx, "git-workspace", "beta");
    selected(&app, cx, builtins::GIT);
    wait_git(cx, "beta.txt");
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(
            app.git
                .snapshot
                .as_ref()
                .unwrap()
                .files
                .iter()
                .all(|f| f.path != "alpha.txt")
        );
        assert!(app.plugins.snapshot.input().session.is_none());
    });
    choose_workspace(cx, "git-workspace", "alpha");
    selected(&app, cx, builtins::GIT);
    wait_git(cx, "alpha.txt");
}
