//! Anonymous Wallhaven access and bounded image work. All I/O runs off the UI thread.

use anyhow::{Context as _, Result, bail, ensure};
use image::{ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
mod errors;
use errors::Failure;
pub use errors::error_message;
use std::{
    collections::VecDeque,
    fs,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

pub const PAGE_SIZE: usize = 24;
pub const MAX_FILE: usize = 20 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
pub const MIN_STRENGTH: u8 = 5;
pub const MAX_STRENGTH: u8 = 30;
// Wallhaven accepts only these palette values for its `colors` search parameter.
pub const COLORS: &[&str] = &[
    "660000", "990000", "cc0000", "cc3333", "ea4c88", "993399", "663399", "333399", "0066cc",
    "0099cc", "66cccc", "77cc33", "669900", "336600", "666600", "999900", "cccc33", "ffff00",
    "ffcc33", "ff9900", "ff6600", "cc6633", "996633", "663300", "000000", "999999", "cccccc",
    "ffffff", "424153",
];
const MAX_JSON: usize = 512 * 1024;
const MAX_THUMB: usize = 1024 * 1024;
const CACHE_FILES: usize = 4;
const CACHE_BYTES: u64 = 80 * 1024 * 1024;
static DECODER: Mutex<()> = Mutex::new(());
static RATE: LazyLock<Mutex<Rate>> = LazyLock::new(|| Mutex::new(Rate::default()));
static HTTP: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(8)))
        .timeout_global(Some(Duration::from_secs(25)))
        .build()
        .into()
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub id: Option<String>,
    #[serde(deserialize_with = "strength")]
    pub strength: u8,
}

fn strength<'de, D: serde::Deserializer<'de>>(de: D) -> Result<u8, D::Error> {
    Ok(i64::deserialize(de)?.clamp(i64::from(MIN_STRENGTH), i64::from(MAX_STRENGTH)) as u8)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            id: None,
            strength: 15,
        }
    }
}

