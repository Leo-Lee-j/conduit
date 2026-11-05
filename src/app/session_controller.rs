//! Session identity, import generations and runtime ownership.
use super::{ActiveRun, PlanStep, SessionNaming};
use crate::domain::Workspace;
use std::collections::HashSet;
use std::{cell::RefCell, collections::HashMap, path::PathBuf};

#[derive(Default)]
pub(super) struct SessionController {
    pub(super) runs: HashMap<u64, ActiveRun>,
    pub(super) queue_saves: HashMap<u64, super::runtime::PendingQueueSave>,
    pub(super) plans: HashMap<u64, Vec<PlanStep>>,
    pub(super) namings: HashMap<u64, SessionNaming>,
    pub(super) naming_errors: HashMap<u64, String>,
    pub(super) unread_completed: HashSet<u64>,

    pub(super) event_cursor: usize,
    pub(super) global_revision: u64,
    positions: RefCell<HashMap<u64, (usize, usize)>>,
    revisions: HashMap<u64, u64>,
    pub(super) scan_cancels: HashMap<PathBuf, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub(super) imports: HashMap<PathBuf, HashMap<u64, u64>>,
    pub(super) scan_progress: HashMap<PathBuf, (usize, usize)>,
}

impl SessionController {
    pub(super) fn cancel_imports(&mut self) {
        for cancel in self.scan_cancels.values() {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
        self.scan_cancels.clear();
        self.imports.clear();
        self.scan_progress.clear();
    }
    pub(super) fn locate(&self, workspaces: &[Workspace], id: u64) -> Option<(usize, usize)> {
        if let Some(&(w, s)) = self.positions.borrow().get(&id)
            && workspaces
                .get(w)
                .and_then(|w| w.sessions.get(s))
                .is_some_and(|s| s.id == id)
        {
            return Some((w, s));
        }
        let mut positions = self.positions.borrow_mut();
        positions.clear();
        for (w, workspace) in workspaces.iter().enumerate() {
            for (s, session) in workspace.sessions.iter().enumerate() {
                positions.insert(session.id, (w, s));
            }
        }
        positions.get(&id).copied()
    }
    pub(super) fn revision(&self, id: u64) -> u64 {
        self.revisions.get(&id).copied().unwrap_or(0)
    }
    pub(super) fn changed(&mut self, id: u64) {
        self.global_revision = self.global_revision.wrapping_add(1);
        let version = self.revisions.entry(id).or_default();
        *version = version.wrapping_add(1);
    }
    pub(super) fn begin_import(&mut self, workspace: &Workspace) {
        self.imports.insert(
            workspace.path.clone(),
            workspace
                .sessions
                .iter()
                .map(|s| (s.id, self.revision(s.id)))
                .collect(),
        );
    }
    pub(super) fn unchanged(&self, versions: &HashMap<u64, u64>, id: u64) -> bool {
        versions.get(&id) == Some(&self.revision(id))
    }
}

impl Drop for SessionController {
    fn drop(&mut self) {
        self.cancel_imports();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_generation_detects_completed_turns_and_new_sessions() {
        let mut controller = SessionController::default();
        let versions = HashMap::from([(7, controller.revision(7))]);
        assert!(controller.unchanged(&versions, 7));
        controller.changed(7);
        assert!(!controller.unchanged(&versions, 7));
        assert!(!controller.unchanged(&versions, 8));
    }
}
