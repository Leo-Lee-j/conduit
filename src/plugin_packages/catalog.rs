//! GitHub discovery is metadata-only. Installation pins all reads to one commit.
use super::*;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const OFFICIAL_REPOSITORY: &str = "Leo-Lee-j/conduit";
pub const TOPIC: &str = "conduit-plugin";
const INDEX: &str = "conduit-plugins.json";
const PER_PAGE: u32 = 6;
const MAX_INDEX: usize = 256 * 1024;
const MAX_CACHE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub kind: String,
    #[serde(default)]
    pub path: String,
    /// Optional tag or commit; absent means the commit containing this index.
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub min_app_version: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema_version: u32,
    plugins: Vec<Declaration>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Source {
    Github {
        repository: String,
        commit: String,
        path: String,
        sha256: String,
    },
    Bundled {
        repository: String,
        sha256: String,
    },
}
impl Source {
    pub fn repository(&self) -> &str {
        match self {
            Self::Github { repository, .. } | Self::Bundled { repository, .. } => repository,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub plugin: Declaration,
    pub repository: String,
    pub commit: String,
    #[serde(default)]
    pub bundled: bool,
}
impl Entry {
    pub fn official(&self) -> bool {
        self.repository.eq_ignore_ascii_case(OFFICIAL_REPOSITORY)
    }
    pub fn key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.repository.to_lowercase(),
            self.plugin.path,
            self.plugin.id
        )
    }
    pub fn repository_url(&self) -> String {
        format!("https://github.com/{}", self.repository)
    }
    pub fn compatible(&self) -> bool {
        self.plugin.kind == "file-icons"
            && self.plugin.min_app_version.as_ref().is_none_or(|v| {
                semver::Version::parse(v).is_ok_and(|minimum| {
                    semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap() >= minimum
                })
            })
    }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Page {
    pub entries: Vec<Entry>,
    pub has_more: bool,
    pub errors: Vec<String>,
    pub fetched_at: u64,
}

fn valid_commit(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn valid_directory(value: &str) -> bool {
    value.is_empty()
        || (value.len() <= 200
            && !value.contains(['\\', ':', '%', '?', '#'])
            && value.split('/').all(|s| {
                !s.is_empty() && s != "." && s != ".." && !s.chars().any(char::is_control)
            }))
}
/// Accept a repository name or its root GitHub URL, never arbitrary URLs or subtrees.
pub fn repository_name(value: &str) -> Result<String> {
    let raw = value.trim().trim_end_matches('/');
    let name = if raw.starts_with("https://") {
        let url = url::Url::parse(raw)?;
        ensure!(
            url.host_str() == Some("github.com")
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Enter a GitHub repository URL or owner/repository"
        );
        url.path().trim_start_matches('/').to_owned()
    } else {
        raw.to_owned()
    };
    let name = name.strip_suffix(".git").unwrap_or(&name);
    let parts: Vec<_> = name.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|p| !p.is_empty()
                && *p != "."
                && *p != ".."
                && p.len() <= 100
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))),
        "Enter owner/repository or its GitHub URL"
    );
    Ok(name.to_owned())
}
fn validate_declaration(p: &Declaration) -> Result<()> {
    Owner::external(PluginId::new(p.id.clone())?)?;
    ensure!(
        p.id.len() <= 64
            && valid_text(&p.name, 100, true)
            && valid_text(&p.description, 1024, false),
        "Invalid catalog metadata"
    );
    semver::Version::parse(&p.version)?;
    ensure!(valid_directory(&p.path), "Invalid plugin directory");
    ensure!(valid_text(&p.kind, 64, true), "Invalid plugin type");
    if let Some(v) = &p.min_app_version {
        semver::Version::parse(v)?;
    }
    if let Some(v) = &p.license {
        ensure!(valid_text(v, 160, true), "Invalid license");
    }
    if let Some(v) = &p.revision {
        ensure!(valid_text(v, 160, true), "Invalid revision");
    }
    Ok(())
}
fn validate_entry(entry: &Entry) -> Result<()> {
    validate_declaration(&entry.plugin)?;
    ensure!(
        repository_name(&entry.repository)? == entry.repository,
        "Invalid repository"
    );
    if entry.bundled {
        ensure!(
            entry.key() == bundled_entry().key()
                && entry.plugin.version == bundled_entry().plugin.version
                && entry.commit.is_empty(),
            "Unknown bundled plugin"
        );
    } else {
        ensure!(valid_commit(&entry.commit), "Invalid commit");
    }
    Ok(())
}
fn raw_url(repository: &str, commit: &str, path: &str) -> String {
    let mut url = url::Url::parse("https://raw.githubusercontent.com/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .extend(repository.split('/'))
        .push(commit)
        .extend(path.split('/'));
    url.into()
}
fn api_url(repository: &str, revision: &str) -> String {
    let mut url = url::Url::parse("https://api.github.com/repos/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .extend(repository.split('/'))
        .push("commits")
        .push(revision);
    url.into()
}
fn package_path(directory: &str, file: &str) -> String {
    if directory.is_empty() {
        file.to_owned()
    } else {
        format!("{directory}/{file}")
    }
}

trait Transport {
    /// 404 is absence. Authentication, rate limits and malformed responses are errors.
    fn get(&self, address: &str, limit: usize) -> Result<Option<Vec<u8>>>;
}
struct Github {
    agent: ureq::Agent,
    deadline: Instant,
}
impl Github {
    fn new() -> Self {
        Self {
            agent: ureq::Agent::config_builder()
                .https_only(true)
                .max_redirects(0)
                .timeout_connect(Some(Duration::from_secs(8)))
                .timeout_global(Some(Duration::from_secs(12)))
                .build()
                .into(),
            deadline: Instant::now() + Duration::from_secs(45),
        }
    }
}
fn transient(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<ureq::Error>() {
        return matches!(
            error,
            ureq::Error::Io(_)
                | ureq::Error::Timeout(_)
                | ureq::Error::ConnectionFailed
                | ureq::Error::BodyStalled
                | ureq::Error::StatusCode(408 | 500..=599)
        );
    }
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::Interrupted
        )
    })
}
fn retry_once<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    match operation() {
        Err(error) if transient(&error) => operation(),
        result => result,
    }
}
impl Transport for Github {
    fn get(&self, address: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        retry_once(|| self.get_once(address, limit))
    }
}
impl Github {
    fn get_once(&self, address: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        ensure!(
            Instant::now() < self.deadline,
            "GitHub request timed out; retry to refresh the catalog"
        );
        let url = url::Url::parse(address)?;
        ensure!(
            url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some("api.github.com" | "raw.githubusercontent.com")
                )
                && url.username().is_empty()
                && url.password().is_none()
                && url.port_or_known_default() == Some(443),
            "Invalid GitHub host"
        );
        let response = self
            .agent
            .get(address)
            .header("User-Agent", concat!("Conduit/", env!("CARGO_PKG_VERSION")))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call();
        let mut response = match response {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(404)) => return Ok(None),
            Err(ureq::Error::StatusCode(403 | 429)) => anyhow::bail!(
                "GitHub denied the request or its rate limit was reached; retry later"
            ),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            response.status().is_success(),
            "Unexpected GitHub response: {}",
            response.status()
        );
        let mut data = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(limit as u64 + 1)
            .read_to_end(&mut data)?;
        ensure!(data.len() <= limit, "GitHub response exceeds size limit");
        Ok(Some(data))
    }
}
fn commit(transport: &impl Transport, repository: &str, revision: &str) -> Result<String> {
    if valid_commit(revision) {
        return Ok(revision.to_lowercase());
    }
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }
    let bytes = transport
        .get(&api_url(repository, revision), MAX_INDEX)?
        .context("Repository or revision not found")?;
    let commit: Commit = serde_json::from_slice(&bytes)?;
    ensure!(
        valid_commit(&commit.sha),
        "GitHub returned an invalid commit"
    );
    Ok(commit.sha)
}
fn repository(transport: &impl Transport, name: &str) -> Result<Vec<Entry>> {
    let repository = repository_name(name)?;
    let head = commit(transport, &repository, "HEAD")?;
    let declarations =
        if let Some(bytes) = transport.get(&raw_url(&repository, &head, INDEX), MAX_INDEX)? {
            let index: Index = serde_json::from_slice(&bytes)?;
            ensure!(
                index.schema_version == 1 && !index.plugins.is_empty() && index.plugins.len() <= 32,
                "Unsupported or oversized plugin index"
            );
            index.plugins
        } else {
            let bytes = transport
                .get(
                    &raw_url(&repository, &head, MANIFEST),
                    MAX_MANIFEST as usize,
                )?
                .context("No conduit-plugins.json or conduit-plugin.json found")?;
            let manifest: Manifest = serde_json::from_slice(&bytes)?;
            validate_manifest(&manifest)?;
            vec![Declaration {
                id: manifest.id,
                name: manifest.name,
                version: manifest.version,
                description: manifest.description,
                kind: "file-icons".into(),
                path: String::new(),
                revision: None,
                license: manifest.license,
                min_app_version: manifest.min_app_version,
            }]
        };
    let mut ids = BTreeSet::new();
    let mut entries = Vec::new();
    for plugin in declarations {
        validate_declaration(&plugin)?;
        ensure!(
            ids.insert(plugin.id.clone()),
            "Duplicate plugin ID in repository catalog"
        );
        let revision = match &plugin.revision {
            Some(revision) => commit(transport, &repository, revision)?,
            None => head.clone(),
        };
        entries.push(Entry {
            plugin,
            repository: repository.clone(),
            commit: revision,
            bundled: false,
        });
    }
    Ok(entries)
}
fn search_url(query: &str, page: u32) -> String {
    let mut url = url::Url::parse("https://api.github.com/search/repositories").unwrap();
    // User terms cannot inject search qualifiers that remove the required topic.
    let words = query
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .filter(|s| !s.is_empty())
        .map(|word| format!("\"{word}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let q = format!("topic:{TOPIC} fork:false archived:false {words}");
    url.query_pairs_mut()
        .append_pair("q", &q)
        .append_pair("sort", "stars")
        .append_pair("per_page", &PER_PAGE.to_string())
        .append_pair("page", &page.to_string());
    url.into()
}
fn fetch_with(transport: &impl Transport, query: &str, page: u32) -> Result<Page> {
    ensure!(
        query.len() <= 200 && (1..=167).contains(&page),
        "Invalid catalog query"
    );
    let mut result = Page {
        fetched_at: now(),
        ..Default::default()
    };
    if query.contains('/') || query.contains("://") {
        result.entries = repository(transport, query)?;
        return Ok(result);
    }
    if page == 1 {
        result.entries = vec![bundled_entry()];
        match repository(transport, OFFICIAL_REPOSITORY) {
            Ok(entries) => merge(&mut result.entries, entries),
            Err(error) => result
                .errors
                .push(format!("{OFFICIAL_REPOSITORY}: {error:#}")),
        }
    }
    #[derive(Deserialize)]
    struct Repo {
        full_name: String,
    }
    #[derive(Deserialize)]
    struct Search {
        total_count: u64,
        items: Vec<Repo>,
        #[serde(default)]
        incomplete_results: bool,
    }
    let bytes = transport
        .get(&search_url(query, page), MAX_INDEX)?
        .context("GitHub search is unavailable")?;
    let found: Search = serde_json::from_slice(&bytes)?;
    ensure!(
        found.items.len() <= PER_PAGE as usize,
        "Oversized search response"
    );
    result.has_more = (page * PER_PAGE) < found.total_count.min(1000) as u32;
    if found.incomplete_results {
        result
            .errors
            .push("GitHub returned an incomplete search result".into());
    }
    for repo in found.items {
        if repo.full_name.eq_ignore_ascii_case(OFFICIAL_REPOSITORY) {
            continue;
        }
        match repository(transport, &repo.full_name) {
            Ok(entries) => merge(&mut result.entries, entries),
            Err(error) => result.errors.push(format!("{}: {error:#}", repo.full_name)),
        }
    }
    Ok(result)
}
pub fn fetch(query: &str, page: u32) -> Result<Page> {
    fetch_with(&Github::new(), query, page)
}
pub fn merge(entries: &mut Vec<Entry>, incoming: Vec<Entry>) {
    for entry in incoming {
        if let Some(existing) = entries.iter_mut().find(|e| e.key() == entry.key()) {
            // A bundled copy is the reliable fallback until the published version catches up.
            if semver::Version::parse(&entry.plugin.version).ok()
                >= semver::Version::parse(&existing.plugin.version).ok()
            {
                *existing = entry;
            }
        } else if entries.len() < 512 {
            entries.push(entry);
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Default, Serialize, Deserialize)]
struct Cache {
    pages: BTreeMap<String, Page>,
}
fn cache_path(root: &Path) -> std::path::PathBuf {
    root.with_file_name("plugin-catalog.json")
}
fn cache_key(query: &str, page: u32) -> String {
    format!("{page}:{}", query.trim().to_lowercase())
}
fn cache(root: &Path) -> Cache {
    read_bounded(&cache_path(root), MAX_CACHE)
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default()
}
pub fn cached(root: &Path, query: &str, page: u32) -> Option<Page> {
    let mut page = cache(root).pages.remove(&cache_key(query, page))?;
    if page.entries.len() > 512
        || page
            .entries
            .iter()
            .any(|entry| validate_entry(entry).is_err())
    {
        return None;
    }
    page.errors.clear();
    Some(page)
}
pub fn save_cache(root: &Path, query: &str, page: u32, value: &Page) -> Result<()> {
    let mut cache = cache(root);
    if cache.pages.len() >= 8 {
        cache.pages.clear();
    }
    cache.pages.insert(cache_key(query, page), value.clone());
    let path = cache_path(root);
    fs::create_dir_all(path.parent().context("Missing catalog directory")?)?;
    // Cache is disposable; partial writes are ignored on the next read.
    let bytes = serde_json::to_vec(&cache)?;
    ensure!(
        bytes.len() as u64 <= MAX_CACHE,
        "Catalog cache exceeds size limit"
    );
    fs::write(path, bytes)?;
    Ok(())
}
pub fn bundled_entry() -> Entry {
    let index: Index =
        serde_json::from_str(include_str!("../../conduit-plugins.json")).expect("bundled catalog");
    Entry {
        plugin: index
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == "conduit-plugins.outline-icons")
            .expect("official icon theme"),
        repository: OFFICIAL_REPOSITORY.into(),
        commit: String::new(),
        bundled: true,
    }
}
fn bundled_package() -> Result<Package> {
    let manifest: Manifest = serde_json::from_slice(include_bytes!(
        "../../plugins/outline-icons/conduit-plugin.json"
    ))?;
    validate_manifest(&manifest)?;
    let data: [(&str, &[u8]); 6] = [
        (
            "file",
            include_bytes!("../../plugins/outline-icons/icons/file.svg"),
        ),
        (
            "folder",
            include_bytes!("../../plugins/outline-icons/icons/folder.svg"),
        ),
        (
            "folder-open",
            include_bytes!("../../plugins/outline-icons/icons/folder-open.svg"),
        ),
        (
            "code",
            include_bytes!("../../plugins/outline-icons/icons/code.svg"),
        ),
        (
            "config",
            include_bytes!("../../plugins/outline-icons/icons/config.svg"),
        ),
        (
            "markdown",
            include_bytes!("../../plugins/outline-icons/icons/markdown.svg"),
        ),
    ];
    let mut assets = BTreeMap::new();
    for (key, bytes) in data {
        validate_svg(bytes)?;
        assets.insert(key.into(), Arc::from(bytes));
    }
    ensure!(
        manifest.file_icons.icons.len() == assets.len()
            && manifest
                .file_icons
                .icons
                .keys()
                .all(|key| assets.contains_key(key)),
        "Bundled icon definitions and assets differ"
    );
    Ok(Package {
        manifest,
        assets,
        source: None,
    })
}
fn package_digest(package: &Package) -> Result<String> {
    let mut digest = Sha256::new();
    let manifest = serde_json::to_vec(&package.manifest)?;
    digest.update((manifest.len() as u64).to_be_bytes());
    digest.update(manifest);
    for (key, bytes) in &package.assets {
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key);
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn download(transport: &impl Transport, entry: &Entry) -> Result<Package> {
    validate_entry(entry)?;
    ensure!(
        entry.compatible(),
        "Unsupported plugin type or application version"
    );
    let mut package = if entry.bundled {
        bundled_package()?
    } else {
        let bytes = transport
            .get(
                &raw_url(
                    &entry.repository,
                    &entry.commit,
                    &package_path(&entry.plugin.path, MANIFEST),
                ),
                MAX_MANIFEST as usize,
            )?
            .context("Plugin manifest not found at the selected commit")?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&manifest)?;
        ensure!(
            manifest.id == entry.plugin.id && manifest.version == entry.plugin.version,
            "Catalog and plugin identity or version differ"
        );
        let mut assets = BTreeMap::new();
        let mut total = 0;
        for (id, definition) in &manifest.file_icons.icons {
            relative_svg(&definition.path)?;
            let bytes = transport
                .get(
                    &raw_url(
                        &entry.repository,
                        &entry.commit,
                        &package_path(&entry.plugin.path, &definition.path),
                    ),
                    MAX_ICON as usize,
                )?
                .with_context(|| format!("Missing icon: {id}"))?;
            total += bytes.len();
            ensure!(total <= MAX_ASSETS, "Plugin assets exceed 2 MiB");
            validate_svg(&bytes).with_context(|| format!("Invalid icon: {id}"))?;
            assets.insert(id.clone(), Arc::from(bytes));
        }
        Package {
            manifest,
            assets,
            source: None,
        }
    };
    ensure!(
        package.manifest.id == entry.plugin.id && package.manifest.version == entry.plugin.version,
        "Catalog and bundled plugin identity or version differ"
    );
    let sha256 = package_digest(&package)?;
    package.source = Some(if entry.bundled {
        Source::Bundled {
            repository: OFFICIAL_REPOSITORY.into(),
            sha256,
        }
    } else {
        Source::Github {
            repository: entry.repository.clone(),
            commit: entry.commit.clone(),
            path: entry.plugin.path.clone(),
            sha256,
        }
    });
    Ok(package)
}
pub fn install(entry: &Entry, root: &Path) -> Result<String> {
    install_package(download(&Github::new(), entry)?, root)
}
pub(super) fn read_receipt(root: &Path, package: &Package) -> Option<Source> {
    let source: Source =
        serde_json::from_slice(&read_bounded(&root.join(".conduit-source.json"), 8192).ok()?)
            .ok()?;
    repository_name(source.repository()).ok()?;
    match &source {
        Source::Github {
            commit,
            path,
            sha256,
            ..
        } if !valid_commit(commit) || !valid_directory(path) || sha256.len() != 64 => return None,
        Source::Bundled { repository, sha256 }
            if !repository.eq_ignore_ascii_case(OFFICIAL_REPOSITORY) || sha256.len() != 64 =>
        {
            return None;
        }
        _ => {}
    }
    let recorded = match &source {
        Source::Github { sha256, .. } | Source::Bundled { sha256, .. } => sha256,
    };
    if recorded != &package_digest(package).ok()? {
        return None;
    }
    Some(source)
}
#[cfg(test)]
mod tests;