impl Settings {
    pub fn sanitize(&mut self) {
        self.strength = self.strength.clamp(MIN_STRENGTH, MAX_STRENGTH);
        if self.id.as_deref().is_some_and(|id| !valid_id(id)) {
            self.id = None;
            self.enabled = false;
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    All,
    General,
    Anime,
    People,
}
impl Category {
    fn mask(self) -> &'static str {
        match self {
            Self::All => "111",
            Self::General => "100",
            Self::Anime => "010",
            Self::People => "001",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Popular,
    Latest,
    Random,
}

#[derive(Clone, Default)]
pub struct Query {
    pub text: String,
    pub category: Category,
    pub sort: Sort,
    pub page: u32,
    pub seed: Option<String>,
    pub color: Option<String>,
    pub solid_only: bool,
}

impl Query {
    fn url(&self) -> Result<String> {
        ensure!(
            self.text.len() <= 200 && (1..=10_000).contains(&self.page),
            Failure::Query
        );
        let mut url = url::Url::parse("https://wallhaven.cc/api/v1/search")?;
        let mut pairs = url.query_pairs_mut();
        // A required, quoted tag can be combined with keywords, unlike `id:50884`.
        let text = if self.solid_only {
            ensure!(
                !self
                    .text
                    .split_whitespace()
                    .any(|part| part.starts_with("id:")),
                Failure::SolidTagConflict
            );
            format!("+\"solid color\" {}", self.text.trim())
        } else {
            self.text.trim().to_owned()
        };
        pairs
            .append_pair("q", text.trim())
            .append_pair("purity", "100")
            .append_pair("categories", self.category.mask())
            .append_pair("atleast", "1920x1080")
            .append_pair("page", &self.page.to_string());
        if let Some(color) = &self.color {
            ensure!(COLORS.contains(&color.as_str()), Failure::Color);
            pairs.append_pair("colors", color);
        }
        match self.sort {
            Sort::Popular => {
                pairs
                    .append_pair("sorting", "toplist")
                    .append_pair("topRange", "1M");
            }
            Sort::Latest => {
                pairs.append_pair("sorting", "date_added");
            }
            Sort::Random => {
                pairs.append_pair("sorting", "random");
                if let Some(seed) = &self.seed {
                    ensure!(valid_seed(seed), Failure::Query);
                    pairs.append_pair("seed", seed);
                }
            }
        }
        drop(pairs);
        Ok(url.into())
    }
}

#[derive(Clone, Deserialize)]
pub struct Wallpaper {
    pub id: String,
    pub path: String,
    pub purity: String,
    pub dimension_x: u32,
    pub dimension_y: u32,
    pub file_size: u64,
    pub file_type: String,
    pub thumbs: Thumbs,
}

#[derive(Clone, Deserialize)]
pub struct Thumbs {
    pub small: String,
}

impl Wallpaper {
    fn validate(&self) -> Result<()> {
        ensure!(
            valid_id(&self.id) && self.purity == "sfw",
            Failure::Response
        );
        let ext = match self.file_type.as_str() {
            "image/jpeg" => "jpg",
            "image/png" => "png",
            _ => bail!(Failure::Format),
        };
        let prefix = &self.id[..2];
        ensure!(
            trusted_url(
                &self.path,
                "w.wallhaven.cc",
                &format!("/full/{prefix}/wallhaven-{}.{ext}", self.id)
            ),
            Failure::Response
        );
        ensure!(
            trusted_url(
                &self.thumbs.small,
                "th.wallhaven.cc",
                &format!("/small/{prefix}/{}.jpg", self.id)
            ),
            Failure::Response
        );
        Ok(())
    }

    pub fn supported(&self) -> bool {
        self.file_size <= MAX_FILE as u64 && valid_dimensions(self.dimension_x, self.dimension_y)
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 6
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}
fn valid_seed(seed: &str) -> bool {
    seed.len() == 6 && seed.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn trusted_url(raw: &str, host: &str, path: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some(host)
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == path
    })
}

#[derive(Deserialize)]
pub struct Page {
    pub data: Vec<Wallpaper>,
    pub meta: Meta,
}
#[derive(Deserialize)]
pub struct Meta {
    pub current_page: u32,
    pub last_page: u32,
    pub seed: Option<String>,
}

fn parse_page(bytes: &[u8], query: &Query) -> Result<Page> {
    ensure!(bytes.len() <= MAX_JSON, Failure::Response);
    let page: Page = serde_json::from_slice(bytes).context(Failure::Response)?;
    ensure!(
        page.data.len() <= PAGE_SIZE && page.meta.current_page == query.page,
        Failure::Response
    );
    ensure!(
        page.meta.seed.as_deref().is_none_or(valid_seed),
        Failure::Response
    );
    for item in &page.data {
        item.validate()?;
    }
    Ok(page)
}

#[derive(Default)]
struct Rate {
    requests: VecDeque<Instant>,
    blocked_until: Option<Instant>,
}
impl Rate {
    fn take(&mut self, now: Instant) -> Result<()> {
        ensure!(
            self.blocked_until.is_none_or(|until| now >= until),
            Failure::RateLimited
        );
        while self
            .requests
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            self.requests.pop_front();
        }
        ensure!(self.requests.len() < 30, Failure::RateLimited);
        self.requests.push_back(now);
        Ok(())
    }
}

pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), Failure::Cancelled);
    Ok(())
}

fn read_bounded(mut reader: impl Read, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        ensure!(data.len() + count <= limit, Failure::DownloadLimit);
        data.extend_from_slice(&buffer[..count]);
    }
    Ok(data)
}

fn get(url: &str, api: bool, limit: usize, cancel: &AtomicBool) -> Result<Vec<u8>> {
    check_cancel(cancel)?;
    if api {
        RATE.lock().unwrap().take(Instant::now())?;
    }
    let response = HTTP
        .get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(if limit == MAX_THUMB {
            8
        } else {
            25
        })))
        .build()
        .header("User-Agent", concat!("Conduit/", env!("CARGO_PKG_VERSION")))
        .call();
    let mut response = match response {
        Err(ureq::Error::StatusCode(429)) => {
            RATE.lock().unwrap().blocked_until = Some(Instant::now() + Duration::from_secs(60));
            bail!(Failure::RateLimited);
        }
        result => result.context(Failure::Network)?,
    };
    ensure!(response.status().as_u16() == 200, Failure::Response);
    read_bounded(response.body_mut().as_reader(), limit, cancel).context(Failure::Network)
}

