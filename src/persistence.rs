//! One writer owns disk I/O; a single pending snapshot coalesces UI saves.
use crate::store::Store;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

struct Snapshot {
    revision: u64,
    store: Store,
}
pub(crate) struct Persistence {
    pending: Arc<Mutex<Option<Snapshot>>>,
    wake: mpsc::SyncSender<()>,
    completed: mpsc::Receiver<(u64, Result<(), String>)>,
    requested: u64,
    acknowledged: u64,
    pub(crate) error: Option<String>,
}

impl Persistence {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self::with_writer(move |store| store.save_to(&path).map_err(|e| format!("{e:#}")))
    }
    fn with_writer(write: impl Fn(Store) -> Result<(), String> + Send + 'static) -> Self {
        let pending = Arc::new(Mutex::new(None::<Snapshot>));
        let queue = pending.clone();
        let (wake, waits) = mpsc::sync_channel(1);
        let (tx, completed) = mpsc::channel();
        thread::spawn(move || {
            while waits.recv().is_ok() {
                let snapshot = queue.lock().unwrap().take();
                if let Some(snapshot) = snapshot {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        write(snapshot.store)
                    }))
                    .unwrap_or_else(|_| Err("保存线程异常".into()));
                    let _ = tx.send((snapshot.revision, result));
                }
            }
        });
        Self {
            pending,
            wake,
            completed,
            requested: 0,
            acknowledged: 0,
            error: None,
        }
    }
    pub(crate) fn submit(&mut self, store: &Store) -> bool {
        self.requested += 1;
        *self.pending.lock().unwrap() = Some(Snapshot {
            revision: self.requested,
            store: store.clone(),
        });
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => true,
            Err(mpsc::TrySendError::Disconnected(())) => {
                self.error = Some("保存线程已退出".into());
                false
            }
        }
    }
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((revision, result)) = self.completed.try_recv() {
            changed = true;
            self.acknowledged = self.acknowledged.max(revision);
            if revision == self.requested {
                self.error = result.err();
            }
        }
        changed
    }
    pub(crate) fn busy(&self) -> bool {
        self.acknowledged < self.requested
    }
    #[cfg(test)]
    pub(crate) fn flush(&mut self) {
        while self.busy() {
            let (revision, result) = self
                .completed
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            self.acknowledged = self.acknowledged.max(revision);
            if revision == self.requested {
                self.error = result.err();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_writer_coalesces_snapshots_and_acknowledges_latest_revision() {
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (saved, results) = mpsc::channel();
        let mut writer = Persistence::with_writer(move |store| {
            entered.send(()).unwrap();
            wait.recv().unwrap();
            saved.send(store.version).unwrap();
            Ok(())
        });
        let mut store = Store {
            version: 1,
            ..Default::default()
        };
        assert!(writer.submit(&store));
        started
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        store.version = 2;
        writer.submit(&store);
        store.version = 3;
        writer.submit(&store);
        assert!(writer.busy());
        release.send(()).unwrap();
        assert_eq!(results.recv().unwrap(), 1);
        started.recv().unwrap();
        release.send(()).unwrap();
        assert_eq!(results.recv().unwrap(), 3);
        writer.flush();
        assert!(!writer.busy());
        assert!(writer.error.is_none());
    }
    #[test]
    fn failed_save_remains_visible_until_a_successful_retry() {
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let mut writer = Persistence::with_writer(move |_| {
            if calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                Err("disk full".into())
            } else {
                Ok(())
            }
        });
        writer.submit(&Store::default());
        writer.flush();
        assert_eq!(writer.error.as_deref(), Some("disk full"));
        writer.submit(&Store::default());
        writer.flush();
        assert!(writer.error.is_none());
    }
}
