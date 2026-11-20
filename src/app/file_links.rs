//! Resolve transcript links without doing filesystem work on the UI thread.

use super::file_preview::SourcePosition;
use super::*;
use crate::workspace_files::FileKey;
use std::path::Component;

#[derive(Debug, PartialEq, Eq)]
enum LinkTarget {
    File(FileKey, Option<SourcePosition>),
    External,
    Anchor,
}

enum LinkAddress {
    File(String, Option<SourcePosition>),
    External,
    Anchor,
}

/// Uses navigation's syntax parser without requiring a workspace or filesystem I/O.
pub(super) fn is_file_link(href: &str) -> bool {
    matches!(parse_link(href), Ok(LinkAddress::File(..)))
}

fn decode_path(text: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut source = text.bytes();
    while let Some(byte) = source.next() {
        if byte == b'%' {
            let hex = |byte: u8| (byte as char).to_digit(16).map(|v| v as u8);
            let high = source.next().and_then(hex);
            let low = source.next().and_then(hex);
            let (Some(high), Some(low)) = (high, low) else {
                return Err(conduit::i18n::text_static(
                    "file_links.the_file_link_contains_invalid_path_encoding",
                )
                .into());
            };
            bytes.push(high * 16 + low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| conduit::i18n::text_static("file_links.the_file_link_is_not_a_valid").into())
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return Err(conduit::i18n::text_static(
                        "file_links.this_link_points_outside_the_workspace_open",
                    )
                    .into());
                }
            }
            _ => result.push(component.as_os_str()),
        }
    }
    Ok(result)
}

fn resolve_link(workspace: &Path, href: &str) -> Result<LinkTarget, String> {
    match parse_link(href)? {
        LinkAddress::File(path, position) => {
            resolve_file(workspace, &path).map(|key| LinkTarget::File(key, position))
        }
        LinkAddress::External => Ok(LinkTarget::External),
        LinkAddress::Anchor => Ok(LinkTarget::Anchor),
    }
}

fn parse_link(href: &str) -> Result<LinkAddress, String> {
    let href = href.trim();
    if href.starts_with('#') {
        return Ok(LinkAddress::Anchor);
    }
    let lower = href.to_ascii_lowercase();
    if ["https:", "http:", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s))
    {
        return Ok(LinkAddress::External);
    }
    let native_href = display_path(Path::new(href));
    let fragment_position = native_href.split_once('#').and_then(|(_, fragment)| {
        let start = fragment.strip_prefix('L')?.split('-').next()?;
        let (line, column) = start.split_once('C').unwrap_or((start, "1"));
        Some(SourcePosition {
            line: positive_number(line)?,
            column: positive_number(column)?,
        })
    });
    let mut path = native_href
        .split(['#', '?'])
        .next()
        .unwrap_or_default()
        .to_owned();
    if lower.starts_with("file:") {
        path = path[5..].to_owned();
        if let Some(rest) = path.strip_prefix("//") {
            if rest.starts_with('/') {
                path = rest.to_owned();
            } else if rest.to_ascii_lowercase().starts_with("localhost/") {
                path = rest[9..].to_owned();
            } else {
                // A file URL authority denotes a UNC share.
                path = format!("//{rest}");
            }
        }
    } else if lower.contains("://") {
        return Err(
            conduit::i18n::text_static("file_links.this_link_cannot_be_opened_in_file").into(),
        );
    }
    // Parse before decoding so escaped colons remain part of the filename.
    // A Windows drive colon is never followed by a bare positive number.
    let mut position = None;
    if let Some((file, last)) = path.rsplit_once(':')
        && let Some(last) = positive_number(last)
    {
        let (file, location) = if let Some((file, line)) = file.rsplit_once(':')
            && let Some(line) = positive_number(line)
        {
            (file, SourcePosition { line, column: last })
        } else {
            (
                file,
                SourcePosition {
                    line: last,
                    column: 1,
                },
            )
        };
        position = Some(location);
        path = file.to_owned();
    }
    #[cfg(windows)]
    if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
        path.remove(0);
    }
    let path = decode_path(&path)?;
    validate_file_path(&path)?;
    Ok(LinkAddress::File(path, fragment_position.or(position)))
}

fn positive_number(value: &str) -> Option<usize> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
        .filter(|value| *value > 0)
}

fn validate_file_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(
            conduit::i18n::text_static("file_links.the_file_link_is_missing_a_valid").into(),
        );
    }
    if path.contains(':')
        && !(path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(
            conduit::i18n::text_static("file_links.this_link_cannot_be_opened_in_file").into(),
        );
    }
    Ok(())
}

