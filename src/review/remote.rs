//! Remote discussion snapshots have independent local handling state and no local Diff anchor.
use super::*;
use crate::github::threads::{Snapshot, Thread};
use std::path::Path;

const MAX_SAVED: usize = 500;
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedThread {
    pub key: String,
    pub workspace: PathBuf,
    pub pr: String,
    pub head: String,
    pub fetched_at: u64,
    pub requested_at: u64,
    pub thread: Thread,
    pub status: Status,
    acknowledged: String,
}

impl SavedThread {
    fn fingerprint(&self) -> String {
        fingerprint(
            &serde_json::to_string(&(&self.head, &self.thread))
                .expect("serializable review snapshot"),
        )
    }

    pub fn changed(&self) -> bool {
        self.acknowledged != self.fingerprint()
    }

    pub fn set_status(&mut self, status: Status) {
        self.status = status;
        self.acknowledged = self.fingerprint();
    }

    pub fn feedback(&self) -> Result<String, String> {
        let mut text = self
            .thread
            .feedback(&self.pr, &self.head)
            .map_err(|e| e.to_string())?;
        text.push_str(&conduit::i18n::format(
            "github.thread_local_status",
            &[("status", self.status.label().into())],
        ));
        text.push('\n');
        if self.changed() {
            text.push_str(&problem("github.thread_updated"));
        }
        if text.len() > MAX_FEEDBACK_BYTES {
            return Err(problem("review.feedback_limit"));
        }
        Ok(text)
    }
}

#[derive(Default)]
pub struct Synced {
    pub added: usize,
    pub updated: usize,
    pub skipped: usize,
}

impl Reviews {
    /// Commit a whole snapshot locally. Missing threads are retained, including on partial pages.
    /// Older results from another open panel cannot overwrite newer observations.
    pub fn sync_remote(&mut self, workspace: &Path, snapshot: &Snapshot) -> Result<Synced, String> {
        let mut result = self.remote.clone();
        let mut synced = Synced::default();
        for thread in &snapshot.threads {
            let key = fingerprint(
                &serde_json::to_string(&(workspace, &snapshot.pr, &thread.id)).unwrap(),
            );
            if let Some(existing) = result.iter_mut().find(|r| r.key == key) {
                if existing.requested_at > snapshot.requested_at {
                    synced.skipped += 1;
                    continue;
                }
                synced.updated +=
                    usize::from(existing.thread != *thread || existing.head != snapshot.head);
                existing.thread = thread.clone();
                existing.head = snapshot.head.clone();
                existing.fetched_at = snapshot.fetched_at;
                existing.requested_at = snapshot.requested_at;
            } else {
                let mut saved = SavedThread {
                    key,
                    workspace: workspace.into(),
                    pr: snapshot.pr.clone(),
                    head: snapshot.head.clone(),
                    fetched_at: snapshot.fetched_at,
                    requested_at: snapshot.requested_at,
                    thread: thread.clone(),
                    status: Status::NeedsReview,
                    acknowledged: String::new(),
                };
                saved.set_status(Status::NeedsReview);
                result.push(saved);
                synced.added += 1;
            }
        }
        if result.len() > MAX_SAVED
            || serde_json::to_vec(&result)
                .map_err(|e| e.to_string())?
                .len()
                > MAX_BYTES
        {
            return Err(problem("github.thread_storage_limit"));
        }
        self.remote = result;
        Ok(synced)
    }
}

#[cfg(test)]
mod tests;
