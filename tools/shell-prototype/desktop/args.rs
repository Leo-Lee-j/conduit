//! Explicit, session-only development loading before creating the desktop window.
use anyhow::{Result, bail, ensure};
use conduit::plugins::commands::Capability;
use conduit_shell_prototype::{adapter::ApprovedPackage, package::PackageSnapshot};
use std::{collections::BTreeSet, ffi::OsString, path::PathBuf};

pub fn parse(args: impl Iterator<Item = OsString>) -> Result<Option<Vec<ApprovedPackage>>> {
    let mut args = args.peekable();
    let mut source = None;
    let mut grants = BTreeSet::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--dev-plugin") => {
                ensure!(source.is_none(), "only one development package is accepted");
                source = Some(PathBuf::from(args.next().ok_or_else(|| {
                    anyhow::anyhow!("--dev-plugin requires a directory")
                })?));
            }
            Some("--allow-permission") => {
                let permission = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--allow-permission requires a permission"))?;
                let grant = match permission.to_str() {
                    Some("workspace.metadata.read") => Capability::WorkspaceSummary,
                    Some("session.summary.read") => Capability::SessionSummary,
                    _ => bail!("unsupported development permission"),
                };
                ensure!(grants.insert(grant), "duplicate development permission");
            }
            Some("--settings" | "--git") => {}
            _ => bail!(
                "unknown desktop development argument: {}",
                arg.to_string_lossy()
            ),
        }
    }
    let Some(source) = source else {
        ensure!(grants.is_empty(), "permissions require --dev-plugin");
        return Ok(None);
    };
    let snapshot =
        PackageSnapshot::prepare(&source, &std::env::temp_dir().join("conduit-shell-desktop"))?;
    Ok(Some(vec![ApprovedPackage::new(snapshot, grants)?]))
}
