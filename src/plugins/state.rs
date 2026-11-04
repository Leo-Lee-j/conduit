//! Pure selection state; GPUI entities and view-instance caches belong to app adapters.
use super::{
    Error, Result,
    context::{PluginContext, ScopeKey},
    contributions::Location,
    ids::{PluginId, ViewId},
    registry::{Registration, Registry},
    require,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewInstanceKey {
    pub owner: PluginId,
    pub view: ViewId,
    pub scope: ScopeKey,
}
#[derive(Clone, Debug)]
pub struct ViewRequest {
    pub(super) registration: Registration,
    pub(super) key: ViewInstanceKey,
    pub(super) context_version: u64,
}
impl ViewRequest {
    pub fn key(&self) -> &ViewInstanceKey {
        &self.key
    }
    pub fn context_version(&self) -> u64 {
        self.context_version
    }
}
#[derive(Clone, Debug)]
struct Selection {
    id: ViewId,
    registration: Registration,
    key: Option<ViewInstanceKey>,
}
impl Selection {
    fn fallback(registry: &Registry, id: ViewId, location: Location) -> Result<Self> {
        let view = registry
            .view(&id)
            .ok_or_else(|| Error("fallback view is not registered".into()))?;
        require(
            view.location == location,
            "fallback view has the wrong location",
        )?;
        let owner = registry.owner_for(id.as_str()).unwrap();
        require(
            registry.is_host_owner(owner),
            "fallback must be supplied by the host",
        )?;
        Ok(Self {
            id,
            registration: registry.registration(owner).unwrap().clone(),
            key: None,
        })
    }
    fn reconcile(&mut self, registry: &Registry, cx: &PluginContext) -> bool {
        if !registry.is_registered(&self.registration) {
            return false;
        }
        match registry.prepare_view(&self.id, cx) {
            Ok(request) => {
                self.key = Some(request.key);
                true
            }
            Err(_) => false,
        }
    }
}
/// Integrators call reconcile after context changes or registry revocation. Revocation
/// already blocks commands; this second step repairs selection and discards stale keys.
pub struct WorkbenchState {
    navigation: Selection,
    right: Selection,
    fallback_navigation: ViewId,
    fallback_right: ViewId,
    collapsed: bool,
    right_open: bool,
}
impl WorkbenchState {
    pub fn new(registry: &Registry, navigation: ViewId, right: ViewId) -> Result<Self> {
        Ok(Self {
            navigation: Selection::fallback(registry, navigation.clone(), Location::Navigation)?,
            right: Selection::fallback(registry, right.clone(), Location::Right)?,
            fallback_navigation: navigation,
            fallback_right: right,
            collapsed: false,
            right_open: false,
        })
    }
    pub fn navigation(&self) -> &ViewId {
        &self.navigation.id
    }
    pub fn right(&self) -> &ViewId {
        &self.right.id
    }
    pub fn navigation_key(&self) -> Option<&ViewInstanceKey> {
        self.navigation.key.as_ref()
    }
    pub fn right_key(&self) -> Option<&ViewInstanceKey> {
        self.right.key.as_ref()
    }
    pub fn collapsed(&self) -> bool {
        self.collapsed
    }
    pub fn right_open(&self) -> bool {
        self.right_open
    }
    pub fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }
    pub fn close_right(&mut self) {
        self.right_open = false;
    }
    pub fn open(
        &mut self,
        registry: &Registry,
        request: &ViewRequest,
        cx: &PluginContext,
    ) -> Result<()> {
        let view = registry.validate_view(request, cx)?;
        let selection = Selection {
            id: view.id.clone(),
            registration: request.registration.clone(),
            key: Some(request.key.clone()),
        };
        match view.location {
            Location::Navigation => {
                self.navigation = selection;
                self.collapsed = false;
            }
            Location::Right => {
                self.right = selection;
                self.right_open = true;
            }
        }
        Ok(())
    }
    pub fn reconcile(&mut self, registry: &Registry, cx: &PluginContext) -> Result<()> {
        if !self.navigation.reconcile(registry, cx) {
            self.navigation = Selection::fallback(
                registry,
                self.fallback_navigation.clone(),
                Location::Navigation,
            )?;
            self.navigation.reconcile(registry, cx);
        }
        if !self.right.reconcile(registry, cx) {
            self.right =
                Selection::fallback(registry, self.fallback_right.clone(), Location::Right)?;
            self.right.reconcile(registry, cx);
            self.right_open = false;
        }
        Ok(())
    }
}
