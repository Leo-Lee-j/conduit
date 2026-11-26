use super::*;
use std::collections::HashSet;

/// Check recorded paths, including deleted files, against Git's current ignore
/// rules. Tracked paths stay visible even when they match an ignore pattern.
/// Call on a worker thread, like the other Git queries.
pub fn ignored_paths(workspace: &Path, paths: &[&str]) -> Result<HashSet<String>> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let git = resolve_git()
        .ok_or_else(|| anyhow!(conduit::i18n::text("git.git_exe_not_found_install_git_or")))?;
    let mut ignored = HashSet::new();
    // Bound input/output per process without a command-line length limit.
    for paths in paths.chunks(256) {
        let input: Vec<_> = paths
            .iter()
            .flat_map(|path| path.bytes().chain(std::iter::once(0)))
            .collect();
        let output = run_git_with_environment(
            &git,
            workspace,
            &["check-ignore", "--stdin", "-z"],
            &AtomicBool::new(false),
            None,
            CommandLimits {
                limit: input.len(),
                timeout: COMMAND_TIMEOUT,
                stop_when_full: false,
                background: true,
                input: Some(input),
            },
        )?;
        if output.truncated {
            bail!("Git ignore output exceeded the input path list");
        }
        for path in output
            .bytes
            .split(|byte| *byte == 0)
            .filter(|p| !p.is_empty())
        {
            ignored.insert(String::from_utf8(path.to_vec())?);
        }
    }
    Ok(ignored)
}