/// Attachment references are native paths, so %, # and + stay literal.
fn resolve_file(workspace: &Path, path: &str) -> Result<FileKey, String> {
    validate_file_path(path)?;
    let root = normalize(Path::new(&display_path(workspace)))?;
    let path = PathBuf::from(display_path(Path::new(path)));
    if (path.has_root() || path.components().any(|c| matches!(c, Component::Prefix(_))))
        && !path.is_absolute()
    {
        return Err(conduit::i18n::text_static(
            "file_links.file_links_require_a_full_absolute_path",
        )
        .into());
    }
    let full = normalize(&if path.is_absolute() {
        path
    } else {
        root.join(path)
    })?;
    if let Some(relative) = workspace_relative(&root, &full) {
        if relative.as_os_str().is_empty() {
            return Err(conduit::i18n::text_static(
                "file_links.this_link_points_to_a_workspace_directory",
            )
            .into());
        }
        return Ok(FileKey {
            workspace: workspace.to_owned(),
            relative,
        });
    }
    // A transcript can cite any local workspace, including one not added to
    // the app. Keep the existing bounded reader by scoping this file to its parent.
    let (Some(parent), Some(name)) = (full.parent(), full.file_name()) else {
        return Err(conduit::i18n::text_static(
            "file_links.this_link_points_to_a_workspace_directory",
        )
        .into());
    };
    Ok(FileKey {
        workspace: parent.to_owned(),
        relative: name.into(),
    })
}

fn workspace_relative(root: &Path, path: &Path) -> Option<PathBuf> {
    // Compare components so similarly named neighbouring workspaces do not match.
    let mut components = path.components();
    for expected in root.components() {
        let actual = components.next()?;
        let matches = if cfg!(windows) {
            actual
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy())
        } else {
            actual == expected
        };
        if !matches {
            return None;
        }
    }
    Some(components.as_path().to_owned())
}

impl ConduitApp {
    fn open_chat_preview(
        &mut self,
        key: FileKey,
        position: Option<SourcePosition>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = PathBuf::from(display_path(&key.workspace.join(&key.relative)));
        // Reuse the registered workspace identity for tab deduplication and
        // navigation. Unregistered paths stay standalone, read-only previews.
        let find_workspace = |path: &Path, canonical: bool| {
            self.store
                .workspaces
                .iter()
                .filter_map(|workspace| {
                    let root = if canonical {
                        workspace.path.canonicalize().ok()?
                    } else {
                        normalize(Path::new(&display_path(&workspace.path))).ok()?
                    };
                    let relative = workspace_relative(&root, path)?;
                    Some(FileKey {
                        workspace: workspace.path.clone(),
                        relative,
                    })
                })
                .max_by_key(|key| key.workspace.components().count())
        };
        let key = find_workspace(&path, false)
            .or_else(|| {
                // Windows short names can refer to a registered workspace even
                // when no path prefix matches. Only resolve filesystem aliases
                // on a miss so ordinary links need no synchronous filesystem IO.
                find_workspace(&path.canonicalize().ok()?, true)
            })
            .unwrap_or(key);
        self.open_file_preview(key, false, window, cx);
        if let Some(position) = position {
            self.reveal_file_position(position, cx);
        }
    }

    fn chat_file_error(&mut self, session: u64, error: String, cx: &mut Context<Self>) {
        let mut notice = Notice::new(
            NoticeSource::Session,
            NoticeLevel::Error,
            conduit::i18n::text_static("files.unable_to_preview_this_file"),
            error,
        );
        notice.target = self.session_notice_target(session);
        self.notify_user(notice, cx);
    }

    pub(super) fn open_chat_file(
        &mut self,
        session: u64,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .store
            .workspaces
            .iter()
            .find(|w| w.sessions.iter().any(|s| s.id == session))
        else {
            return;
        };
        match resolve_file(&workspace.path, path) {
            Ok(key) => self.open_chat_preview(key, None, window, cx),
            Err(error) => self.chat_file_error(session, error, cx),
        }
    }

