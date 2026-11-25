//! Bounded, generation-scoped directory notifications for visible previews.
//! Native I/O stays on one worker; the UI only exchanges paths and dirty ids.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const DEBOUNCE: Duration = Duration::from_millis(75);
const MAX_DELAY: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Signals {
    generation: u64,
    targets: Vec<(u64, PathBuf)>,
    dirty: HashMap<u64, (Instant, Instant)>,
    stopped: bool,
}

impl Signals {
    fn signal(&mut self, generation: u64, ids: impl IntoIterator<Item = u64>, now: Instant) {
        if self.stopped || generation != self.generation {
            return;
        }
        for id in ids {
            if self.targets.iter().any(|(current, _)| *current == id) {
                self.dirty
                    .entry(id)
                    .and_modify(|(_, last)| *last = now)
                    .or_insert((now, now));
            }
        }
    }

    fn take(&mut self, now: Instant) -> Vec<u64> {
        let mut ready = Vec::with_capacity(self.dirty.len());
        self.dirty.retain(|id, (first, last)| {
            if now.saturating_duration_since(*last) >= DEBOUNCE
                || now.saturating_duration_since(*first) >= MAX_DELAY
            {
                ready.push(*id);
                false
            } else {
                true
            }
        });
        ready
    }
}

#[derive(Default)]
pub(super) struct Watcher {
    shared: Arc<Mutex<Signals>>,
    worker: Option<JoinHandle<()>>,
    retry_at: Option<Instant>,
}

impl Watcher {
    pub(super) fn sync(&mut self, mut targets: Vec<(u64, PathBuf)>) {
        targets.sort_by_key(|(id, _)| *id);
        let changed = {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            if shared.targets == targets {
                false
            } else {
                shared.generation = shared.generation.wrapping_add(1);
                shared.targets = targets;
                shared.dirty.clear();
                true
            }
        };
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            self.worker.take();
        }
        if self.worker.is_none()
            && !self
                .shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .targets
                .is_empty()
            && self.retry_at.is_none_or(|at| Instant::now() >= at)
        {
            self.retry_at = Some(Instant::now() + RETRY);
            let shared = self.shared.clone();
            self.worker = thread::Builder::new()
                .name("file-preview-watch".into())
                .spawn(move || run(shared))
                .ok();
        }
        if changed && let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }

    pub(super) fn take_changes(&mut self, now: Instant) -> Vec<u64> {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(now)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stopped = true;
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        // Never join filesystem I/O on the UI thread, including network paths.
    }
}

#[cfg(windows)]
mod native;

#[cfg(windows)]
fn run(shared: Arc<Mutex<Signals>>) {
    use std::collections::BTreeMap;

    struct Registration {
        contents: bool,
        ids: Vec<u64>,
        handle: Option<native::Notification>,
        retry_at: Instant,
    }
    let mut generation = u64::MAX;
    let mut registrations = BTreeMap::<PathBuf, Registration>::new();
    loop {
        let targets = {
            let shared = shared.lock().unwrap_or_else(|e| e.into_inner());
            if shared.stopped {
                return;
            }
            (shared.generation != generation).then(|| (shared.generation, shared.targets.clone()))
        };
        if let Some((next, targets)) = targets {
            generation = next;
            registrations.clear();
            for (id, file) in targets {
                let Some(parent) = file.parent() else {
                    continue;
                };
                // A parent-directory watch also notices deletion/replacement of
                // the watched directory itself, without watching whole trees.
                for (path, contents) in [(Some(parent), true), (parent.parent(), false)] {
                    let Some(path) = path else { continue };
                    let registration =
                        registrations
                            .entry(path.to_path_buf())
                            .or_insert_with(|| Registration {
                                contents: false,
                                ids: Vec::new(),
                                handle: None,
                                retry_at: Instant::now(),
                            });
                    registration.contents |= contents;
                    if !registration.ids.contains(&id) {
                        registration.ids.push(id);
                    }
                }
            }
        }
        let now = Instant::now();
        let mut changed = Vec::new();
        let mut changed_directories = Vec::new();
        for (directory, registration) in &mut registrations {
            if let Some(handle) = &registration.handle {
                match handle.changed() {
                    Ok(false) => {}
                    Ok(true) => {
                        changed.extend_from_slice(&registration.ids);
                        changed_directories.push(directory.clone());
                    }
                    Err(_) => {
                        registration.handle = None;
                        registration.retry_at = now + RETRY;
                        changed.extend_from_slice(&registration.ids);
                        changed_directories.push(directory.clone());
                    }
                }
            } else if now >= registration.retry_at {
                registration.handle =
                    native::Notification::open(directory, registration.contents).ok();
                registration.retry_at = now + RETRY;
                // Force a read after registration, closing the gap between the
                // initial snapshot and native subscription, including failures.
                changed.extend_from_slice(&registration.ids);
            }
        }
        // A notification on a parent can mean that a watched directory was
        // replaced. Its old handle may still refer to the detached directory.
        // Reopen direct children before trusting further content notifications.
        for (directory, registration) in &mut registrations {
            if directory
                .parent()
                .is_some_and(|parent| changed_directories.iter().any(|changed| changed == parent))
            {
                registration.handle = None;
                registration.retry_at = now;
            }
        }
        if !changed.is_empty() {
            shared.lock().unwrap_or_else(|e| e.into_inner()).signal(
                generation,
                changed,
                Instant::now(),
            );
        }
        if registrations.is_empty() {
            thread::park();
        } else {
            thread::park_timeout(Duration::from_millis(50));
        }
    }
}

#[cfg(not(windows))]
fn run(_: Arc<Mutex<Signals>>) {
    // The current desktop target is Windows. Other targets retain polling.
}

#[cfg(test)]
mod tests;
