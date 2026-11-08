//! Resolve transcript links without doing filesystem work on the UI thread.

use super::*;
use crate::workspace_files::FileKey;
use std::path::Component;

#[derive(Debug, PartialEq, Eq)]
enum LinkTarget {
    File(FileKey),
    External,
    Anchor,
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
    let href = href.trim();
    if href.starts_with('#') {
        return Ok(LinkTarget::Anchor);
    }
    let lower = href.to_ascii_lowercase();
    if ["https:", "http:", "mailto:"]
        .iter()
        .any(|s| lower.starts_with(s))
    {
        return Ok(LinkTarget::External);
    }
    let native_href = display_path(Path::new(href));
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
    // CLI citations commonly append :line or :line:column. Leave the drive
    // letter intact, and decode afterwards so %3A in a filename stays literal.
    for _ in 0..2 {
        if let Some((file, line)) = path.rsplit_once(':')
            && !line.is_empty()
            && line.bytes().all(|b| b.is_ascii_digit())
        {
            path = file.to_owned();
        }
    }
    #[cfg(windows)]
    if path.starts_with('/') && path.as_bytes().get(2) == Some(&b':') {
        path.remove(0);
    }
    resolve_file(workspace, &decode_path(&path)?).map(LinkTarget::File)
}

/// Attachment references are native paths, so %, # and + stay literal.
fn resolve_file(workspace: &Path, path: &str) -> Result<FileKey, String> {
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
    // Windows paths are case insensitive. Compare components rather than a
    // string prefix so a neighbouring directory cannot match the workspace.
    let mut components = full.components();
    for expected in root.components() {
        let Some(actual) = components.next() else {
            return Err(conduit::i18n::text_static(
                "file_links.this_link_points_outside_the_workspace_open",
            )
            .into());
        };
        let matches = if cfg!(windows) {
            actual
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy())
        } else {
            actual == expected
        };
        if !matches {
            return Err(conduit::i18n::text_static(
                "file_links.this_link_points_outside_the_workspace_open",
            )
            .into());
        }
    }
    let relative = components.as_path().to_owned();
    if relative.as_os_str().is_empty() {
        return Err(conduit::i18n::text_static(
            "file_links.this_link_points_to_a_workspace_directory",
        )
        .into());
    }
    Ok(FileKey {
        workspace: workspace.to_owned(),
        relative,
    })
}

impl ConduitApp {
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
            Ok(key) => self.open_workspace_file(key, false, window, cx),
            Err(error) => {
                self.message = Some(error);
                cx.notify();
            }
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
            Ok(LinkTarget::File(key)) => self.open_workspace_file(key, false, window, cx),
            Ok(LinkTarget::External) => cx.open_url(href),
            Ok(LinkTarget::Anchor) => {}
            Err(error) => {
                self.message = Some(error);
                cx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LinkTarget, resolve_file, resolve_link};
    use crate::{file_ref::display_path, workspace_files::FileKey};
    use std::path::PathBuf;

    #[test]
    fn local_links_decode_and_normalize_citations() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join("conduit-links");
        for href in [
            "./src/../docs/%E4%B8%AD%E6%96%87%20notes.md#L12",
            "docs/中文%20notes.md:12:3",
        ] {
            assert_eq!(
                resolve_link(&root, href),
                Ok(LinkTarget::File(FileKey {
                    workspace: root.clone(),
                    relative: PathBuf::from("docs/中文 notes.md")
                }))
            );
        }
        let absolute = root.join("a+b.rs");
        assert_eq!(
            resolve_link(&root, &display_path(&absolute)),
            Ok(LinkTarget::File(FileKey {
                workspace: root.clone(),
                relative: PathBuf::from("a+b.rs")
            }))
        );
        let url = format!(
            "file:///{}#L2",
            display_path(&absolute)
                .replace('\\', "/")
                .trim_start_matches('/')
        );
        assert_eq!(
            resolve_link(&root, &url),
            resolve_link(&root, &display_path(&absolute))
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
                matches!(resolve_link(&root, &href), Ok(LinkTarget::File(_))),
                "{href}"
            );
        }
    }

    #[test]
    fn local_links_reject_escape_and_bad_encoding() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join("conduit-links");
        for href in [
            "../outside.rs",
            "./%2e%2e/outside.rs",
            "a%FF.rs",
            "a%2.rs",
            "a%00.rs",
            "",
            ".",
            "javascript:alert(1)",
        ] {
            assert!(resolve_link(&root, href).is_err(), "{href}");
        }
        let neighbour = root.with_file_name("conduit-links-other").join("file.rs");
        assert!(resolve_link(&root, &display_path(&neighbour)).is_err());
        for href in ["https://example.com/a.rs#L2", "mailto:user@example.com"] {
            assert_eq!(resolve_link(&root, href), Ok(LinkTarget::External));
        }
        assert_eq!(resolve_link(&root, "#section"), Ok(LinkTarget::Anchor));
    }
}
