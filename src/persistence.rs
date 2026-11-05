//! One writer owns disk I/O. Ordinary saves coalesce around durable send barriers.
use crate::store::Store;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};

struct Snapshot {
    revision: u64,
    store: Store,
    receipt: Option<mpsc::Sender<Result<(), String>>>,
}
/// A receipt belongs to one immutable snapshot, never a later coalesced save.
pub(crate) struct SaveReceipt(mpsc::Receiver<Result<(), String>>);
impl SaveReceipt {
    pub(crate) fn poll(&self) -> Option<Result<(), String>> {
        match self.0.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("保存线程已退出".into())),
        }
    }
}
pub(crate) struct Persistence {
    pending: Arc<Mutex<VecDeque<Snapshot>>>,
    wake: mpsc::SyncSender<()>,
    completed: mpsc::Receiver<(u64, Result<(), String>)>,
    requested: u64,
    acknowledged: u64,
    pub(crate) error: Option<String>,
    // Additional immutable message versions retained behind the largest snapshot.
    // One large library is always saveable; slow writers cannot accumulate unlimited copies.
    extra_bytes_budget: usize,
}

impl Persistence {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self::with_writer(move |store| store.save_to(&path).map_err(|e| format!("{e:#}")))
    }
    pub(crate) fn with_writer(
        write: impl Fn(Store) -> Result<(), String> + Send + 'static,
    ) -> Self {
        let pending = Arc::new(Mutex::new(VecDeque::<Snapshot>::new()));
        let queue = pending.clone();
        let (wake, waits) = mpsc::sync_channel(1);
        let (tx, completed) = mpsc::channel();
        thread::spawn(move || {
            while waits.recv().is_ok() {
                loop {
                    let Some(snapshot) = queue.lock().unwrap().pop_front() else {
                        break;
                    };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        write(snapshot.store)
                    }))
                    .unwrap_or_else(|_| Err("保存线程异常".into()));
                    if let Some(receipt) = snapshot.receipt {
                        let _ = receipt.send(result.clone());
                    }
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
            extra_bytes_budget: 128 * 1024 * 1024,
        }
    }
    pub(crate) fn submit(&mut self, store: &Store) -> bool {
        self.enqueue(store, None)
    }
    pub(crate) fn submit_confirmed(&mut self, store: &Store) -> Result<SaveReceipt, String> {
        let (sender, receiver) = mpsc::channel();
        if self.enqueue(store, Some(sender)) {
            Ok(SaveReceipt(receiver))
        } else {
            Err(self
                .error
                .clone()
                .unwrap_or_else(|| "保存线程已退出".into()))
        }
    }
    fn enqueue(
        &mut self,
        store: &Store,
        receipt: Option<mpsc::Sender<Result<(), String>>>,
    ) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if receipt.is_some() && pending.len() >= 32 {
            self.error = Some("待保存消息过多，请等待磁盘写入完成。".into());
            return false;
        }
        let replace = pending
            .back()
            .is_some_and(|snapshot| snapshot.receipt.is_none());
        let keep = pending.len() - usize::from(replace);
        let (retained, largest) = if keep == 0 {
            (0, 0)
        } else {
            retained_bytes(
                pending
                    .iter()
                    .take(keep)
                    .map(|s| &s.store)
                    .chain(std::iter::once(store)),
            )
        };
        if retained.saturating_sub(largest) > self.extra_bytes_budget {
            self.error = Some("待保存正文占用过多，请等待磁盘写入完成后重试。".into());
            return false;
        }
        self.requested += 1;
        // Ordinary saves can coalesce, but a send barrier must actually be
        // written even if a later snapshot cancels or edits that pending send.
        if replace {
            pending.pop_back();
        }
        pending.push_back(Snapshot {
            revision: self.requested,
            store: store.clone(),
            receipt,
        });
        drop(pending);
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

