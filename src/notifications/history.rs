//! Bounded, separately locked notification history. All writes run off the UI thread.
use super::{Center, Snapshot};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
type Pending = Arc<Mutex<Option<(u64, Snapshot)>>>;

pub(crate) struct History {
    pending: Pending,
    wake: Option<mpsc::SyncSender<()>>,
    completed: mpsc::Receiver<(u64, Result<(), String>)>,
    requested: u64,
    acknowledged: u64,
    last_submit: Instant,
    pub error: Option<String>,
}

fn load(path: &Path, now: u64) -> Result<Center, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Center::default()),
        Err(error) => {
            return Err(conduit::i18n::format(
                "notifications.history.unable_to_read_notification_history_original_file",
                &[("error", format!("{}", error))],
            ));
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(conduit::i18n::text_static(
            "notifications.history.notification_history_exceeds_capacity_original_file_preserved",
        )
        .into());
    }
    let snapshot = serde_json::from_slice(&bytes).map_err(|_| {
        conduit::i18n::text_static(
            "notifications.history.notification_history_is_corrupt_original_file_preserved",
        )
        .to_string()
    })?;
    Center::restore(snapshot, now)
}

impl History {
    pub(crate) fn open(path: PathBuf, writable: bool, now: u64) -> (Center, Self) {
        let (sender, completed) = mpsc::channel();
        let pending = Arc::new(Mutex::new(None));
        let mut history = Self {
            pending,
            wake: None,
            completed,
            requested: 0,
            acknowledged: 0,
            last_submit: Instant::now(),
            error: None,
        };
        // Lock before loading so this snapshot cannot overwrite another process's changes.
        let lock = if writable {
            let locked = (|| -> Result<File, String> {
                fs::create_dir_all(path.parent().ok_or(conduit::i18n::text_static(
                    "notifications.history.invalid_notification_history_directory",
                ))?)
                .map_err(|error| error.to_string())?;
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(path.with_extension("lock"))
                    .map_err(|error| error.to_string())?;
                lock.try_lock().map_err(|_| {
                    conduit::i18n::text_static(
                        "notifications.history.notification_history_is_in_use_by_another",
                    )
                    .to_string()
                })?;
                Ok(lock)
            })();
            match locked {
                Ok(lock) => Some(lock),
                Err(error) => {
                    history.error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        let center = match load(&path, now) {
            Ok(center) => center,
            Err(error) => {
                history.error = Some(error);
                return (Center::default(), history);
            }
        };
        if let Some(lock) = lock {
            let (wake, receiver) = mpsc::sync_channel(1);
            let pending = history.pending.clone();
            std::thread::spawn(move || {
                let _lock = lock;
                while receiver.recv().is_ok() {
                    loop {
                        let Some((revision, snapshot)) = pending.lock().unwrap().take() else {
                            break;
                        };
                        let result = save(&path, &snapshot).map_err(|error| {
                            conduit::i18n::format(
                                "notifications.history.failed_to_save_notification_history",
                                &[("error", format!("{}", error))],
                            )
                        });
                        let _ = sender.send((revision, result));
                    }
                }
                // Drop may enqueue a final snapshot before disconnecting the wake channel.
                if let Some((_, snapshot)) = pending.lock().unwrap().take() {
                    let _ = save(&path, &snapshot);
                }
            });
            history.wake = Some(wake);
        }
        (center, history)
    }
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((revision, result)) = self.completed.try_recv() {
            self.acknowledged = self.acknowledged.max(revision);
            if revision == self.requested {
                self.error = result.err();
                changed = true;
            }
        }
        changed
    }
    pub(crate) fn submit(&mut self, center: &Center, force: bool) {
        let Some(wake) = &self.wake else { return };
        if center.revision() == self.requested && self.error.is_none() {
            return;
        }
        let interval = if self.error.is_some() {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(500)
        };
        if !force && self.last_submit.elapsed() < interval {
            return;
        }
        // A failed close attempt must not continuously requeue the same revision.
        if force && self.requested == center.revision() {
            return;
        }
        self.requested = center.revision();
        self.last_submit = Instant::now();
        *self.pending.lock().unwrap() = Some((self.requested, center.snapshot()));
        if let Err(mpsc::TrySendError::Disconnected(())) = wake.try_send(()) {
            self.error = Some(
                conduit::i18n::text_static(
                    "notifications.history.notification_history_save_thread_exited",
                )
                .into(),
            );
            self.wake = None;
        }
    }
    pub(crate) fn busy(&self) -> bool {
        self.wake.is_some() && self.acknowledged < self.requested
    }
}

fn save(path: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(snapshot)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::other(conduit::i18n::text_static(
            "notifications.history.history_exceeds_capacity",
        )));
    }
    let mut file = File::create(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(not(windows))]
    fs::rename(temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::{Level, Notice, Source};
    fn path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "conduit-notifications-{}-{name}",
                std::process::id()
            ))
            .join("notifications.json")
    }
    fn flush(history: &mut History) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while history.busy() {
            assert!(Instant::now() < deadline);
            history.poll();
            std::thread::yield_now();
        }
    }
    #[test]
    fn history_roundtrips_without_touching_the_conversation_store() {
        let _locale = crate::test_support::chinese();
        let path = path("roundtrip");
        let (mut center, mut history) = History::open(path.clone(), true, 100);
        let id = center
            .publish(Notice::new(Source::App, Level::Info, "已完成", "内容"), 100)
            .id;
        center.mark_read(id, 101);
        history.submit(&center, true);
        flush(&mut history);
        assert!(history.error.is_none());
        let (restored, readonly) = History::open(path.clone(), false, 102);
        assert!(readonly.wake.is_none());
        assert_eq!(restored.get(id).unwrap().read_at, Some(101));
        assert!(!path.with_file_name("state.json").exists());
    }
    #[test]
    fn failed_history_write_recovers_without_recursively_publishing() {
        let _locale = crate::test_support::chinese();
        let path = path("write-recovery");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let blocker = path.with_extension("json.tmp");
        fs::create_dir(&blocker).unwrap();
        let (mut center, mut history) = History::open(path.clone(), true, 100);
        let id = center
            .publish(Notice::new(Source::App, Level::Info, "结果", "记录"), 100)
            .id;
        history.submit(&center, true);
        flush(&mut history);
        assert!(history.error.is_some());
        assert_eq!(center.records().len(), 1);
        fs::remove_dir(&blocker).unwrap();
        center.mark_read(id, 101);
        history.submit(&center, true);
        flush(&mut history);
        assert!(history.error.is_none());
        assert_eq!(
            load(&path, 102).unwrap().get(id).unwrap().read_at,
            Some(101)
        );
    }

    #[test]
    fn corruption_and_a_second_writer_never_overwrite_history() {
        let _locale = crate::test_support::chinese();
        let path = path("corrupt");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"invalid-json").unwrap();
        let (mut center, mut history) = History::open(path.clone(), true, 100);
        assert!(history.error.is_some());
        center.publish(Notice::new(Source::App, Level::Error, "错误", "测试"), 100);
        history.submit(&center, true);
        assert_eq!(fs::read(&path).unwrap(), b"invalid-json");
        let good = path.with_file_name("other.json");
        let (_, _first) = History::open(good.clone(), true, 100);
        let (_, second) = History::open(good, true, 100);
        assert!(second.wake.is_none());
        assert!(second.error.is_some());
    }
}
