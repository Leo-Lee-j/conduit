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
                return Err("文件链接包含无效的路径编码。".into());
            };
            bytes.push(high * 16 + low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| "文件链接不是有效的 UTF-8 路径。".into())
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return Err("此链接指向工作区外部，请在对应工作区中打开。".into());
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
        return Err("此链接无法在文件预览中打开。".into());
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
        return Err("文件链接缺少有效路径。".into());
    }
    if path.contains(':')
        && !(path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err("此链接无法在文件预览中打开。".into());
    }
    let root = normalize(Path::new(&display_path(workspace)))?;
    let path = PathBuf::from(display_path(Path::new(path)));
    if (path.has_root() || path.components().any(|c| matches!(c, Component::Prefix(_))))
        && !path.is_absolute()
    {
        return Err("文件链接需要完整的绝对路径或工作区相对路径。".into());
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
            return Err("此链接指向工作区外部，请在对应工作区中打开。".into());
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
            return Err("此链接指向工作区外部，请在对应工作区中打开。".into());
        }
    }
    let relative = components.as_path().to_owned();
    if relative.as_os_str().is_empty() {
        return Err("此链接指向工作区目录，请选择具体文件。".into());
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