fn retained_bytes<'a>(stores: impl Iterator<Item = &'a Store>) -> (usize, usize) {
    let mut identities = HashSet::new();
    let mut total = 0;
    let mut largest = 0;
    for store in stores {
        let mut size = 0;
        for item in store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .flat_map(|s| &s.messages)
        {
            let bytes = crate::transcript::resident_bytes(std::slice::from_ref(item));
            size += bytes;
            if identities.insert(item.identity().as_ptr()) {
                total += bytes;
            }
        }
        largest = largest.max(size);
    }
    (total, largest)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_writer_bounds_retained_versions_without_losing_send_barriers() {
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut writer = Persistence::with_writer(move |store| {
            entered.send(store.version).unwrap();
            wait.recv_timeout(std::time::Duration::from_secs(3))
                .unwrap();
            Ok(())
        });
        writer.extra_bytes_budget = 8 * 1024;
        writer.submit(&Store::default());
        started.recv().unwrap();
        let fixture = crate::transcript::tests::Fixture::new();
        let mut store = fixture.store(2);
        store.workspaces[0].sessions[0].messages[1].text = "a".repeat(16 * 1024);
        let before = std::time::Instant::now();
        let receipt = writer.submit_confirmed(&store).unwrap();
        // Sharing the same bodies adds no retention pressure.
        let shared = writer.submit_confirmed(&store).unwrap();
        store.workspaces[0].sessions[0].messages[1].text = "b".repeat(16 * 1024);
        assert!(writer.submit_confirmed(&store).is_err());
        let pending = writer.pending.lock().unwrap();
        assert_eq!(pending.len(), 2);
        let (bytes, largest) = retained_bytes(pending.iter().map(|s| &s.store));
        assert_eq!(bytes, largest);
        eprintln!(
            "slow writer: queued message bytes={bytes}, additional versions={}",
            bytes - largest
        );
        drop(pending);
        assert!(receipt.poll().is_none());
        release.send(()).unwrap();
        started.recv().unwrap();
        assert!(receipt.poll().is_none());
        release.send(()).unwrap();
        started.recv().unwrap();
        release.send(()).unwrap();
        writer.flush();
        assert_eq!(receipt.poll(), Some(Ok(())));
        assert_eq!(shared.poll(), Some(Ok(())));
        eprintln!(
            "controlled send confirmation latency: {:?}",
            before.elapsed()
        );
    }
    #[test]
    fn send_receipts_survive_coalescing_and_keep_their_own_failure() {
        let (entered, started) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut writer = Persistence::with_writer(move |store| {
            entered.send(store.version).unwrap();
            wait.recv_timeout(std::time::Duration::from_secs(3))
                .unwrap();
            if store.version == 2 {
                Err("disk full".into())
            } else {
                Ok(())
            }
        });
        let mut store = Store {
            version: 1,
            ..Default::default()
        };
        writer.submit(&store);
        assert_eq!(started.recv().unwrap(), 1);
        store.version = 2;
        let failed = writer.submit_confirmed(&store).unwrap();
        store.version = 3;
        let saved = writer.submit_confirmed(&store).unwrap();
        store.version = 4;
        writer.submit(&store);
        store.version = 5;
        writer.submit(&store);
        assert!(failed.poll().is_none());
        assert!(saved.poll().is_none());
        for version in [2, 3, 5] {
            release.send(()).unwrap();
            assert_eq!(
                started
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap(),
                version
            );
        }
        release.send(()).unwrap();
        writer.flush();
        assert_eq!(failed.poll(), Some(Err("disk full".into())));
        assert_eq!(saved.poll(), Some(Ok(())));
        assert!(writer.error.is_none());
    }

    #[test]
    fn writer_panic_reports_failed_receipt_and_accepts_a_later_retry() {
        let mut writer = Persistence::with_writer(|store| {
            assert_ne!(store.version, 1, "injected writer panic");
            Ok(())
        });
        let failed = writer
            .submit_confirmed(&Store {
                version: 1,
                ..Default::default()
            })
            .unwrap();
        let saved = writer.submit_confirmed(&Store::default()).unwrap();
        writer.flush();
        assert_eq!(failed.poll(), Some(Err("保存线程异常".into())));
        assert_eq!(saved.poll(), Some(Ok(())));
    }
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
