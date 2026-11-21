//! Bounded, read-only child history. Task summaries never serve as transcripts.
use super::same_dir;
use crate::{cli_adapter::child_history::Snapshot, domain::Provider, subagents::ChildBinding};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

mod claude;

pub(crate) fn source(
    provider: Provider,
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> Option<PathBuf> {
    let value = |name: &str| {
        env.get(name)
            .map(PathBuf::from)
            .or_else(|| std::env::var_os(name).map(PathBuf::from))
    };
    let home = || value("USERPROFILE").or_else(|| value("HOME"));
    let path = match provider {
        Provider::Codex => value("CODEX_HOME").or_else(|| home().map(|p| p.join(".codex"))),
        Provider::Claude => {
            value("CLAUDE_CONFIG_DIR").or_else(|| home().map(|p| p.join(".claude")))
        }
        Provider::OpenCode => value("OPENCODE_DB").or_else(|| {
            value("XDG_DATA_HOME")
                .or_else(|| home().map(|p| p.join(".local/share")))
                .map(|p| p.join("opencode/opencode.db"))
        }),
        Provider::Qoder => None,
    }?;
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    Some(path.canonicalize().unwrap_or(path))
}

pub(crate) fn read(
    provider: Provider,
    cwd: &Path,
    binding: &ChildBinding,
    cancel: &AtomicBool,
) -> Result<Snapshot> {
    let budget = Budget {
        started: Instant::now(),
        cancel,
    };
    budget.check(0, 0)?;
    ensure!(
        binding.native != binding.root_native && !binding.native.is_empty(),
        "Invalid child identity"
    );
    match provider {
        Provider::OpenCode => super::opencode::read_child(cwd, binding, &budget),
        Provider::Claude => claude::read(cwd, binding, &budget),
        _ => anyhow::bail!("Native child history is unavailable for this engine"),
    }
}

pub(super) struct Budget<'a> {
    started: Instant,
    cancel: &'a AtomicBool,
}
impl Budget<'_> {
    pub(super) fn check(&self, bytes: u64, items: usize) -> Result<()> {
        ensure!(
            !self.cancel.load(Ordering::Acquire),
            "Child history read cancelled"
        );
        ensure!(
            self.started.elapsed() < Duration::from_secs(45),
            "Child history read timed out"
        );
        ensure!(
            bytes <= 64 * 1024 * 1024 && items <= 100_000,
            "Child history exceeds the read budget"
        );
        Ok(())
    }
}

pub(crate) fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Do not follow symlinks outside the engine's data namespace.
pub(super) fn contained(path: &Path, root: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .context("Child history file is unavailable")?;
    ensure!(
        path.starts_with(root.canonicalize()?),
        "Child history escaped its data source"
    );
    Ok(path)
}

#[cfg(test)]
mod tests;
