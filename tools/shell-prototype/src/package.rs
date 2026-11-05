//! Session-only snapshots of explicitly selected development packages.
use crate::manifest::{MAX_MANIFEST_BYTES, Manifest, relative_path};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, Metadata, OpenOptions},
    io::Read,
    path::Path,
};

pub const MAX_ENTRIES: usize = 256;
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DIRECTORIES_DEEP: usize = 16;

pub struct PackageSnapshot {
    directory: tempfile::TempDir,
    digest: String,
    manifest: Manifest,
}

impl PackageSnapshot {
    /// Copy and validate before constructing a window/runtime. Never executes JS,
    /// writes the selected source, or creates a durable installation/trust record.
    pub fn prepare(source: &Path, staging_parent: &Path) -> Result<Self> {
        let files = read_package(source)?;
        let expected = digest(&files);
        fs::create_dir_all(staging_parent)?;
        let directory = tempfile::Builder::new()
            .prefix("conduit-dev-")
            .tempdir_in(staging_parent)?;
        for (name, bytes) in &files {
            let path = directory.path().join(name);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, bytes)?;
        }
        // Parse and hash the copy that will execute, including both manifests.
        let copied = read_package(directory.path())?;
        ensure!(
            digest(&copied) == expected,
            "development snapshot copy changed"
        );
        let manifest = parse(&copied)?;
        Ok(Self {
            directory,
            digest: expected,
            manifest,
        })
    }

    pub fn inspect(source: &Path) -> Result<(Manifest, String)> {
        let files = read_package(source)?;
        Ok((parse(&files)?, digest(&files)))
    }
    pub fn root(&self) -> &Path {
        self.directory.path()
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn verify(&self) -> Result<()> {
        ensure!(
            digest(&read_package(self.root())?) == self.digest,
            "development snapshot changed; restart the development session"
        );
        Ok(())
    }
}

fn parse(files: &BTreeMap<String, Vec<u8>>) -> Result<Manifest> {
    let shell = files
        .get("gpui-shell.json")
        .context("missing gpui-shell.json")?;
    let conduit = files
        .get("conduit-plugin.json")
        .context("missing conduit-plugin.json")?;
    let manifest = Manifest::parse(shell, conduit)?;
    for entry in manifest.entries() {
        ensure!(files.contains_key(entry), "missing view entry: {entry}");
    }
    Ok(manifest)
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn read_package(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let metadata = fs::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir() && !linked(&metadata),
        "package root must be a real directory, not a link/reparse point"
    );
    let root = root.canonicalize()?;
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut pending = vec![(root.clone(), 0)];
    let mut total = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        ensure!(
            depth <= MAX_DIRECTORIES_DEEP,
            "package directory depth exceeded"
        );
        ensure!(
            !linked(&fs::symlink_metadata(&directory)?),
            "package contains a link/reparse point"
        );
        for entry in fs::read_dir(&directory)? {
            let path = entry?.path();
            let relative = path
                .strip_prefix(&root)?
                .to_str()
                .context("non-Unicode package path")?
                .replace('\\', "/");
            relative_path(&relative)?;
            ensure!(
                names.insert(relative.to_ascii_lowercase()),
                "case-colliding package paths"
            );
            ensure!(
                names.len() <= MAX_ENTRIES,
                "package exceeds entry count limit"
            );
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(!linked(&metadata), "package contains a link/reparse point");
            ensure!(
                path.canonicalize()?.starts_with(&root),
                "package path escapes root"
            );
            if metadata.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            ensure!(metadata.is_file(), "package entry is not a regular file");
            let maximum = if matches!(relative.as_str(), "gpui-shell.json" | "conduit-plugin.json")
            {
                MAX_MANIFEST_BYTES
            } else {
                MAX_FILE_BYTES
            };
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
            }
            let file = options.open(&path)?;
            let opened = file.metadata()?;
            ensure!(
                opened.is_file() && !linked(&opened) && opened.len() <= maximum as u64,
                "file is linked or exceeds size limit"
            );
            let mut bytes = Vec::new();
            file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= maximum, "file grew past size limit");
            total = total.saturating_add(bytes.len());
            ensure!(
                total <= MAX_PACKAGE_BYTES,
                "package exceeds total byte limit"
            );
            files.insert(relative, bytes);
        }
    }
    Ok(files)
}

// Sorted UTF-8 path, u64 little-endian lengths, then exact file bytes.
fn digest(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hash = Sha256::new();
    hash.update(b"conduit-development-package-v1\0");
    for (name, bytes) in files {
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
mod tests;
