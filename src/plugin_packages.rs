//! Data-only icon themes. Discovery never evaluates plugin code.
use crate::plugins::{
    contributions::ContributionSet,
    ids::{Owner, PluginId},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path},
    sync::Arc,
};

pub mod catalog;

pub const MANIFEST: &str = "conduit-plugin.json";
const MAX_MANIFEST: u64 = 128 * 1024;
const MAX_ICON: u64 = 64 * 1024;
const MAX_ASSETS: usize = 2 * 1024 * 1024;
const MAX_PACKAGES: usize = 64;

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Preferences {
    pub disabled: BTreeSet<String>,
    pub file_icon_theme: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub file_icons: IconTheme,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub min_app_version: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct IconTheme {
    pub icons: BTreeMap<String, IconDefinition>,
    pub file: Option<String>,
    pub folder: Option<String>,
    pub folder_expanded: Option<String>,
    pub file_names: BTreeMap<String, String>,
    pub file_extensions: BTreeMap<String, String>,
    pub folder_names: BTreeMap<String, String>,
    pub folder_names_expanded: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconDefinition {
    pub path: String,
    #[serde(default)]
    pub light: Option<String>,
    #[serde(default)]
    pub dark: Option<String>,
}
impl IconTheme {
    /// Exact names take precedence; compound suffixes match longest first.
    pub fn resolve(&self, path: &Path, directory: bool, expanded: bool) -> Option<&str> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if directory {
            if expanded {
                self.folder_names_expanded
                    .get(&name)
                    .or_else(|| self.folder_names.get(&name))
                    .or(self.folder_expanded.as_ref())
                    .or(self.folder.as_ref())
            } else {
                self.folder_names.get(&name).or(self.folder.as_ref())
            }
            .map(String::as_str)
        } else {
            self.file_names
                .get(&name)
                .or_else(|| {
                    name.match_indices('.')
                        .find_map(|(at, _)| self.file_extensions.get(&name[at + 1..]))
                })
                .or(self.file.as_ref())
                .map(String::as_str)
        }
    }
}
pub fn color(value: &str) -> Option<u32> {
    let hex = value.strip_prefix('#')?;
    (hex.len() == 6 && hex.is_ascii())
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}
#[derive(Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub assets: BTreeMap<String, Arc<[u8]>>,
    /// Receipt written by the installer, never supplied by a package manifest.
    pub source: Option<catalog::Source>,
}
impl Package {
    pub fn contributions(&self) -> Result<ContributionSet> {
        let mut set =
            ContributionSet::empty(Owner::external(PluginId::new(self.manifest.id.clone())?)?);
        set.file_icon_themes
            .push(crate::plugins::contributions::FileIconThemeDescriptor {
                id: crate::plugins::ids::ItemId::new(format!("{}.file-icons", self.manifest.id))?,
                title: self.manifest.name.clone(),
            });
        Ok(set)
    }
}
#[derive(Default)]
pub struct Discovery {
    pub packages: Vec<Package>,
    pub errors: Vec<String>,
}
fn valid_text(text: &str, max: usize, required: bool) -> bool {
    text.len() <= max
        && (!required || !text.trim().is_empty())
        && !text.chars().any(char::is_control)
}
fn validate_manifest(manifest: &Manifest) -> Result<()> {
    ensure!(
        manifest.schema_version == 1,
        "Unsupported plugin schema version"
    );
    Owner::external(PluginId::new(manifest.id.clone())?)?;
    ensure!(manifest.id.len() <= 64, "Plugin identifier is too long");
    ensure!(
        valid_text(&manifest.name, 100, true) && valid_text(&manifest.description, 1024, false),
        "Invalid plugin name or description"
    );
    semver::Version::parse(&manifest.version).context("Invalid plugin version")?;
    for value in [&manifest.author, &manifest.license].into_iter().flatten() {
        ensure!(valid_text(value, 160, true), "Invalid author or license");
    }
    if let Some(version) = &manifest.min_app_version {
        semver::Version::parse(version).context("Invalid minimum application version")?;
    }
    let theme = &manifest.file_icons;
    ensure!(
        !theme.icons.is_empty() && theme.icons.len() <= 128,
        "An icon theme must define 1–128 icons"
    );
    for (key, icon) in &theme.icons {
        ensure!(valid_text(key, 80, true), "Invalid icon identifier");
        relative_svg(&icon.path)?;
        for value in [&icon.light, &icon.dark].into_iter().flatten() {
            ensure!(color(value).is_some(), "Icon colors must use #RRGGBB");
        }
    }
    let maps = [
        &theme.file_names,
        &theme.file_extensions,
        &theme.folder_names,
        &theme.folder_names_expanded,
    ];
    ensure!(
        maps.iter().map(|m| m.len()).sum::<usize>() <= 1024,
        "Too many icon associations"
    );
    for map in maps {
        for (key, icon) in map {
            ensure!(
                valid_text(key, 160, true)
                    && key == &key.to_lowercase()
                    && !key.contains(['/', '\\', ':']),
                "Icon associations must use lowercase file names or extensions"
            );
            ensure!(theme.icons.contains_key(icon), "Unknown icon: {icon}");
        }
    }
    for icon in [&theme.file, &theme.folder, &theme.folder_expanded]
        .into_iter()
        .flatten()
    {
        ensure!(
            theme.icons.contains_key(icon),
            "Unknown default icon: {icon}"
        );
    }
    Ok(())
}
fn relative_svg(path: &str) -> Result<&Path> {
    let relative = Path::new(path);
    ensure!(
        !path.is_empty()
            && path.len() <= 160
            && !path.contains(['\\', ':'])
            && relative
                .components()
                .all(|p| matches!(p, Component::Normal(_)))
            && relative.extension().is_some_and(|e| e == "svg"),
        "Icon paths must be relative SVG paths"
    );
    Ok(relative)
}
fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= limit,
        "File exceeds size limit: {}",
        path.display()
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "File exceeds size limit");
    Ok(bytes)
}
fn validate_svg(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).context("SVG must be UTF-8")?;
    let doc = roxmltree::Document::parse(text).context("Invalid SVG")?;
    let root = doc.root_element();
    ensure!(
        root.tag_name().name() == "svg"
            && root.tag_name().namespace() == Some("http://www.w3.org/2000/svg"),
        "SVG requires the standard namespace"
    );
    let view_box = root
        .attribute("viewBox")
        .context("SVG requires a viewBox")?
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|v| !v.is_empty())
        .map(str::parse::<f32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        view_box.len() == 4
            && view_box.iter().all(|v| v.is_finite() && v.abs() <= 4096.)
            && view_box[2] > 0.
            && view_box[3] > 0.,
        "Invalid SVG viewBox"
    );
    ensure!(doc.descendants().count() <= 2048, "SVG is too complex");
    for node in doc.descendants().filter(|node| node.is_element()) {
        ensure!(
            matches!(
                node.tag_name().name(),
                "svg"
                    | "g"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "title"
                    | "desc"
            ),
            "Unsupported SVG element: {}",
            node.tag_name().name()
        );
        for attr in node.attributes() {
            ensure!(
                matches!(
                    attr.name(),
                    "viewBox"
                        | "width"
                        | "height"
                        | "d"
                        | "fill"
                        | "stroke"
                        | "stroke-width"
                        | "stroke-linecap"
                        | "stroke-linejoin"
                        | "stroke-miterlimit"
                        | "stroke-dasharray"
                        | "stroke-dashoffset"
                        | "fill-rule"
                        | "clip-rule"
                        | "opacity"
                        | "fill-opacity"
                        | "stroke-opacity"
                        | "transform"
                        | "x"
                        | "y"
                        | "x1"
                        | "y1"
                        | "x2"
                        | "y2"
                        | "cx"
                        | "cy"
                        | "r"
                        | "rx"
                        | "ry"
                        | "points"
                        | "preserveAspectRatio"
                        | "version"
                        | "id"
                ),
                "Unsupported SVG attribute: {}",
                attr.name()
            );
            ensure!(
                !attr.value().to_ascii_lowercase().contains("url("),
                "SVG references are not supported"
            );
        }
    }
    Ok(())
}
pub fn read_package(source: &Path) -> Result<Package> {
    let root = source
        .canonicalize()
        .context("Plugin folder is unavailable")?;
    let manifest_path = root.join(MANIFEST).canonicalize()?;
    ensure!(
        manifest_path.starts_with(&root),
        "Manifest escapes plugin folder"
    );
    let manifest: Manifest = serde_json::from_slice(&read_bounded(&manifest_path, MAX_MANIFEST)?)?;
    validate_manifest(&manifest)?;
    let mut assets = BTreeMap::new();
    let mut total = 0;
    for (id, definition) in &manifest.file_icons.icons {
        let path = root.join(relative_svg(&definition.path)?).canonicalize()?;
        ensure!(path.starts_with(&root), "Icon escapes plugin folder");
        let bytes = read_bounded(&path, MAX_ICON)?;
        total += bytes.len();
        ensure!(total <= MAX_ASSETS, "Plugin assets exceed 2 MiB");
        validate_svg(&bytes).with_context(|| format!("Invalid icon {id}"))?;
        assets.insert(id.clone(), Arc::from(bytes));
    }
    Ok(Package {
        manifest,
        assets,
        source: None,
    })
}
pub fn discover(root: &Path) -> Discovery {
    let mut result = Discovery::default();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return result,
        Err(error) => {
            result.errors.push(error.to_string());
            return result;
        }
    };
    for (index, entry) in entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .enumerate()
    {
        if index >= MAX_PACKAGES {
            result.errors.push("Plugin limit reached (64)".into());
            break;
        }
        let loaded = (|| {
            ensure!(
                entry.file_type()?.is_dir(),
                "Plugin entry is not a directory"
            );
            let canonical_root = root.canonicalize()?;
            ensure!(
                entry.path().canonicalize()?.parent() == Some(canonical_root.as_path()),
                "Plugin folder escapes installation root"
            );
            let mut package = read_package(&entry.path())?;
            package.source = catalog::read_receipt(&entry.path(), &package);
            ensure!(
                entry.file_name().to_string_lossy() == package.manifest.id,
                "Plugin folder and identifier differ"
            );
            Ok(package)
        })();
        match loaded {
            Ok(package) => result.packages.push(package),
            Err(error) => result.errors.push(format!(
                "{}: {error:#}",
                entry.file_name().to_string_lossy()
            )),
        }
    }
    result
        .packages
        .sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    result
}
pub fn install(source: &Path, root: &Path) -> Result<String> {
    install_package(read_package(source)?, root)
}
/// Publish a fully validated snapshot. Online and local installs use the same transaction.
pub(super) fn install_package(package: Package, root: &Path) -> Result<String> {
    validate_manifest(&package.manifest)?;
    if let Some(minimum) = &package.manifest.min_app_version {
        ensure!(
            semver::Version::parse(env!("CARGO_PKG_VERSION"))? >= semver::Version::parse(minimum)?,
            "This plugin requires Conduit {minimum} or newer"
        );
    }
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    ensure!(
        fs::read_dir(&root)?
            .filter_map(Result::ok)
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count()
            < MAX_PACKAGES,
        "Plugin limit reached (64)"
    );
    let target = root.join(&package.manifest.id);
    ensure!(
        !target.exists(),
        "Plugin is already installed; uninstall it before installing another version"
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let staging = root.join(format!(".install-{}-{nonce}", package.manifest.id));
    fs::create_dir(&staging)?;
    let result = (|| -> Result<()> {
        for (id, definition) in &package.manifest.file_icons.icons {
            let path = staging.join(relative_svg(&definition.path)?);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, &package.assets[id])?;
        }
        fs::write(
            staging.join(MANIFEST),
            serde_json::to_vec_pretty(&package.manifest)?,
        )?;
        if let Some(source) = &package.source {
            fs::write(
                staging.join(".conduit-source.json"),
                serde_json::to_vec_pretty(source)?,
            )?;
        }
        fs::rename(&staging, &target).context("Could not complete plugin installation")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result?;
    Ok(package.manifest.id)
}
pub fn uninstall(root: &Path, id: &str) -> Result<()> {
    Owner::external(PluginId::new(id.to_owned())?)?;
    let root = root.canonicalize()?;
    let target = root.join(id);
    if !target.exists() {
        return Ok(());
    }
    ensure!(
        !target.symlink_metadata()?.file_type().is_symlink()
            && target.canonicalize()?.parent() == Some(root.as_path()),
        "Plugin folder escapes installation root"
    );
    // Only remove the exact installed package, never the selected source folder.
    fs::remove_dir_all(target)?;
    Ok(())
}

#[cfg(test)]
mod tests;
