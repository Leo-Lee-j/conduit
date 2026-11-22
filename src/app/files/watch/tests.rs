use super::*;

fn signals() -> Signals {
    Signals {
        generation: 4,
        targets: vec![(1, "a/file.txt".into()), (2, "b/file.txt".into())],
        ..Default::default()
    }
}

#[test]
fn content_watch_debounces_bursts_and_bounds_continuous_writes() {
    let mut signals = signals();
    let start = Instant::now();
    signals.signal(4, [1], start);
    signals.signal(4, [1, 2], start + Duration::from_millis(50));
    assert!(signals.take(start + Duration::from_millis(100)).is_empty());
    let mut ready = signals.take(start + Duration::from_millis(125));
    ready.sort_unstable();
    assert_eq!(ready, [1, 2]);
    assert!(signals.take(start + Duration::from_secs(1)).is_empty());
    for millis in (0..=250).step_by(25) {
        signals.signal(4, [1], start + Duration::from_millis(millis));
    }
    assert_eq!(signals.take(start + MAX_DELAY), [1]);
}

#[test]
fn content_watch_pending_is_bounded_and_rejects_old_generations_and_shutdown() {
    let mut signals = signals();
    let now = Instant::now();
    for id in 0..100_000 {
        signals.signal(4, [id, 1, 2], now);
    }
    assert_eq!(signals.dirty.len(), 2);
    signals.dirty.clear();
    signals.generation += 1;
    signals.targets.remove(0);
    signals.signal(4, [1, 2], now);
    signals.signal(5, [1, 2], now);
    assert_eq!(signals.take(now + MAX_DELAY), [2]);
    signals.stopped = true;
    signals.signal(5, [2], now);
    assert!(signals.dirty.is_empty());
}

#[test]
fn content_watch_target_changes_clear_pending_and_keep_stable_order() {
    let mut watcher = Watcher::default();
    // Keep this pure: no native thread is started for the manually staged state.
    {
        let mut shared = watcher.shared.lock().unwrap();
        shared.targets = vec![(1, "a".into()), (2, "b".into())];
        shared.signal(0, [1, 2], Instant::now());
    }
    watcher.retry_at = Some(Instant::now() + Duration::from_secs(60));
    watcher.sync(vec![(2, "b".into()), (1, "a".into())]);
    assert_eq!(watcher.shared.lock().unwrap().generation, 0);
    watcher.sync(vec![(2, "new/b".into())]);
    let shared = watcher.shared.lock().unwrap();
    assert_eq!(shared.generation, 1);
    assert!(shared.dirty.is_empty());
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "conduit-content-watch-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn await_ids(watcher: &mut Watcher, expected: &[u64]) {
        // Faster than registration retry: a failed subscription must not make
        // ordinary notification assertions pass via its periodic forced reads.
        await_ids_for(watcher, expected, Duration::from_millis(1500));
    }
    fn await_ids_for(watcher: &mut Watcher, expected: &[u64], timeout: Duration) {
        let start = Instant::now();
        let mut received = Vec::new();
        while start.elapsed() < timeout {
            received.extend(watcher.take_changes(Instant::now()));
            if expected.iter().all(|id| received.contains(id)) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out awaiting {expected:?}; received {received:?}");
    }
    fn overwrite_same_stamp(path: &Path, text: &str) {
        let before = crate::workspace_files::stamp(path).unwrap();
        fs::write(path, text).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(before.modified.unwrap()))
            .unwrap();
        assert_eq!(crate::workspace_files::stamp(path).unwrap(), before);
    }

    #[test]
    fn content_watch_native_overwrite_replace_rename_recreate_and_both_files() {
        let directory = Directory::new();
        let left = directory.0.join("left");
        let right = directory.0.join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        let a = left.join("a.txt");
        let b = right.join("b.txt");
        fs::write(&a, "old").unwrap();
        fs::write(&b, "old").unwrap();
        let mut watcher = Watcher::default();
        watcher.sync(vec![(1, a.clone()), (2, b.clone())]);
        await_ids(&mut watcher, &[1, 2]);
        overwrite_same_stamp(&a, "new");
        await_ids(&mut watcher, &[1]);
        let temp = left.join("atomic.tmp");
        fs::write(&temp, "atomic replacement").unwrap();
        crate::store::replace_file(&temp, &a).unwrap();
        await_ids(&mut watcher, &[1]);
        fs::rename(&a, left.join("renamed.txt")).unwrap();
        await_ids(&mut watcher, &[1]);
        fs::write(&a, "recreated").unwrap();
        await_ids(&mut watcher, &[1]);
        fs::remove_file(&a).unwrap();
        await_ids(&mut watcher, &[1]);
        fs::write(&a, "both a").unwrap();
        fs::write(&b, "both b").unwrap();
        await_ids(&mut watcher, &[1, 2]);
        watcher.sync(Vec::new());
        fs::write(&a, "after removal").unwrap();
        thread::sleep(Duration::from_millis(200));
        assert!(watcher.take_changes(Instant::now()).is_empty());
        let shared = watcher.shared.clone();
        let worker = watcher.worker.take().unwrap();
        drop(watcher);
        worker.thread().unpark();
        // Joining is safe here on the test thread, never on the UI thread.
        worker.join().unwrap();
        assert!(shared.lock().unwrap().stopped);
    }

    #[test]
    fn content_watch_rearms_after_directory_replacement_and_registration_failure() {
        let directory = Directory::new();
        let parent = directory.0.join("current");
        fs::create_dir_all(&parent).unwrap();
        let path = parent.join("a.txt");
        fs::write(&path, "old").unwrap();
        let mut watcher = Watcher::default();
        watcher.sync(vec![(1, path.clone())]);
        await_ids(&mut watcher, &[1]);
        fs::rename(&parent, directory.0.join("previous")).unwrap();
        fs::create_dir_all(&parent).unwrap();
        fs::write(&path, "new directory").unwrap();
        await_ids(&mut watcher, &[1]);
        // Require a later content notification from the newly opened handle.
        thread::sleep(Duration::from_millis(200));
        watcher.take_changes(Instant::now() + MAX_DELAY);
        fs::write(&path, "later content").unwrap();
        await_ids(&mut watcher, &[1]);

        let missing = directory.0.join("missing/nested/a.txt");
        watcher.sync(vec![(2, missing.clone())]);
        await_ids(&mut watcher, &[2]); // Failed subscription still requests a read.
        fs::create_dir_all(missing.parent().unwrap()).unwrap();
        fs::write(&missing, "created later").unwrap();
        await_ids_for(&mut watcher, &[2], Duration::from_secs(5)); // Retry closes the unsubscribed interval.
        fs::write(&missing, "after retry").unwrap();
        await_ids(&mut watcher, &[2]);
    }
}
