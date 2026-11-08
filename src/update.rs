pub mod install;
pub mod manifest;

use anyhow::{Context, Result, bail};
use manifest::{Manifest, verify_manifest, verify_package};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const REPOSITORY: &str = "Leo-Lee-j/conduit";
pub const PUBLIC_KEY: &str = match option_env!("CONDUIT_UPDATE_PUBLIC_KEY") {
    Some(key) => key,
    None => "",
};
const MAX_METADATA: u64 = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub auto_check: bool,
    pub auto_download: bool,
    pub auto_install: bool,
    pub skipped_version: Option<String>,
    pub last_seen_version: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            auto_check: true,
            auto_download: true,
            auto_install: false,
            skipped_version: None,
            last_seen_version: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub manifest: Manifest,
    pub raw: Vec<u8>,
    pub signature: String,
}

#[derive(Debug)]
pub enum Event {
    Checked(Option<Release>),
    Progress { downloaded: u64, total: u64 },
    Downloaded(PathBuf),
    Cancelled,
    Failed(String),
}

pub struct Task {
    pub events: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
}

impl Task {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn allowed_url(value: &str) -> Result<url::Url> {
    let url = url::Url::parse(value)?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443),
        conduit::i18n::text("update.update_url_must_use_https")
    );
    anyhow::ensure!(
        matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        ),
        conduit::i18n::text("update.update_download_host_is_not_allowed")
    );
    Ok(url)
}

/// Follow only GitHub/CDN HTTPS redirects; never leak an authorization header.
fn get(agent: &ureq::Agent, address: &str) -> Result<ureq::http::Response<ureq::Body>> {
    let mut url = allowed_url(address)?;
    for _ in 0..=5 {
        let response = agent
            .get(url.as_str())
            .header("User-Agent", concat!("Conduit/", env!("CARGO_PKG_VERSION")))
            .header(
                "Accept",
                if url.host_str() == Some("api.github.com") {
                    "application/vnd.github+json"
                } else {
                    "application/octet-stream"
                },
            )
            .call()
            .context(conduit::i18n::text_static(
                "update.unable_to_reach_update_service_check_your",
            ))?;
        if response.status().is_redirection() {
            let next = response
                .headers()
                .get("location")
                .context(conduit::i18n::text_static(
                    "update.download_redirect_is_missing_its_url",
                ))?
                .to_str()?;
            url = allowed_url(url.join(next)?.as_str())?;
        } else {
            return Ok(response);
        }
    }
    bail!(conduit::i18n::text(
        "update.too_many_update_download_redirects"
    ))
}

// Retry only transient failures, never malformed data, TLS verification or
// allowlist/signature failures. Bounded backoff avoids amplifying outages.
fn transient(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<ureq::Error>() {
        return matches!(
            error,
            ureq::Error::StatusCode(408 | 429 | 500..=599)
                | ureq::Error::Io(_)
                | ureq::Error::Timeout(_)
                | ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
                | ureq::Error::BodyStalled
        );
    }
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::Interrupted
                | std::io::ErrorKind::WouldBlock
        )
    })
}

fn retry_metadata<T>(
    mut request: impl FnMut() -> Result<T>,
    mut sleep: impl FnMut(Duration),
) -> Result<T> {
    for attempt in 0..3 {
        match request() {
            Err(error) if attempt < 2 && transient(&error) => {
                sleep(Duration::from_millis(500 << attempt));
            }
            result => return result,
        }
    }
    unreachable!()
}

fn metadata(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    retry_metadata(
        || {
            let mut bytes = Vec::new();
            get(agent, url)?
                .body_mut()
                .as_reader()
                .take(MAX_METADATA + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_METADATA,
                conduit::i18n::text("update.update_information_exceeds_the_size_limit")
            );
            Ok(bytes)
        },
        thread::sleep,
    )
}

