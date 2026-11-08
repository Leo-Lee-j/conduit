use super::ToolDetailState;
use conduit::plugins::context::{SessionKey, WorkspaceKey};
use gpui_kit::{point, px};

fn key(workspace: &str, id: u64) -> SessionKey {
    SessionKey {
        workspace: WorkspaceKey::from_host_path(std::env::temp_dir().join(workspace)).unwrap(),
        id,
    }
}

#[test]
fn cache_bounds_selection_and_scrolls_and_evicts_least_recently_used() {
    let _locale = crate::test_support::chinese();
    let mut state = ToolDetailState::default();
    let first = key("tool-cache", 0);
    state.select(&first, "shared");
    state
        .scroll(&first, "shared")
        .set_offset(point(px(0.), px(-42.)));
    for id in 1..32 {
        state.select(&key("tool-cache", id), "shared");
    }
    // Reopening an older tool makes it recent; adding a new entry evicts session 1.
    state.select(&first, "shared");
    state.select(&key("tool-cache", 32), "shared");
    assert_eq!(state.scrolls.len(), 32);
    assert_eq!(state.selected.len(), 32);
    assert!(state.selected(&key("tool-cache", 1)).is_none());
    assert_eq!(state.scroll(&first, "shared").offset().y, px(-42.));
    assert_eq!(state.selected(&key("tool-cache", 32)), Some("shared"));

    // Changing the tool within one session keeps its latest selection when an
    // older tool in that session is evicted.
    state.select(&first, "new");
    for id in 33..65 {
        state.select(&key("tool-cache", id), "shared");
        state.scroll(&first, "new");
    }
    assert!(
        !state
            .scrolls
            .contains_key(&(first.clone(), "shared".into()))
    );
    assert_eq!(state.selected(&first), Some("new"));
    assert_eq!(state.scrolls.len(), 32);
    assert!(state.selected.len() <= 32);
}

#[test]
fn workspace_identity_isolates_same_session_and_message_ids_and_deletion_cleans_state() {
    let _locale = crate::test_support::chinese();
    let mut state = ToolDetailState::default();
    let alpha = key("tool-alpha", 7);
    let beta = key("tool-beta", 7);
    let other = key("tool-alpha", 8);
    state.select(&alpha, "shared");
    state
        .scroll(&alpha, "shared")
        .set_offset(point(px(0.), px(-90.)));
    assert!(state.selected(&beta).is_none());
    state.select(&beta, "shared");
    assert_eq!(state.scroll(&beta, "shared").offset().y, px(0.));
    assert_eq!(state.scroll(&alpha, "shared").offset().y, px(-90.));
    state.select(&other, "keep");
    state.forget_session(7);
    assert!(state.selected(&alpha).is_none());
    assert!(state.selected(&beta).is_none());
    assert_eq!(state.selected(&other), Some("keep"));
    assert_eq!(state.scrolls.len(), 1);
}
