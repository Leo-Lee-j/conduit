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

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(30)))
        .timeout_global(Some(Duration::from_secs(30 * 60)))
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
        "更新地址必须使用 HTTPS"
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
        "不允许的更新下载主机"
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
            .header("Accept", "application/vnd.github+json")
            .call()
            .context("无法访问更新服务，请检查网络或稍后重试")?;
        if response.status().is_redirection() {
            let next = response
                .headers()
                .get("location")
                .context("下载重定向缺少地址")?
                .to_str()?;
            url = allowed_url(url.join(next)?.as_str())?;
        } else {
            return Ok(response);
        }
    }
    bail!("更新下载重定向次数过多")
}

fn metadata(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    get(agent, url)?
        .body_mut()
        .as_reader()
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= MAX_METADATA, "更新信息超出大小限制");
    Ok(bytes)
}

pub fn installed_release(data_dir: &Path) -> Result<Release> {
    let bytes = fs::read(data_dir.join("installed-release.json"))?;
    anyhow::ensure!(
        bytes.len() <= 2 * MAX_METADATA as usize,
        "已安装版本信息过大"
    );
    let mut release: Release = serde_json::from_slice(&bytes)?;
    release.manifest = verify_manifest(&release.raw, &release.signature, PUBLIC_KEY)?;
    anyhow::ensure!(
        release.manifest.version == env!("CARGO_PKG_VERSION"),
        "已安装版本信息不匹配"
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
        "无法检查可用磁盘空间：{}",
        std::io::Error::last_os_error()
    );
    anyhow::ensure!(
        available >= required,
        "磁盘空间不足，至少需要 {} MiB",
        required / (1024 * 1024)
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
        "此构建未配置更新公钥，请安装正式发布版本"
    );
    let client = agent();
    let address = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let data = match metadata(&client, &address) {
        Ok(data) => data,
        Err(error)
            if matches!(
                error.downcast_ref::<ureq::Error>(),
                Some(ureq::Error::StatusCode(404))
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let release: GithubRelease = serde_json::from_slice(&data)?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = semver::Version::parse(release.tag_name.trim_start_matches('v'))?;
    if version <= semver::Version::parse(env!("CARGO_PKG_VERSION"))? {
        return Ok(None);
    }
    let asset = |name: &str| -> Result<&str> {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.browser_download_url.as_str())
            .context("发布缺少更新清单或签名")
    };
    let raw = metadata(&client, asset("update.json")?)?;
    let signature = String::from_utf8(metadata(&client, asset("update.json.sig")?)?)?;
    let manifest = verify_manifest(&raw, &signature, PUBLIC_KEY)?;
    anyhow::ensure!(
        manifest.version == version.to_string(),
        "更新版本与 GitHub 标签不一致"
    );
    anyhow::ensure!(
        release
            .assets
            .iter()
            .any(|a| a.name == manifest.filename && a.browser_download_url == manifest.url),
        "发布缺少清单指定的安装包"
    );
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
        let mut response = get(&agent(), &manifest.url)?;
        let mut output = File::create(&partial)?;
        let mut input = response.body_mut().as_reader();
        let mut buf = [0_u8; 64 * 1024];
        let mut downloaded = 0;
        let mut notified = Instant::now();
        loop {
            anyhow::ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
            let read = input.read(&mut buf)?;
            if read == 0 {
                break;
            }
            downloaded += read as u64;
            anyhow::ensure!(downloaded <= manifest.size, "安装包超出预期大小");
            output.write_all(&buf[..read])?;
            if notified.elapsed() >= Duration::from_millis(200) {
                progress(downloaded);
                notified = Instant::now();
            }
        }
        output.sync_all()?;
        drop(output);
        verify_package(&partial, &manifest)?;
        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
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
mod tests {
    use super::*;
    #[test]
    fn download_addresses_reject_untrusted_hosts_and_transport() {
        for address in [
            "http://github.com/x",
            "https://github.com.evil.test/x",
            "https://user@github.com/x",
            "https://github.com:444/x",
            "file:///x",
        ] {
            assert!(allowed_url(address).is_err(), "{address}");
        }
        assert!(
            allowed_url("https://release-assets.githubusercontent.com/test?token=test").is_ok()
        );
    }
    #[test]
    fn preferences_are_backward_compatible() {
        let p: Preferences = serde_json::from_str("{}").unwrap();
        assert!(p.auto_check && p.auto_download && !p.auto_install);
    }
}