    pub(super) fn open_chat_link(
        &mut self,
        session: u64,
        href: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .store
            .workspaces
            .iter()
            .find(|w| w.sessions.iter().any(|s| s.id == session))
        else {
            return;
        };
        match resolve_link(&workspace.path, href) {
            Ok(LinkTarget::File(key, position)) => {
                self.open_chat_preview(key, position, window, cx)
            }
            Ok(LinkTarget::External) => cx.open_url(href),
            Ok(LinkTarget::Anchor) => {}
            Err(error) => self.chat_file_error(session, error, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LinkTarget, SourcePosition, resolve_file, resolve_link};
    use crate::{file_ref::display_path, workspace_files::FileKey};
    use std::path::PathBuf;

    #[test]
    fn local_links_decode_and_normalize_citations() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join("conduit-links");
        for (href, column) in [
            ("./src/../docs/%E4%B8%AD%E6%96%87%20notes.md#L12", 1),
            ("docs/中文%20notes.md:12:3", 3),
            ("docs/中文%20notes.md:12", 1),
            ("docs/中文%20notes.md#L12-L18", 1),
            ("docs/中文%20notes.md#L12C3-L18C5", 3),
        ] {
            assert_eq!(
                resolve_link(&root, href),
                Ok(LinkTarget::File(
                    FileKey {
                        workspace: root.clone(),
                        relative: PathBuf::from("docs/中文 notes.md")
                    },
                    Some(SourcePosition { line: 12, column })
                ))
            );
        }
        let absolute = root.join("a+b.rs");
        assert_eq!(
            resolve_link(&root, &display_path(&absolute)),
            Ok(LinkTarget::File(
                FileKey {
                    workspace: root.clone(),
                    relative: PathBuf::from("a+b.rs")
                },
                None
            ))
        );
        let url = format!(
            "file:///{}#L2",
            display_path(&absolute)
                .replace('\\', "/")
                .trim_start_matches('/')
        );
        assert_eq!(
            resolve_link(&root, &url),
            Ok(LinkTarget::File(
                FileKey {
                    workspace: root.clone(),
                    relative: PathBuf::from("a+b.rs"),
                },
                Some(SourcePosition { line: 2, column: 1 })
            ))
        );
        assert_eq!(
            resolve_file(&root, "notes 100%#1.md"),
            Ok(FileKey {
                workspace: root.clone(),
                relative: PathBuf::from("notes 100%#1.md")
            })
        );
        #[cfg(windows)]
        for href in [
            format!("/{}:12:2", display_path(&absolute).replace('\\', "/")),
            format!(
                "file://localhost/{}",
                display_path(&absolute).replace('\\', "/")
            ),
            display_path(&absolute).to_ascii_uppercase(),
        ] {
            assert!(
                matches!(resolve_link(&root, &href), Ok(LinkTarget::File(..))),
                "{href}"
            );
        }
    }

    #[test]
    fn local_links_resolve_files_outside_the_session_workspace() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join("conduit-links");
        let other = root.with_file_name("conduit-links-other");
        let file = other.join("中文 notes.md");
        let expected = FileKey {
            workspace: other,
            relative: "中文 notes.md".into(),
        };
        for (href, position) in [
            ("../conduit-links-other/中文%20notes.md".to_owned(), None),
            (
                "./%2e%2e/conduit-links-other/中文%20notes.md#L2".to_owned(),
                Some(SourcePosition { line: 2, column: 1 }),
            ),
            (
                format!("{}:12:3", display_path(&file)),
                Some(SourcePosition {
                    line: 12,
                    column: 3,
                }),
            ),
            (
                format!(
                    "file:///{}#L12",
                    display_path(&file)
                        .replace('\\', "/")
                        .trim_start_matches('/')
                ),
                Some(SourcePosition {
                    line: 12,
                    column: 1,
                }),
            ),
        ] {
            assert_eq!(
                resolve_link(&root, &href),
                Ok(LinkTarget::File(expected.clone(), position)),
                "{href}"
            );
        }
        assert_eq!(resolve_file(&root, &display_path(&file)), Ok(expected));
        #[cfg(windows)]
        assert_eq!(
            resolve_link(std::path::Path::new(r"C:\repo"), "D:/other/file.rs:12"),
            Ok(LinkTarget::File(
                FileKey {
                    workspace: PathBuf::from(r"D:\other"),
                    relative: "file.rs".into(),
                },
                Some(SourcePosition {
                    line: 12,
                    column: 1
                })
            ))
        );
    }

    #[test]
    fn local_links_reject_invalid_paths_and_bad_encoding() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join("conduit-links");
        for href in [
            "a%FF.rs",
            "a%2.rs",
            "a%00.rs",
            "",
            ".",
            "javascript:alert(1)",
        ] {
            assert!(resolve_link(&root, href).is_err(), "{href}");
        }
        for href in ["https://example.com/a.rs#L2", "mailto:user@example.com"] {
            assert_eq!(resolve_link(&root, href), Ok(LinkTarget::External));
        }
        assert_eq!(resolve_link(&root, "#section"), Ok(LinkTarget::Anchor));
    }
}
