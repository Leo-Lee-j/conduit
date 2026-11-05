use super::*;
use crate::plugins::{
    builtins,
    commands::Capability,
    context::{ContextInput, ContextTracker, Scope, SessionKey, When, WorkspaceKey},
    contributions::{ContributionSet, Location, ViewDescriptor},
    ids::{Owner, ViewId},
    registry::{HostCatalog, RendererSpec},
};
use std::{cell::Cell, collections::BTreeSet, rc::Rc};

fn input(workspace: u64) -> ContextInput {
    let workspace =
        WorkspaceKey::from_host_path(std::env::temp_dir().join(format!("instance-{workspace}")))
            .unwrap();
    ContextInput {
        workspace: Some(workspace.clone()),
        session: Some(SessionKey { workspace, id: 7 }),
        writable: true,
        ..Default::default()
    }
}
fn request(registry: &Registry, cx: &PluginContext, view: &str) -> ViewRequest {
    registry
        .prepare_view(&ViewId::new(view).unwrap(), cx)
        .unwrap()
}
struct Released(Rc<Cell<usize>>);
impl Drop for Released {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn bounded_lru_preserves_visible_views_and_releases_hidden_state() {
    let registry = builtins::register(true).unwrap().registry;
    let mut tracker = ContextTracker::default();
    let mut cache = InstanceCache::<Released>::default();
    let drops = Rc::new(Cell::new(0));
    let mut first = None;
    for index in 0..40 {
        let cx = tracker.update(input(index)).unwrap();
        cache.reconcile(&registry, cx, &[], |_| true);
        let view = request(&registry, cx, builtins::EXAMPLE_INDEX);
        if index == 0 {
            first = Some(view.key().clone());
        }
        cache
            .acquire(&registry, cx, &view, || Released(drops.clone()))
            .unwrap();
    }
    assert_eq!(cache.len(), 32);
    assert_eq!(drops.get(), 8);
    assert!(cache.get(&first.unwrap()).is_none());
    let cx = tracker.update(input(39)).unwrap();
    let left = request(&registry, cx, builtins::EXAMPLE_INDEX);
    let right = request(&registry, cx, builtins::EXAMPLE_SUMMARY);
    let visible = vec![left.key().clone(), right.key().clone()];
    cache
        .acquire(&registry, cx, &right, || Released(drops.clone()))
        .unwrap();
    for _ in 0..5 {
        cache.reconcile(&registry, cx, &visible, |_| true);
        cache
            .acquire(&registry, cx, &left, || panic!("visible state was evicted"))
            .unwrap();
    }
    assert_eq!(cache.len(), 32);
    assert!(cache.get(right.key()).is_some());
    drop(cache);
    assert_eq!(drops.get(), 41);
}

#[test]
fn full_visible_budget_rejects_creation_without_dropping_visible_resources() {
    let registry = builtins::register(true).unwrap().registry;
    let mut tracker = ContextTracker::default();
    let cx = tracker.update(input(0)).unwrap();
    let mut cache = InstanceCache::<u64>::new(1);
    let left = request(&registry, cx, builtins::EXAMPLE_INDEX);
    let right = request(&registry, cx, builtins::EXAMPLE_SUMMARY);
    cache.acquire(&registry, cx, &left, || 42).unwrap();
    assert!(
        cache
            .acquire(&registry, cx, &right, || panic!("over-budget allocation"))
            .is_err()
    );
    assert_eq!(cache.get(left.key()), Some(&42));
}

#[test]
fn hidden_and_replaced_requests_cancel_and_completion_is_single_use() {
    let registry = builtins::register(true).unwrap().registry;
    let mut tracker = ContextTracker::default();
    let cx = tracker.update(input(0)).unwrap();
    let mut cache = InstanceCache::<u64, Released>::default();
    let view = request(&registry, cx, builtins::EXAMPLE_INDEX);
    cache.acquire(&registry, cx, &view, || 0).unwrap();
    let drops = Rc::new(Cell::new(0));
    let first = cache.begin(&registry, cx, view.key()).unwrap();
    assert!(cache.attach(&first, Released(drops.clone())));
    let second = cache.begin(&registry, cx, view.key()).unwrap();
    assert!(first.is_cancelled());
    assert_eq!(drops.get(), 1);
    assert!(cache.complete(&registry, cx, &first).is_none());
    *cache.complete(&registry, cx, &second).unwrap() = 7;
    assert!(cache.complete(&registry, cx, &second).is_none());
    let third = cache.begin(&registry, cx, view.key()).unwrap();
    cache.attach(&third, Released(drops.clone()));
    cache.reconcile(&registry, cx, &[], |_| true);
    assert!(third.is_cancelled());
    assert_eq!(drops.get(), 2);
    assert_eq!(cache.get(view.key()), Some(&7));
    cache
        .acquire(&registry, cx, &view, || panic!("lost hidden state"))
        .unwrap();
    assert!(cache.complete(&registry, cx, &third).is_none());
}

#[test]
fn route_readonly_and_closing_versions_reject_results_even_before_reconcile() {
    let registry = builtins::register(true).unwrap().registry;
    for change in 0..5 {
        let mut tracker = ContextTracker::default();
        let cx = tracker.update(input(0)).unwrap().clone();
        let mut cache = InstanceCache::<u64, Released>::default();
        let view = request(&registry, &cx, builtins::EXAMPLE_INDEX);
        cache.acquire(&registry, &cx, &view, || 42).unwrap();
        let ticket = cache.begin(&registry, &cx, view.key()).unwrap();
        let drops = Rc::new(Cell::new(0));
        cache.attach(&ticket, Released(drops.clone()));
        let mut next = input(0);
        match change {
            0 => next = input(1),
            1 => next.session.as_mut().unwrap().id = 8,
            2 => next.document = Some("new document".into()),
            3 => next.writable = false,
            _ => next.closing = true,
        }
        let next = tracker.update(next).unwrap();
        assert!(cache.complete(&registry, next, &ticket).is_none());
        cache.reconcile(&registry, next, std::slice::from_ref(view.key()), |_| true);
        assert!(ticket.is_cancelled());
        assert_eq!(drops.get(), 1);
        if change == 4 {
            assert!(cache.is_empty());
        } else {
            assert_eq!(cache.get(view.key()), Some(&42));
        }
        let again = tracker.update(input(0)).unwrap();
        cache
            .acquire(
                &registry,
                again,
                &request(&registry, again, builtins::EXAMPLE_INDEX),
                || 0,
            )
            .unwrap();
        assert!(cache.complete(&registry, again, &ticket).is_none());
    }
}

#[test]
fn revocation_rebind_eviction_and_cache_identity_never_revive_tickets() {
    let mut registry = builtins::register(true).unwrap().registry;
    let mut tracker = ContextTracker::default();
    let cx = tracker.update(input(0)).unwrap().clone();
    let view = request(&registry, &cx, builtins::EXAMPLE_INDEX);
    let mut cache = InstanceCache::<u64>::new(1);
    cache.acquire(&registry, &cx, &view, || 1).unwrap();
    let ticket = cache.begin(&registry, &cx, view.key()).unwrap();
    let owner = registry.registration(&view.key().owner).unwrap().clone();
    registry.unregister(&owner).unwrap();
    registry
        .register(
            builtins::workspace_overview().unwrap(),
            BTreeSet::from([Capability::WorkspaceSummary]),
        )
        .unwrap();
    assert!(cache.complete(&registry, &cx, &ticket).is_none());
    let replacement = request(&registry, &cx, builtins::EXAMPLE_INDEX);
    cache.acquire(&registry, &cx, &replacement, || 2).unwrap();
    assert_eq!(cache.get(view.key()), Some(&2));
    let new = cache.begin(&registry, &cx, view.key()).unwrap();
    assert_ne!(ticket.generation(), new.generation());
    let mut foreign = InstanceCache::<u64>::new(1);
    foreign.acquire(&registry, &cx, &replacement, || 3).unwrap();
    foreign.begin(&registry, &cx, view.key()).unwrap();
    assert!(foreign.complete(&registry, &cx, &new).is_none());
    cache.reconcile(&registry, &cx, &[], |_| true);
    let next = tracker.update(input(1)).unwrap();
    let other = request(&registry, next, builtins::EXAMPLE_INDEX);
    cache.acquire(&registry, next, &other, || 4).unwrap();
    assert!(cache.get(view.key()).is_none());
    cache.clear();
    let again = tracker.update(input(0)).unwrap();
    cache
        .acquire(
            &registry,
            again,
            &request(&registry, again, builtins::EXAMPLE_INDEX),
            || 5,
        )
        .unwrap();
    assert!(cache.complete(&registry, again, &new).is_none());
}

#[test]
fn two_owners_session_scope_removal_readonly_and_drop_release_resources() {
    let mut sets = vec![];
    let mut catalog = HostCatalog::default();
    for owner in ["test.alpha", "test.beta"] {
        let owner = PluginId::new(owner).unwrap();
        let mut set = ContributionSet::empty(Owner::external(owner.clone()).unwrap());
        let id = ViewId::new(format!("{owner}.view")).unwrap();
        catalog
            .allow_renderer(
                id.as_str(),
                RendererSpec {
                    owner,
                    location: Location::Right,
                    scope: Scope::Session,
                },
            )
            .unwrap();
        set.views.push(ViewDescriptor {
            renderer: id.to_string(),
            id,
            title: "view".into(),
            location: Location::Right,
            scope: Scope::Session,
            when: When::default(),
        });
        sets.push(set);
    }
    let mut registry = Registry::new(catalog);
    for set in sets {
        registry.register(set, BTreeSet::new()).unwrap();
    }
    let mut tracker = ContextTracker::default();
    let cx = tracker.update(input(0)).unwrap().clone();
    let mut cache = InstanceCache::<Released, Released>::default();
    let drops = Rc::new(Cell::new(0));
    let views = [
        request(&registry, &cx, "test.alpha.view"),
        request(&registry, &cx, "test.beta.view"),
    ];
    for view in &views {
        cache
            .acquire(&registry, &cx, view, || Released(drops.clone()))
            .unwrap();
        let ticket = cache.begin(&registry, &cx, view.key()).unwrap();
        cache.attach(&ticket, Released(drops.clone()));
    }
    let visible: Vec<_> = views.iter().map(|r| r.key().clone()).collect();
    cache.reconcile(&registry, &cx, &visible, |key| {
        key.owner != views[0].key().owner
    });
    assert_eq!(drops.get(), 2);
    assert_eq!(cache.len(), 1);
    let mut readonly = input(0);
    readonly.writable = false;
    let readonly = tracker.update(readonly).unwrap();
    cache.reconcile(&registry, readonly, &visible, |_| true);
    assert!(cache.is_empty());
    assert_eq!(drops.get(), 4);
    let cx = tracker.update(input(0)).unwrap();
    let view = request(&registry, cx, "test.beta.view");
    cache
        .acquire(&registry, cx, &view, || Released(drops.clone()))
        .unwrap();
    let ticket = cache.begin(&registry, cx, view.key()).unwrap();
    cache.attach(&ticket, Released(drops.clone()));
    drop(cache);
    assert!(ticket.is_cancelled());
    assert_eq!(drops.get(), 6);
}
