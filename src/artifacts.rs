//! User-selected project outputs and service links; no servers are started here.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub id: u64,
    pub workspace: PathBuf,
    pub title: String,
    pub target: String,
}

pub fn url(workspace: &Path, target: &str) -> Result<String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        let parsed = url::Url::parse(target)?;
        ensure!(
            parsed.host_str().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none(),
            conduit::i18n::text("artifact.invalid")
        );
        return Ok(parsed.to_string());
    }
    let path = crate::checkpoints::checked_path(workspace, Path::new(target))?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        path.is_file()
            && matches!(
                extension.as_str(),
                "html"
                    | "htm"
                    | "pdf"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "gif"
                    | "svg"
                    | "txt"
                    | "md"
                    | "json"
                    | "csv"
            ),
        conduit::i18n::text("artifact.invalid")
    );
    url::Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| anyhow::anyhow!(conduit::i18n::text("artifact.invalid")))
}

pub fn context(workspace: &Path, relative: &Path, range: &str) -> Result<String> {
    use std::io::Read;
    let path = crate::checkpoints::checked_path(workspace, relative)?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1024 * 1024 && !bytes.contains(&0),
        conduit::i18n::text("checkpoint.text_only")
    );
    let text = String::from_utf8(bytes)?;
    let lines: Vec<_> = text.lines().collect();
    let (start, end) = if range.trim().is_empty() {
        (1, lines.len())
    } else {
        let (a, b) = range
            .trim()
            .split_once('-')
            .unwrap_or((range.trim(), range.trim()));
        (a.parse::<usize>()?, b.parse::<usize>()?)
    };
    ensure!(
        start > 0 && end >= start && end <= lines.len(),
        conduit::i18n::text("artifact.range")
    );
    let selected = lines[start - 1..end].join("\n");
    ensure!(
        selected.len() <= 64 * 1024,
        conduit::i18n::text("workflow.context_limit")
    );
    Ok(conduit::i18n::format(
        "artifact.context",
        &[
            ("workspace", workspace.display().to_string()),
            ("path", relative.display().to_string()),
            ("start", start.to_string()),
            ("end", end.to_string()),
            ("hash", crate::review::fingerprint(&text)),
            ("text", selected),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_reject_credentials_schemes_and_escape_paths() {
        assert!(url(Path::new("."), "https://user:secret@example.com").is_err());
        assert!(url(Path::new("."), "javascript:alert(1)").is_err());
        assert!(url(Path::new("."), "../outside.html").is_err());
        assert_eq!(
            url(Path::new("."), "http://localhost:5173/report").unwrap(),
            "http://localhost:5173/report"
        );
    }
}
