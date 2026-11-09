//! Convert between repository text and working files using Git's attributes.
use super::*;

// An encoded working file can be larger than its UTF-8 repository blob.
const WORKTREE_LIMIT: usize = 4 * TEXT_LIMIT;

pub(super) fn read_text_object(
    git: &Path,
    root: &Path,
    oid: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let output = run_git_bounded(
        git,
        root,
        &["cat-file", "blob", oid],
        cancel,
        TEXT_LIMIT,
        COMMAND_TIMEOUT,
        true,
    )?;
    if output.truncated {
        bail!(conduit::i18n::text("git_conflicts.too_large"));
    }
    if output.bytes.contains(&0) {
        bail!(conduit::i18n::text("git_conflicts.binary"));
    }
    String::from_utf8(output.bytes)
        .map_err(|_| anyhow!(conduit::i18n::text("git_conflicts.binary")))
}

pub(super) fn read_worktree(
    git: &Path,
    expected: &ConflictSnapshot,
    cancel: &AtomicBool,
) -> Result<String> {
    let full = checked_path(&expected.root, &expected.path)?;
    if fs::metadata(&full)?.len() > WORKTREE_LIMIT as u64 {
        bail!(conduit::i18n::text("git_conflicts.too_large"));
    }
    // Use the same clean/encoding conversion as `git add`, without touching
    // the index or the working file. The resulting object is also recoverable.
    let oid = run_git(
        git,
        &expected.root,
        &[
            "hash-object",
            "-w",
            &format!("--path={}", expected.path),
            "--",
            &full.to_string_lossy(),
        ],
        cancel,
    )?;
    read_text_object(git, &expected.root, oid.trim(), cancel)
}

pub(super) fn marker_size(
    git: &Path,
    expected: &ConflictSnapshot,
    cancel: &AtomicBool,
) -> Result<usize> {
    let attribute = run_git(
        git,
        &expected.root,
        &[
            "--literal-pathspecs",
            "check-attr",
            "-z",
            "conflict-marker-size",
            "--",
            &expected.path,
        ],
        cancel,
    )?;
    Ok(attribute
        .split('\0')
        .nth(2)
        .and_then(|value| value.parse().ok())
        .filter(|size| *size > 0)
        .unwrap_or(7))
}

/// Only discard the working copy in favor of a fresh block plan when it is
/// exactly the automatic merge. Labels are branch names in `git merge` and
/// commit descriptions in `git rebase`, so compare their marker tokens only.
pub(super) fn is_original_merge(
    git: &Path,
    expected: &ConflictSnapshot,
    versions: [&str; 3],
    current: &str,
    cancel: &AtomicBool,
) -> Result<bool> {
    let scratch = Scratch::new_in(&std::env::temp_dir(), "merge-original")?;
    let files = ["base", "ours", "theirs"].map(|name| scratch.0.join(name));
    for (path, text) in files.iter().zip(versions) {
        fs::write(path, text)?;
    }
    let size = marker_size(git, expected, cancel)?;
    if size > TEXT_LIMIT {
        bail!(conduit::i18n::text("git_conflicts.too_large"));
    }
    let labels = ["ours", "base", "theirs"].map(|name| format!("{}-{name}", scratch.0.display()));
    let merge = run_git_bounded(
        git,
        &expected.root,
        &[
            "merge-file",
            "-p",
            "--marker-size",
            &size.to_string(),
            "-L",
            &labels[0],
            "-L",
            &labels[1],
            "-L",
            &labels[2],
            "--",
            &files[1].to_string_lossy(),
            &files[0].to_string_lossy(),
            &files[2].to_string_lossy(),
        ],
        cancel,
        WORKTREE_LIMIT,
        COMMAND_TIMEOUT,
        true,
    )?;
    if merge.truncated {
        return Ok(false);
    }
    let markers = ['<', '|', '>'].map(|symbol| symbol.to_string().repeat(size));
    let labelled: Vec<_> = markers
        .iter()
        .zip(labels)
        .map(|(token, label)| format!("{token} {label}"))
        .collect();
    let mut actual = current.split_inclusive('\n');
    for line in merge.text.split_inclusive('\n') {
        let Some(other) = actual.next() else {
            return Ok(false);
        };
        if let Some(index) = labelled
            .iter()
            .position(|label| line.trim_end_matches(['\r', '\n']) == label)
        {
            if !other.starts_with(&format!("{} ", markers[index]))
                || line.ends_with('\n') != other.ends_with('\n')
            {
                return Ok(false);
            }
        } else if line != other {
            return Ok(false);
        }
    }
    Ok(actual.next().is_none())
}

pub(super) fn to_worktree(
    git: &Path,
    expected: &ConflictSnapshot,
    scratch: &Path,
    text: &str,
    cancel: &AtomicBool,
) -> Result<Vec<u8>> {
    let source = scratch.join("merged");
    fs::write(&source, text)?;
    // The editor holds index text: applying clean/encoding conversion here
    // would interpret UTF-8 as the working-tree encoding a second time.
    let oid = run_git(
        git,
        &expected.root,
        &[
            "hash-object",
            "-w",
            "--no-filters",
            "--",
            &source.to_string_lossy(),
        ],
        cancel,
    )?;
    let output = run_git_bounded(
        git,
        &expected.root,
        &[
            "cat-file",
            "--filters",
            &format!("--path={}", expected.path),
            oid.trim(),
        ],
        cancel,
        WORKTREE_LIMIT,
        COMMAND_TIMEOUT,
        true,
    )?;
    if output.truncated {
        bail!(conduit::i18n::text("git_conflicts.too_large"));
    }
    // Some Git versions report a failed checkout encoding on stderr but still
    // return success with the original UTF-8 bytes. Check the reverse conversion
    // before replacing the working file, rather than trusting that exit status.
    let converted = scratch.join("converted");
    fs::write(&converted, &output.bytes)?;
    let cleaned = run_git(
        git,
        &expected.root,
        &[
            "hash-object",
            "-w",
            &format!("--path={}", expected.path),
            "--",
            &converted.to_string_lossy(),
        ],
        cancel,
    )?;
    if cleaned.trim() != oid.trim()
        && read_text_object(git, &expected.root, cleaned.trim(), cancel)?
            != text.replace("\r\n", "\n")
    {
        bail!(conduit::i18n::text("git_conflicts.conversion_failed"));
    }
    Ok(output.bytes)
}