pub fn search(query: &Query, cancel: &AtomicBool) -> Result<Page> {
    parse_page(&get(&query.url()?, true, MAX_JSON, cancel)?, query)
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= 8192
        && height <= 8192
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// Produces one static, straight-alpha BGRA frame for GPUI, with no UI-thread conversion.
fn decode(bytes: &[u8], thumbnail: bool, cancel: &AtomicBool) -> Result<RgbaImage> {
    let _guard = DECODER.lock().unwrap_or_else(|e| e.into_inner());
    check_cancel(cancel)?;
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context(Failure::Decode)?;
    ensure!(
        matches!(reader.format(), Some(ImageFormat::Jpeg | ImageFormat::Png)),
        Failure::Format
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    let decoder = reader.into_decoder().context(Failure::Decode)?;
    let (width, height) = decoder.dimensions();
    ensure!(
        valid_dimensions(width, height) && decoder.total_bytes() <= MAX_PIXELS * 4,
        Failure::ImageLimit
    );
    let decoded = image::DynamicImage::from_decoder(decoder).context(Failure::Decode)?;
    check_cancel(cancel)?;
    let (max_w, max_h) = if thumbnail { (320, 200) } else { (3840, 2160) };
    let mut pixels = if width > max_w || height > max_h {
        // Integer area sampling avoids the float intermediate buffer of resize filters.
        decoded.thumbnail(max_w, max_h).into_rgba8()
    } else {
        decoded.into_rgba8()
    };
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    check_cancel(cancel)?;
    Ok(pixels)
}

pub fn thumbnail(item: &Wallpaper, cancel: &AtomicBool) -> Result<RgbaImage> {
    item.validate()?;
    decode(
        &get(&item.thumbs.small, false, MAX_THUMB, cancel)?,
        true,
        cancel,
    )
}

pub fn directory(manifest: &Path) -> PathBuf {
    manifest.with_file_name("wallpapers")
}
fn cached_path(root: &Path, id: &str) -> Result<PathBuf> {
    ensure!(valid_id(id), Failure::CacheRead);
    Ok(root.join(format!("{id}.img")))
}

pub fn restore(root: &Path, id: &str, cancel: &AtomicBool) -> Result<RgbaImage> {
    let path = cached_path(root, id)?;
    let file = fs::File::open(path).map_err(|error| {
        let failure = if error.kind() == std::io::ErrorKind::NotFound {
            Failure::CacheMissing
        } else {
            Failure::CacheRead
        };
        anyhow::Error::new(error).context(failure)
    })?;
    ensure!(
        file.metadata().context(Failure::CacheRead)?.len() <= MAX_FILE as u64,
        Failure::ImageLimit
    );
    decode(
        &read_bounded(file, MAX_FILE, cancel).context(Failure::CacheRead)?,
        false,
        cancel,
    )
}

pub fn download(
    item: &Wallpaper,
    root: &Path,
    protected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<RgbaImage> {
    item.validate()?;
    ensure!(item.supported(), Failure::ImageLimit);
    if let Ok(image) = restore(root, &item.id, cancel) {
        return Ok(image);
    }
    check_cancel(cancel)?;
    let bytes = get(&item.path, false, MAX_FILE, cancel)?;
    let pixels = decode(&bytes, false, cancel)?;
    check_cancel(cancel)?;
    fs::create_dir_all(root).context(Failure::CacheWrite)?;
    let path = cached_path(root, &item.id)?;
    let temp = path.with_extension("tmp");
    let saved = (|| -> Result<()> {
        let mut file = fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        check_cancel(cancel)?;
        crate::store::replace_file(&temp, &path)?;
        prune(
            root,
            &[Some(item.id.as_str()), protected]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )
    })();
    if saved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    saved.context(Failure::CacheWrite)?;
    Ok(pixels)
}

fn prune(root: &Path, protected: &[&str]) -> Result<()> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("img")
            || !path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(valid_id)
            || !entry.file_type()?.is_file()
        {
            continue;
        }
        let metadata = entry.metadata()?;
        entries.push((
            path,
            metadata.len(),
            metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        ));
    }
    entries.sort_by_key(|(_, _, modified)| *modified);
    let mut count = entries.len();
    let mut bytes: u64 = entries.iter().map(|(_, bytes, _)| bytes).sum();
    for (path, size, _) in entries {
        if count <= CACHE_FILES && bytes <= CACHE_BYTES {
            break;
        }
        if path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|id| protected.contains(&id))
        {
            continue;
        }
        fs::remove_file(path)?;
        count -= 1;
        bytes -= size;
    }
    ensure!(
        count <= CACHE_FILES && bytes <= CACHE_BYTES,
        Failure::CacheLimit
    );
    Ok(())
}

#[cfg(test)]
mod tests;