pub fn installed_release(data_dir: &Path) -> Result<Release> {
    let bytes = fs::read(data_dir.join("installed-release.json"))?;
    anyhow::ensure!(
        bytes.len() <= 2 * MAX_METADATA as usize,
        conduit::i18n::text("update.installed_version_information_is_too_large")
    );
    let mut release: Release = serde_json::from_slice(&bytes)?;
    release.manifest = verify_manifest(&release.raw, &release.signature, PUBLIC_KEY)?;
    anyhow::ensure!(
        release.manifest.version == env!("CARGO_PKG_VERSION"),
        conduit::i18n::text("update.installed_version_information_mismatch")
    );
    Ok(release)
}

pub fn ensure_disk_space(directory: &Path, required: u64) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = directory
        .canonicalize()?
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut available = 0;
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    anyhow::ensure!(
        result != 0,
        conduit::i18n::format(
            "update.unable_to_check_available_disk_space",
            &[("arg0", format!("{}", std::io::Error::last_os_error()))]
        )
    );
    anyhow::ensure!(
        available >= required,
        conduit::i18n::format(
            "update.insufficient_disk_space_at_least_mib_required",
            &[("arg0", format!("{}", required / (1024 * 1024)))]
        )
    );
    Ok(())
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}
#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

pub fn check() -> Task {
    let (tx, events) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    thread::spawn(move || {
        let result = check_release();
        let _ = tx.send(match result {
            Ok(release) => Event::Checked(release),
            Err(error) => Event::Failed(format!("{error:#}")),
        });
    });
    Task { events, cancel }
}

fn check_release() -> Result<Option<Release>> {
    anyhow::ensure!(
        !PUBLIC_KEY.is_empty(),
        conduit::i18n::text("update.this_build_has_no_update_public_key")
    );
    let client = agent(Duration::from_secs(45));
    check_release_with(
        |url| metadata(&client, url),
        PUBLIC_KEY,
        env!("CARGO_PKG_VERSION"),
    )
}

fn check_release_with(
    mut fetch: impl FnMut(&str) -> Result<Vec<u8>>,
    public_key: &str,
    current_version: &str,
) -> Result<Option<Release>> {
    anyhow::ensure!(
        !public_key.is_empty(),
        conduit::i18n::text("update.this_build_has_no_update_public_key")
    );
    let address = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let data = match fetch(&address) {
        Ok(data) => data,
        Err(error)
            if matches!(
                error.downcast_ref::<ureq::Error>(),
                Some(ureq::Error::StatusCode(404))
            ) =>
        {
            return Ok(None);
        }
        // Anonymous API access may be rate limited independently of GitHub's
        // release downloads. Only service/transport errors use this fallback;
        // invalid API data and security errors must still fail closed.
        Err(error)
            if transient(&error)
                || matches!(
                    error.downcast_ref::<ureq::Error>(),
                    Some(ureq::Error::StatusCode(403))
                ) =>
        {
            return check_signed_latest(&mut fetch, public_key, current_version).with_context(
                || {
                    conduit::i18n::format(
                        "update.github_api_unavailable_fallback_to_the_official",
                        &[("error", format!("{:#}", error))],
                    )
                },
            );
        }
        Err(error) => return Err(error),
    };
    let release: GithubRelease = serde_json::from_slice(&data)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = semver::Version::parse(release.tag_name.trim_start_matches('v'))?;
    if version <= semver::Version::parse(current_version)? {
        return Ok(None);
    }
    let asset = |name: &str| -> Result<&str> {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
            .context(conduit::i18n::text_static(
                "update.release_is_missing_its_update_manifest_or",
            ))
    };
    let raw = fetch(asset("update.json")?)?;
    let signature = String::from_utf8(fetch(asset("update.json.sig")?)?)?;
    let manifest = verify_manifest(&raw, &signature, public_key)?;
    anyhow::ensure!(
        manifest.version == version.to_string(),
        conduit::i18n::text("update.update_version_does_not_match_the_github")
    );
    anyhow::ensure!(
        release
            .assets
            .iter()
            .any(|a| a.name == manifest.filename && a.browser_download_url == manifest.url),
        conduit::i18n::text("update.release_is_missing_the_installer_specified_in")
    );
    Ok(Some(Release {
        manifest,
        raw,
        signature,
    }))
}

