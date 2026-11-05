//! Host-owned bounded state. No GPUI types or work execution: adapters supply state
//! and an owned task resource, whose Drop must stop delivery when cancelled.
use super::{
    Error, Result,
    context::PluginContext,
    ids::PluginId,
    registry::{Registration, Registry},
    require,
    state::{ViewInstanceKey, ViewRequest},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A completion capability, created by the host and never reconstructed from input.
#[derive(Clone, Debug)]
pub struct RequestTicket {
    request: ViewRequest,
    generation: u64,
    cancelled: Arc<AtomicBool>,
}
impl RequestTicket {
    pub fn key(&self) -> &ViewInstanceKey {
        self.request.key()
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn context_version(&self) -> u64 {
        self.request.context_version()
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

struct Pending<R> {
    ticket: RequestTicket,
    resource: Option<R>,
}
impl<R> Drop for Pending<R> {
    fn drop(&mut self) {
        // Signal before dropping the resource; even a non-cooperative worker cannot
        // publish through the completion gate after its delivery task is released.
        self.ticket.cancelled.store(true, Ordering::Release);
    }
}
struct Instance<T, R> {
    registration: Registration,
    generation: u64,
    used: u64,
    visible: bool,
    pending: Option<Pending<R>>,
    value: T,
}

/// One budget across all owners and scopes in a window. Visible state is protected;
/// hiding cancels work but retains state; eviction/revocation/clear drops both.
pub struct InstanceCache<T, R = ()> {
    entries: BTreeMap<ViewInstanceKey, Instance<T, R>>,
    capacity: usize,
    clock: u64,
}
impl<T, R> Default for InstanceCache<T, R> {
    fn default() -> Self {
        Self::new(32)
    }
}
impl<T, R> InstanceCache<T, R> {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            capacity,
            clock: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn get(&self, key: &ViewInstanceKey) -> Option<&T> {
        self.entries.get(key).map(|entry| &entry.value)
    }
    pub fn get_mut(&mut self, key: &ViewInstanceKey) -> Option<&mut T> {
        self.entries.get_mut(key).map(|entry| &mut entry.value)
    }
    pub fn is_visible(&self, key: &ViewInstanceKey) -> bool {
        self.entries.get(key).is_some_and(|entry| entry.visible)
    }
    pub fn visible_keys(&self) -> Vec<ViewInstanceKey> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.visible)
            .map(|(key, _)| key.clone())
            .collect()
    }
    pub fn has_pending(&self, key: &ViewInstanceKey) -> bool {
        self.entries
            .get(key)
            .is_some_and(|entry| entry.pending.is_some())
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn remove_owner(&mut self, owner: &PluginId) {
        self.entries.retain(|key, _| &key.owner != owner);
    }
    pub fn retain_registered(&mut self, registry: &Registry) {
        self.entries
            .retain(|_, entry| registry.is_registered(&entry.registration));
    }

    /// Called before rendering, scheduling, or accepting a result. Existence checks
    /// use host identities, including hidden workspaces/sessions, not array indices.
    pub fn reconcile(
        &mut self,
        registry: &Registry,
        cx: &PluginContext,
        visible: &[ViewInstanceKey],
        mut scope_exists: impl FnMut(&ViewInstanceKey) -> bool,
    ) {
        self.entries.retain(|key, entry| {
            if cx.input().closing
                || !registry.is_registered(&entry.registration)
                || (!cx.writable() && !registry.is_host_owner(&key.owner))
                || !scope_exists(key)
            {
                return false;
            }
            entry.visible = visible.contains(key)
                && registry
                    .prepare_view(&key.view, cx)
                    .is_ok_and(|request| request.key() == key);
            if !entry.visible
                || entry
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.ticket.context_version() != cx.version())
            {
                entry.pending = None;
            }
            true
        });
    }

    /// Only a validated, physically visible host view should acquire an instance.
    pub fn acquire(
        &mut self,
        registry: &Registry,
        cx: &PluginContext,
        request: &ViewRequest,
        create: impl FnOnce() -> T,
    ) -> Result<&mut T> {
        registry.validate_view(request, cx)?;
        self.retain_registered(registry);
        let key = request.key();
        if !self.entries.contains_key(key) {
            if self.entries.len() >= self.capacity {
                let oldest = self
                    .entries
                    .iter()
                    .filter(|(_, entry)| !entry.visible)
                    .min_by_key(|(_, entry)| entry.used)
                    .map(|(key, _)| key.clone())
                    .ok_or_else(|| {
                        Error("view instance budget is occupied by visible views".into())
                    })?;
                self.entries.remove(&oldest);
            }
            self.clock = self
                .clock
                .checked_add(1)
                .ok_or_else(|| Error("instance generation exhausted".into()))?;
            self.entries.insert(
                key.clone(),
                Instance {
                    registration: request.registration.clone(),
                    generation: self.clock,
                    used: self.clock,
                    visible: true,
                    pending: None,
                    value: create(),
                },
            );
        }
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or_else(|| Error("instance clock exhausted".into()))?;
        let entry = self.entries.get_mut(key).unwrap();
        entry.visible = true;
        entry.used = self.clock;
        Ok(&mut entry.value)
    }

    /// At most one request per instance. Replacing it cancels the previous resource.
    pub fn begin(
        &mut self,
        registry: &Registry,
        cx: &PluginContext,
        key: &ViewInstanceKey,
    ) -> Result<RequestTicket> {
        let request = registry.prepare_view(&key.view, cx)?;
        require(request.key() == key, "request scope changed")?;
        let entry = self
            .entries
            .get_mut(key)
            .ok_or_else(|| Error("view instance was released".into()))?;
        require(
            entry.visible && registry.is_registered(&entry.registration),
            "view instance is inactive",
        )?;
        let ticket = RequestTicket {
            request,
            generation: entry.generation,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        entry.pending = Some(Pending {
            ticket: ticket.clone(),
            resource: None,
        });
        Ok(ticket)
    }
    fn matches(entry: &Instance<T, R>, ticket: &RequestTicket) -> bool {
        entry.visible
            && entry.generation == ticket.generation
            && !ticket.is_cancelled()
            && entry
                .pending
                .as_ref()
                .is_some_and(|pending| Arc::ptr_eq(&pending.ticket.cancelled, &ticket.cancelled))
    }
    pub fn attach(&mut self, ticket: &RequestTicket, resource: R) -> bool {
        let Some(entry) = self
            .entries
            .get_mut(ticket.key())
            .filter(|entry| Self::matches(entry, ticket))
        else {
            return false;
        };
        let pending = entry.pending.as_mut().unwrap();
        if pending.resource.is_some() {
            return false;
        }
        pending.resource = Some(resource);
        true
    }
    /// Single-use publication gate. The adapter reconciles visibility/existence first.
    pub fn complete(
        &mut self,
        registry: &Registry,
        cx: &PluginContext,
        ticket: &RequestTicket,
    ) -> Option<&mut T> {
        registry.validate_view(&ticket.request, cx).ok()?;
        let entry = self
            .entries
            .get_mut(ticket.key())
            .filter(|entry| Self::matches(entry, ticket))?;
        entry.pending = None;
        Some(&mut entry.value)
    }
}

#[cfg(test)]
#[path = "instances/tests.rs"]
mod tests;
