use super::*;
use crate::github::threads::tests::snapshot;

#[test]
fn remote_sync_is_idempotent_and_preserves_handling_on_remote_updates() {
    let workspace = Path::new("C:/repo");
    let mut snapshot = snapshot();
    let mut reviews = Reviews::default();
    assert_eq!(reviews.sync_remote(workspace, &snapshot).unwrap().added, 1);
    assert_eq!(reviews.remote[0].status, Status::NeedsReview);
    assert!(!reviews.remote[0].changed());
    reviews.remote[0].set_status(Status::Resolved);
    let again = reviews.sync_remote(workspace, &snapshot).unwrap();
    assert_eq!((again.added, again.updated, again.skipped), (0, 0, 0));
    assert_eq!(reviews.remote.len(), 1);
    let original = snapshot.clone();
    snapshot.fetched_at += 1;
    snapshot.requested_at += 1_000_000_000;
    snapshot.threads[0].is_resolved = true;
    snapshot.threads[0].is_outdated = true;
    snapshot.threads[0].comments[0].body = "Edited comment".into();
    assert_eq!(
        reviews.sync_remote(workspace, &snapshot).unwrap().updated,
        1
    );
    assert_eq!(reviews.remote[0].status, Status::Resolved);
    assert!(reviews.remote[0].changed());
    assert!(
        reviews.remote[0]
            .feedback()
            .unwrap()
            .contains("Edited comment")
    );
    assert_eq!(
        reviews.sync_remote(workspace, &original).unwrap().skipped,
        1
    );
    assert!(reviews.remote[0].thread.is_resolved);
    let mut restored: Reviews =
        serde_json::from_str(&serde_json::to_string(&reviews).unwrap()).unwrap();
    assert!(restored.remote[0].changed());
    restored.remote[0].set_status(Status::Dismissed);
    assert!(!restored.remote[0].changed());
    assert!(restored.comments.is_empty());
    assert!(restored.viewed.is_empty());
}

#[test]
fn remote_sync_scopes_to_workspace_and_pr_and_never_deletes_absent_threads() {
    let mut reviews = Reviews::default();
    let snapshot = snapshot();
    let workspace = Path::new("C:/repo");
    reviews.sync_remote(workspace, &snapshot).unwrap();
    reviews
        .sync_remote(Path::new("C:/other"), &snapshot)
        .unwrap();
    let mut other = snapshot.clone();
    other.pr = "https://github.com/example/other/pull/7".into();
    reviews.sync_remote(workspace, &other).unwrap();
    assert_eq!(reviews.remote.len(), 3);
    assert_ne!(reviews.remote[0].key, reviews.remote[1].key);
    assert_ne!(reviews.remote[0].key, reviews.remote[2].key);
    other.threads.clear();
    other.limited = true;
    reviews.sync_remote(workspace, &other).unwrap();
    assert_eq!(reviews.remote.len(), 3);
    other.limited = false;
    reviews.sync_remote(workspace, &other).unwrap();
    assert_eq!(reviews.remote.len(), 3);
    let legacy: Reviews = serde_json::from_str("{}").unwrap();
    assert!(legacy.remote.is_empty());
}

#[test]
fn remote_sync_capacity_failure_is_atomic_and_preserves_previous_evidence() {
    let workspace = Path::new("C:/repo");
    let mut snapshot = snapshot();
    let mut reviews = Reviews::default();
    reviews.sync_remote(workspace, &snapshot).unwrap();
    let original = reviews.remote[0].clone();
    reviews.remote = vec![original; MAX_SAVED];
    snapshot.threads[0].comments[0].body = "Changed".into();
    let mut new = snapshot.threads[0].clone();
    new.id = "another".into();
    snapshot.threads.push(new);
    assert!(reviews.sync_remote(workspace, &snapshot).is_err());
    assert_eq!(reviews.remote.len(), MAX_SAVED);
    assert_eq!(reviews.remote[0].thread.comments[0].body, "Check input 1");
    reviews.remote.truncate(1);
    snapshot.threads[0].comments[0].body = "x".repeat(MAX_BYTES);
    assert!(reviews.sync_remote(workspace, &snapshot).is_err());
    assert_eq!(reviews.remote.len(), 1);
    assert_eq!(reviews.remote[0].thread.comments[0].body, "Check input 1");
}