/// Fixed official endpoint; the signed manifest still restricts package host,
/// repository, target, version and hash, even without the GitHub API listing.
fn check_signed_latest(
    fetch: &mut impl FnMut(&str) -> Result<Vec<u8>>,
    public_key: &str,
    current_version: &str,
) -> Result<Option<Release>> {
    let base = format!("https://github.com/{REPOSITORY}/releases/latest/download");
    let raw = fetch(&format!("{base}/update.json"))?;
    // Pin the signature to this manifest's version: Latest can change between
    // requests. Validate the untrusted fields before using them in a URL, then
    // authenticate the original bytes before making any update decision.
    let candidate: Manifest = serde_json::from_slice(&raw)?;
    candidate.validate()?;
    let signature_url = format!(
        "https://github.com/{REPOSITORY}/releases/download/v{}/update.json.sig",
        candidate.version
    );
    let signature = String::from_utf8(fetch(&signature_url)?)?;
    let manifest = verify_manifest(&raw, &signature, public_key)?;
    if semver::Version::parse(&manifest.version)? <= semver::Version::parse(current_version)? {
        return Ok(None);
    }
    Ok(Some(Release {
        manifest,
        raw,
        signature,
    }))
}

pub fn download(release: Release, cache: PathBuf) -> Task {
    let (tx, events) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancelled = cancel.clone();
    thread::spawn(move || {
        let result = download_release(&release, &cache, &cancelled, |downloaded| {
            let _ = tx.send(Event::Progress {
                downloaded,
                total: release.manifest.size,
            });
        });
        let event = match result {
            Ok(path) => Event::Downloaded(path),
            Err(_) if cancelled.load(Ordering::Relaxed) => Event::Cancelled,
            Err(error) => Event::Failed(format!("{error:#}")),
        };
        let _ = tx.send(event);
    });
    Task { events, cancel }
}

fn download_release(
    release: &Release,
    cache: &Path,
    cancel: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<PathBuf> {
    // Revalidate even when the UI received this Release earlier.
    let manifest = verify_manifest(&release.raw, &release.signature, PUBLIC_KEY)?;
    fs::create_dir_all(cache)?;
    let path = cache.join(&manifest.filename);
    if verify_package(&path, &manifest).is_ok() {
        return Ok(path);
    }
    ensure_disk_space(cache, manifest.size + 32 * 1024 * 1024)?;
    let partial = cache.join(format!("{}.{}.part", manifest.filename, std::process::id()));
    let result = (|| {
        let mut response = get(&agent(Duration::from_secs(30 * 60)), &manifest.url)?;
        let mut output = File::create(&partial)?;
        let mut input = response.body_mut().as_reader();
        let mut buf = [0_u8; 64 * 1024];
        let mut downloaded = 0;
        let mut notified = Instant::now();
        loop {
            anyhow::ensure!(
                !cancel.load(Ordering::Relaxed),
                conduit::i18n::text("update.download_cancelled")
            );
            let read = input.read(&mut buf)?;
            if read == 0 {
                break;
            }
            downloaded += read as u64;
            anyhow::ensure!(
                downloaded <= manifest.size,
                conduit::i18n::text("update.installer_exceeds_the_expected_size")
            );
            output.write_all(&buf[..read])?;
            if notified.elapsed() >= Duration::from_millis(200) {
                progress(downloaded);
                notified = Instant::now();
            }
        }
        output.sync_all()?;
        drop(output);
        verify_package(&partial, &manifest)?;
        anyhow::ensure!(
            !cancel.load(Ordering::Relaxed),
            conduit::i18n::text("update.download_cancelled")
        );
        if path.exists() {
            fs::remove_file(&path)?;
        }
        fs::rename(&partial, &path)?;
        progress(manifest.size);
        Ok(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests;
