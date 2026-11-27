//! Diagnose lock contention without guessing whether another writer is alive.
use super::*;

fn guidance(path: &Path) -> String {
    conduit::i18n::format(
        "git.lock_file_exists",
        &[("path", path.display().to_string())],
    )
}

#[derive(Debug)]
struct LockConflict(PathBuf);

impl std::fmt::Display for LockConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&guidance(&self.0))
    }
}

pub(super) fn is_contention(error: &anyhow::Error) -> bool {
    error.downcast_ref::<LockConflict>().is_some()
}

const LOCK_WAIT: Duration = Duration::from_millis(300);

pub(super) struct IndexLock {
    pub path: PathBuf,
    pub owned: bool,
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        if self.owned {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn acquire(path: &Path, cancel: &AtomicBool) -> Result<IndexLock> {
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) {
            bail!(conduit::i18n::text("git.git_read_cancelled"));
        }
        match acquire_index_lock(path) {
            Ok(file) => {
                drop(file);
                return Ok(IndexLock {
                    path: path.into(),
                    owned: true,
                });
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::AlreadyExists)
                    && started.elapsed() < LOCK_WAIT =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
}

fn acquire_index_lock(path: &Path) -> Result<File> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            let exists = error.kind() == std::io::ErrorKind::AlreadyExists;
            let error = anyhow::Error::new(error);
            if exists {
                error.context(LockConflict(path.into()))
            } else {
                error.context(conduit::i18n::format(
                    "git.lock_file_create_failed",
                    &[("path", path.display().to_string())],
                ))
            }
        })
}

/// Compare the complete index, including unrelated entries and extensions.
/// Called with the real index locked before publishing an optimistic result.
pub(super) fn same_index(first: &Path, second: &Path) -> Result<bool> {
    let mut first = File::open(first)?;
    let mut second = File::open(second)?;
    if first.metadata()?.len() != second.metadata()?.len() {
        return Ok(false);
    }
    let mut left = [0; 8192];
    let mut right = [0; 8192];
    let mut remaining = first.metadata()?.len();
    while remaining > 0 {
        let size = remaining.min(left.len() as u64) as usize;
        first.read_exact(&mut left[..size])?;
        second.read_exact(&mut right[..size])?;
        if left[..size] != right[..size] {
            return Ok(false);
        }
        remaining -= size as u64;
    }
    Ok(true)
}

pub(super) fn explain_command_error(
    error: anyhow::Error,
    stderr: &str,
    workspace: &Path,
) -> anyhow::Error {
    match conflicting_lock(stderr) {
        Some(path) => error.context(LockConflict(workspace.join(path))),
        None => error,
    }
}

fn conflicting_lock(stderr: &str) -> Option<&Path> {
    stderr.lines().find_map(|line| {
        // Match Git's local lock-file diagnostic, including ref/config locks.
        // Do not treat a remote rejection, permissions error or any mention of
        // a tracked *.lock file as permission to suggest removing that file.
        if !line.starts_with("fatal: ") && !line.starts_with("error: ") {
            return None;
        }
        let (_, rest) = line.split_once("Unable to create '")?;
        let (path, _) = rest.rsplit_once("': File exists")?;
        path.ends_with(".lock").then_some(Path::new(path))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_diagnostics_recognize_local_index_and_ref_contention_only() {
        for path in [
            "D:/repo/.git/index.lock",
            "D:/中文 空格/it's here/.git/refs/heads/main.lock",
            ".git/packed-refs.lock",
            ".git/config.lock",
        ] {
            for prefix in ["fatal: ", "error: cannot lock ref 'refs/heads/main': "] {
                let stderr = format!("{prefix}Unable to create '{path}': File exists\n");
                assert_eq!(conflicting_lock(&stderr), Some(Path::new(path)));
            }
        }
        for stderr in [
            "fatal: Unable to create '.git/index.lock': Permission denied",
            "fatal: Unable to create '.git/index.lock': No such file or directory",
            "fatal: Unable to create 'tracked.txt': File exists",
            "remote: error: Unable to create '.git/index.lock': File exists",
            "error: pathspec 'tracked.lock' did not match any file(s) known to git",
            "error: cannot lock ref 'refs/heads/main': is at abc but expected def",
        ] {
            assert_eq!(conflicting_lock(stderr), None, "{stderr}");
        }
    }

    #[test]
    fn lock_diagnostics_keep_raw_errors_and_resolve_relative_paths() {
        let _locale = crate::test_support::chinese();
        let workspace = std::env::temp_dir().join("repository");
        let raw = "fatal: Unable to create '.git/index.lock': File exists";
        let error = explain_command_error(anyhow!(raw), raw, &workspace);
        let message = format!("{error:#}");
        assert!(message.starts_with("Git 锁文件已存在"), "{message}");
        assert!(message.contains(&workspace.join(".git/index.lock").display().to_string()));
        assert!(message.contains(raw));
        let raw = "fatal: Unable to create '.git/index.lock': Permission denied";
        assert_eq!(
            explain_command_error(anyhow!(raw), raw, &workspace).to_string(),
            raw
        );
    }
}
