use super::{File, MAX_TREE_FILES, Transport, fail, text};
use anyhow::{Result, ensure};
use std::{
    io::{Cursor, Read},
    sync::Arc,
};
use zip::ZipArchive;

const MAX_ARCHIVE: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Archive {
    zip: ZipArchive<Cursor<Arc<[u8]>>>,
    root: String,
}
impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("entries", &self.zip.len())
            .finish()
    }
}

pub(super) fn load(
    client: &impl Transport,
    repo: &str,
    commit: &str,
) -> Result<(Vec<File>, Archive)> {
    let url = format!("https://codeload.github.com/{repo}/zip/{commit}");
    let bytes = client.get(&url, MAX_ARCHIVE)?;
    let zip = ZipArchive::new(Cursor::new(Arc::<[u8]>::from(bytes)))
        .map_err(|_| fail("invalid_response"))?;
    ensure!(
        !zip.is_empty() && zip.len() <= MAX_TREE_FILES,
        text("response_limit")
    );
    let mut files = Vec::new();
    let mut root = String::new();
    for index in 0..zip.len() {
        client.check()?;
        let entry = zip
            .by_index_data(index)
            .map_err(|_| fail("invalid_response"))?;
        let name = entry.name().map_err(|_| fail("invalid_response"))?;
        let Some((prefix, path)) = name.split_once('/') else {
            return Err(fail("invalid_response"));
        };
        if root.is_empty() {
            ensure!(
                prefix
                    .strip_suffix(commit)
                    .is_some_and(|name| name.ends_with('-')),
                text("invalid_response")
            );
            root = prefix.to_owned();
        }
        ensure!(prefix == root, text("invalid_response"));
        if path.is_empty() && entry.is_dir() {
            continue;
        }
        // Don't extract wholesale. The installer validates and writes only the selected skill.
        let mode = match entry.unix_mode().map(|mode| mode & 0o170000) {
            Some(0o120000) => "120000",
            Some(0o040000) if entry.is_dir() => "040000",
            Some(0o100000) if !entry.is_dir() => "100644",
            None | Some(0) if entry.is_dir() => "040000",
            None | Some(0) => "100644",
            _ => "unsupported",
        };
        files.push(File {
            path: path.trim_end_matches('/').to_owned(),
            mode: mode.into(),
            size: usize::try_from(entry.size()).unwrap_or(usize::MAX),
        });
    }
    Ok((files, Archive { zip, root }))
}

impl Archive {
    pub(super) fn read(
        &mut self,
        client: &impl Transport,
        path: &str,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let mut entry = self
            .zip
            .by_name(&format!("{}/{path}", self.root))
            .map_err(|_| fail("invalid_response"))?;
        ensure!(!entry.is_dir() && !entry.is_symlink(), text("linked_file"));
        ensure!(entry.size() <= limit as u64, text("response_limit"));
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            client.check()?;
            let count = entry
                .read(&mut chunk)
                .map_err(|_| fail("invalid_response"))?;
            if count == 0 {
                return Ok(bytes);
            }
            ensure!(bytes.len() + count <= limit, text("response_limit"));
            bytes.extend_from_slice(&chunk[..count]);
        }
    }
}
