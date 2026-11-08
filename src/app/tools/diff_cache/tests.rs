use super::{DiffCache, MAX_ENTRIES, MAX_PATCH_BYTES};
use gpui_kit::TestAppContext;

fn patch(text: &str) -> String {
    format!("--- a/review.rs\n+++ b/review.rs\n@@ -1 +1 @@\n-old\n+{text}\n")
}

#[gpui_kit::test]
fn diff_cache_reuses_replaces_and_evicts_least_recent_card(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "diff-cache-lru");
    cx.update(|_, cx| {
        app.update(cx, |_, cx| {
            let mut cache = DiffCache::default();
            let key = (1, "0".to_string());
            let first = cache.state(&key, &patch("first"), cx).unwrap();
            assert_eq!(cache.state(&key, &patch("first"), cx).unwrap(), first);
            let replacement = cache.state(&key, &patch("replacement"), cx).unwrap();
            assert_ne!(replacement, first);
            for index in 1..MAX_ENTRIES {
                cache
                    .state(&(1, index.to_string()), &patch("small"), cx)
                    .unwrap();
            }
            cache.state(&key, &patch("replacement"), cx).unwrap();
            cache.state(&(2, "new".into()), &patch("new"), cx).unwrap();
            assert_eq!(cache.entries.len(), MAX_ENTRIES);
            assert!(cache.entries.contains_key(&key));
            assert!(!cache.entries.contains_key(&(1, "1".into())));
            cache.forget_session(1);
            assert_eq!(cache.entries.len(), 1);
            assert!(cache.entries.contains_key(&(2, "new".into())));
        })
    });
}

#[gpui_kit::test]
fn diff_cache_bounds_patch_bytes_and_does_not_retain_oversized_cards(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "diff-cache-bytes");
    cx.update(|_, cx| {
        app.update(cx, |_, cx| {
            let mut cache = DiffCache::default();
            let large = patch(&"x".repeat(MAX_PATCH_BYTES / 2));
            cache.state(&(1, "first".into()), &large, cx).unwrap();
            cache.state(&(1, "second".into()), &large, cx).unwrap();
            assert_eq!(cache.entries.len(), 1);
            assert!(cache.entries.contains_key(&(1, "second".into())));
            assert!(
                cache
                    .entries
                    .values()
                    .map(|entry| entry.patch.len())
                    .sum::<usize>()
                    <= MAX_PATCH_BYTES
            );
            let oversized = patch(&"x".repeat(MAX_PATCH_BYTES));
            assert!(
                cache
                    .state(&(2, "oversized".into()), &oversized, cx)
                    .is_some()
            );
            assert_eq!(cache.entries.len(), 1);
            assert!(!cache.entries.contains_key(&(2, "oversized".into())));
        })
    });
}

#[gpui_kit::test]
fn deleting_session_and_removing_workspace_release_cached_diff_entities(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "diff-cache-lifecycle");
    let (first, second) = cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.writable = false;
            let first = app
                .tool_diffs
                .state(&(1, "first".into()), &patch("first"), cx)
                .unwrap()
                .downgrade();
            let second = app
                .tool_diffs
                .state(&(2, "second".into()), &patch("second"), cx)
                .unwrap()
                .downgrade();
            app.delete_session(1, cx);
            assert!(app.session(1).is_none());
            assert_eq!(app.tool_diffs.entries.len(), 1);
            app.remove_workspace(1, cx);
            assert!(app.session(2).is_none());
            assert!(app.tool_diffs.entries.is_empty());
            (first, second)
        })
    });
    cx.run_until_parked();
    assert!(first.upgrade().is_none());
    assert!(second.upgrade().is_none());
}
