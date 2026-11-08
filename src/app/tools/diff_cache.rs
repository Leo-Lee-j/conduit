//! Bounded LRU of parsed tool cards. Visible elements can retain their own state.
use super::*;

const MAX_ENTRIES: usize = 32;
// This bounds retained patch text, not the native allocations of the diff renderer.
const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;

struct CachedDiff {
    patch: String,
    state: Entity<DiffState>,
    used: u64,
}

#[derive(Default)]
pub(in crate::app) struct DiffCache {
    entries: HashMap<(u64, String), CachedDiff>,
    clock: u64,
}

impl DiffCache {
    pub fn forget_session(&mut self, session: u64) {
        self.entries.retain(|(owner, _), _| *owner != session);
    }

    pub fn state(
        &mut self,
        key: &(u64, String),
        patch: &str,
        cx: &mut Context<ConduitApp>,
    ) -> Option<Entity<DiffState>> {
        self.clock += 1;
        if let Some(cached) = self.entries.get_mut(key)
            && cached.patch == patch
        {
            cached.used = self.clock;
            return Some(cached.state.clone());
        }
        self.entries.remove(key);
        let files = DiffFile::parse(patch).ok()?;
        if files.is_empty() {
            return None;
        }
        let state = cx.new(|cx| {
            DiffState::new(files, cx)
                .with_mode(DiffMode::Unified)
                .with_context_lines(None)
        });
        // A large card remains usable without evicting the entire cache or being retained.
        if patch.len() > MAX_PATCH_BYTES {
            return Some(state);
        }
        let mut bytes: usize = self.entries.values().map(|entry| entry.patch.len()).sum();
        while self.entries.len() >= MAX_ENTRIES || bytes + patch.len() > MAX_PATCH_BYTES {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())?;
            let removed = self.entries.remove(&oldest)?;
            bytes -= removed.patch.len();
        }
        self.entries.insert(
            key.clone(),
            CachedDiff {
                patch: patch.to_owned(),
                state: state.clone(),
                used: self.clock,
            },
        );
        Some(state)
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
