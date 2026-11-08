//! Bounded, read-only workspace browsing. No filesystem work belongs on the UI thread.
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

/// Human-readable native Unicode paths. Strip Windows' implementation-only
/// verbatim prefix for display/copy, without changing any filesystem operation.
pub fn display_path(path: &Path) -> String {
    let label = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = label.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(disk) = label.strip_prefix(r"\\?\") {
            return disk.to_owned();
        }
    }
    label.into_owned()
}

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_FILE_LINES: usize = 10_000;
const MAX_DIRECTORY_ENTRIES: usize = 20_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct FileKey {
    pub workspace: PathBuf,
    pub relative: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub relative: PathBuf,
    pub name: String,
    pub directory: bool,
    pub link: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}

pub fn stamp(path: &Path) -> Result<Stamp, String> {
    let metadata = fs::metadata(path).map_err(|e| {
        conduit::i18n::format(
            "workspace_files.unable_to_read_file_status",
            &[("e", format!("{}", e))],
        )
    })?;
    Ok(Stamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

/// Canonical containment also rejects junctions/symlinks pointing outside the workspace.
fn checked_path(key: &FileKey) -> Result<PathBuf, String> {
    if key
        .relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(conduit::i18n::text_static(
            "workspace_files.path_must_be_inside_the_current_workspace",
        )
        .into());
    }
    let root = key.workspace.canonicalize().map_err(|e| {
        conduit::i18n::format(
            "workspace_files.workspace_inaccessible",
            &[("e", format!("{}", e))],
        )
    })?;
    let path = root.join(&key.relative).canonicalize().map_err(|e| {
        conduit::i18n::format(
            "workspace_files.path_deleted_or_inaccessible",
            &[("e", format!("{}", e))],
        )
    })?;
    if !path.starts_with(&root) {
        return Err(conduit::i18n::text_static(
            "file_links.this_link_points_outside_the_workspace_open",
        )
        .into());
    }
    Ok(path)
}

pub fn directory(key: &FileKey, show_ignored: bool) -> Result<(Vec<Entry>, Stamp), String> {
    let path = checked_path(key)?;
    if !path.is_dir() {
        return Err(
            conduit::i18n::text_static("workspace_files.this_directory_no_longer_exists").into(),
        );
    }
    // Validate permissions before the walker, which can otherwise yield an empty tree.
    fs::read_dir(&path).map_err(|e| {
        conduit::i18n::format(
            "workspace_files.unable_to_read_directory",
            &[("e", format!("{}", e))],
        )
    })?;
    let initial = stamp(&path)?;
    // Start at the workspace root so parent .gitignore rules remain effective.
    let walker = ignore::WalkBuilder::new(&key.workspace)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .max_depth(Some(key.relative.components().count() + 1))
        .ignore(!show_ignored)
        .git_ignore(!show_ignored)
        .git_global(!show_ignored)
        .git_exclude(!show_ignored)
        .filter_entry({
            let wanted = key.relative.clone();
            let root = key.workspace.clone();
            move |entry| {
                let Ok(relative) = entry.path().strip_prefix(&root) else {
                    return false;
                };
                if relative.as_os_str().is_empty() {
                    return true;
                }
                if entry.file_name() == ".git" {
                    return false;
                }
                let relevant =
                    wanted.starts_with(relative) || relative.parent() == Some(wanted.as_path());
                relevant
                    && (show_ignored
                        || !entry.file_type().is_some_and(|t| t.is_dir())
                        || !matches!(
                            entry.file_name().to_str(),
                            Some("node_modules" | "target" | ".venv")
                        ))
            }
        })
        .build();
    let mut entries = Vec::new();
    for entry in walker {
        let entry = entry.map_err(|e| {
            conduit::i18n::format(
                "workspace_files.some_directories_are_inaccessible",
                &[("e", format!("{}", e))],
            )
        })?;
        if entry.depth() == 0 {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(&key.workspace) else {
            continue;
        };
        if relative.parent() != Some(key.relative.as_path()) {
            continue;
        }
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            return Err(conduit::i18n::text_static(
                "workspace_files.this_directory_contains_more_than_20_000",
            )
            .into());
        }
        let kind = entry.file_type();
        entries.push(Entry {
            relative: relative.to_owned(),
            name: entry.file_name().to_string_lossy().into_owned(),
            directory: kind.is_some_and(|t| t.is_dir()),
            link: kind.is_some_and(|t| t.is_symlink()),
        });
    }
    entries.sort_by_cached_key(|e| (!e.directory, e.name.to_lowercase(), e.name.clone()));
    Ok((entries, initial))
}

pub struct TextFile {
    pub text: String,
    pub stamp: Stamp,
    pub encoding: &'static str,
}

pub fn read(key: &FileKey) -> Result<TextFile, String> {
    let path = checked_path(key)?;
    if !path.is_file() {
        return Err(
            conduit::i18n::text_static("workspace_files.this_path_is_not_a_regular_file").into(),
        );
    }
    let initial = stamp(&path)?;
    if initial.len > MAX_FILE_BYTES {
        return Err(conduit::i18n::text_static(
            "workspace_files.file_exceeds_the_1_mib_preview_limit",
        )
        .into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .map_err(|e| {
            conduit::i18n::format(
                "workspace_files.unable_to_open_file",
                &[("e", format!("{}", e))],
            )
        })?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            conduit::i18n::format(
                "workspace_files.failed_to_read_file",
                &[("e", format!("{}", e))],
            )
        })?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(conduit::i18n::text_static(
            "workspace_files.file_exceeds_the_1_mib_preview_limit",
        )
        .into());
    }
    let (text, encoding) = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err(conduit::i18n::text_static(
                "workspace_files.incomplete_utf_16_file_contents",
            )
            .into());
        }
        let little = bytes[0] == 0xff;
        let units: Vec<_> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if little {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            })
            .collect();
        (
            String::from_utf16(&units).map_err(|_| {
                conduit::i18n::text_static("workspace_files.invalid_utf_16_encoding").to_string()
            })?,
            if little { "UTF-16 LE" } else { "UTF-16 BE" },
        )
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        (
            String::from_utf8(bytes.to_vec()).map_err(|_| {
                conduit::i18n::text_static("workspace_files.unknown_encoding_or_binary_file_utf_8")
                    .to_string()
            })?,
            "UTF-8",
        )
    };
    if text
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(conduit::i18n::text_static(
            "workspace_files.this_is_a_binary_file_text_preview",
        )
        .into());
    }
    if text.lines().count() > MAX_FILE_LINES || text.lines().any(|line| line.len() > 20_000) {
        return Err(conduit::i18n::text_static(
            "workspace_files.file_line_count_or_line_length_exceeds",
        )
        .into());
    }
    // Display normalized newlines while preserving the original file on disk.
    Ok(TextFile {
        text: text.replace("\r\n", "\n"),
        stamp: initial,
        encoding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "conduit-browser-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn display_path_preserves_unicode_and_punctuation() {
        let _locale = crate::test_support::chinese();
        let path = Path::new("工作区").join("文档").join("示例 [预览] #1.rs");
        assert_eq!(display_path(&path), path.to_str().unwrap());
        #[cfg(windows)]
        {
            assert_eq!(
                display_path(Path::new(r"\\?\G:\工作区\文档\示例 [预览] #1.rs")),
                r"G:\工作区\文档\示例 [预览] #1.rs"
            );
            assert_eq!(
                display_path(Path::new(r"\\?\UNC\服务器\共享\文档\示例.rs")),
                r"\\服务器\共享\文档\示例.rs"
            );
        }
    }

    #[test]
    fn lazy_tree_sort_and_parent_ignore_rules() {
        let _locale = crate::test_support::chinese();
        let root = root();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join(".gitignore"), "src/secret.txt\n").unwrap();
        fs::write(root.join("src/secret.txt"), "secret").unwrap();
        fs::write(root.join("src/中 文.rs"), "fn main() {}\n").unwrap();
        let key = FileKey {
            workspace: root.clone(),
            relative: "src".into(),
        };
        let (entries, _) = directory(&key, false).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].directory);
        assert_eq!(entries[1].name, "中 文.rs");
        assert_eq!(directory(&key, true).unwrap().0.len(), 3);
        assert!(
            !directory(
                &FileKey {
                    workspace: root.clone(),
                    relative: PathBuf::new()
                },
                false
            )
            .unwrap()
            .0
            .iter()
            .any(|e| e.name == "target")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bounded_reader_encoding_and_path_validation() {
        let _locale = crate::test_support::chinese();
        let root = root();
        let key = FileKey {
            workspace: root.clone(),
            relative: "test.txt".into(),
        };
        fs::write(root.join("test.txt"), "中文\r\n").unwrap();
        assert_eq!(read(&key).unwrap().text, "中文\n");
        fs::write(root.join("test.txt"), [0xff, 0xfe, 0x2d, 0x4e]).unwrap();
        assert_eq!(read(&key).unwrap().text, "中");
        fs::write(root.join("test.txt"), [0, 1, 2]).unwrap();
        assert!(read(&key).is_err());
        fs::write(
            root.join("test.txt"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read(&key).err().unwrap().contains("1 MiB"));
        assert!(
            read(&FileKey {
                workspace: root.clone(),
                relative: "../escape".into()
            })
            .is_err()
        );
        assert!(
            read(&FileKey {
                workspace: root.clone(),
                relative: root.join("test.txt")
            })
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
