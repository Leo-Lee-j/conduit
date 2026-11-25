//! Wallhaven and local backgrounds. All I/O and decoding run off the UI thread.

use anyhow::{Context as _, Result, bail, ensure};
use image::{ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
mod errors;
use errors::Failure;
pub use errors::error_message;
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Cursor, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

pub const PAGE_SIZE: usize = 24;
pub const MIN_STRENGTH: u8 = 5;
pub const MAX_STRENGTH: u8 = 30;
pub const HISTORY_LIMIT: usize = 24;
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
    pub history: Vec<HistoryEntry>,
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
            history: Vec::new(),
        }
    }
}

impl Settings {
    pub fn sanitize(&mut self) {
        self.strength = self.strength.clamp(MIN_STRENGTH, MAX_STRENGTH);
        if self.id.as_deref().is_some_and(|id| !valid_cache_id(id)) {
            self.id = None;
            self.enabled = false;
        }
        let mut seen = std::collections::HashSet::new();
        self.history
            .retain(|entry| entry.valid() && seen.insert(entry.id.clone()));
        self.history.truncate(HISTORY_LIMIT);
        // Keep the previously selected wallpaper accessible when upgrading old settings.
        if let Some(id) = self.id.clone()
            && !self.history.iter().any(|entry| entry.id == id)
        {
            self.remember(HistoryEntry {
                id,
                ..Default::default()
            });
        }
    }

    pub fn remember(&mut self, mut entry: HistoryEntry) {
        if !entry.valid() {
            return;
        }
        if entry.item.is_none() {
            entry.item = self
                .history
                .iter()
                .find(|old| old.id == entry.id)
                .and_then(|old| old.item.clone());
        }
        if entry.local.is_none() {
            entry.local = self
                .history
                .iter()
                .find(|old| old.id == entry.id)
                .and_then(|old| old.local.clone());
        }
        self.history.retain(|old| old.id != entry.id);
        self.history.insert(0, entry);
        self.history.truncate(HISTORY_LIMIT);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub item: Option<Wallpaper>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<LocalImage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalImage {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl HistoryEntry {
    fn valid(&self) -> bool {
        if is_local_id(&self.id) {
            return self.item.is_none()
                && self
                    .local
                    .as_ref()
                    .is_none_or(|local| valid_dimensions(local.width, local.height));
        }
        valid_id(&self.id)
            && self.local.is_none()
            && self
                .item
                .as_ref()
                .is_none_or(|item| item.id == self.id && item.validate().is_ok())
    }

    pub fn dimensions(&self) -> Option<(u32, u32)> {
        self.local
            .as_ref()
            .map(|local| (local.width, local.height))
            .or_else(|| {
                self.item
                    .as_ref()
                    .map(|item| (item.dimension_x, item.dimension_y))
            })
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
    Views,
    Favorites,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Resolution {
    Any,
    #[default]
    FullHd,
    Qhd,
    UltraHd,
}
impl Resolution {
    fn value(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::FullHd => Some("1920x1080"),
            Self::Qhd => Some("2560x1440"),
            Self::UltraHd => Some("3840x2160"),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Ratio {
    #[default]
    Any,
    Wide,
    Ultrawide,
    Portrait,
    Square,
}
impl Ratio {
    fn value(self) -> Option<&'static str> {
        match self {
            Self::Any => None,
            Self::Wide => Some("16x9"),
            Self::Ultrawide => Some("21x9"),
            Self::Portrait => Some("9x16"),
            Self::Square => Some("1x1"),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum TopRange {
    Day,
    Week,
    #[default]
    Month,
    Year,
    HalfYear,
}
impl TopRange {
    fn value(self) -> &'static str {
        match self {
            Self::Day => "1d",
            Self::Week => "1w",
            Self::Month => "1M",
            Self::Year => "1y",
            Self::HalfYear => "6M",
        }
    }
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
    pub resolution: Resolution,
    pub ratio: Ratio,
    pub top_range: TopRange,
}

impl Query {
    pub(super) fn url(&self) -> Result<String> {
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
            .append_pair("order", "desc")
            .append_pair("page", &self.page.to_string());
        if let Some(resolution) = self.resolution.value() {
            pairs.append_pair("atleast", resolution);
        }
        if let Some(ratio) = self.ratio.value() {
            pairs.append_pair("ratios", ratio);
        }
        if let Some(color) = &self.color {
            ensure!(COLORS.contains(&color.as_str()), Failure::Color);
            pairs.append_pair("colors", color);
        }
        match self.sort {
            Sort::Popular => {
                pairs
                    .append_pair("sorting", "toplist")
                    .append_pair("topRange", self.top_range.value());
            }
            Sort::Latest => {
                pairs.append_pair("sorting", "date_added");
            }
            Sort::Views => {
                pairs.append_pair("sorting", "views");
            }
            Sort::Favorites => {
                pairs.append_pair("sorting", "favorites");
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbs {
    pub small: String,
}

impl Wallpaper {
    fn validate(&self) -> Result<()> {
        ensure!(
            valid_id(&self.id)
                && self.purity == "sfw"
                && valid_dimensions(self.dimension_x, self.dimension_y),
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
}
fn is_local_id(id: &str) -> bool {
    id.strip_prefix("local-").is_some_and(|hash| {
        hash.len() == 64
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn valid_cache_id(id: &str) -> bool {
    valid_id(id) || is_local_id(id)
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
    let mut response = response(url, api, limit, cancel)?;
    read_bounded(response.body_mut().as_reader(), limit, cancel).context(Failure::Network)
}

fn response(
    url: &str,
    api: bool,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<ureq::http::Response<ureq::Body>> {
    check_cancel(cancel)?;
    if api {
        RATE.lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(Instant::now())?;
    }
    let response = HTTP
        .get(url)
        .config()
        .timeout_global(Some(Duration::from_secs(if limit == MAX_THUMB {
            8
        } else if limit == usize::MAX {
            300
        } else {
            25
        })))
        .build()
        .header("User-Agent", concat!("Conduit/", env!("CARGO_PKG_VERSION")))
        .call();
    let response = match response {
        Err(ureq::Error::StatusCode(429)) => {
            RATE.lock().unwrap_or_else(|e| e.into_inner()).blocked_until =
                Some(Instant::now() + Duration::from_secs(60));
            bail!(Failure::RateLimited);
        }
        result => result.context(Failure::Network)?,
    };
    ensure!(response.status().as_u16() == 200, Failure::Response);
    Ok(response)
}

pub fn search(query: &Query, cancel: &AtomicBool) -> Result<Page> {
    parse_page(&get(&query.url()?, true, MAX_JSON, cancel)?, query)
}

/// Explicit history application may recover an evicted original, including legacy ID-only entries.
pub fn reapply(
    root: &Path,
    entry: HistoryEntry,
    protected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(RgbaImage, HistoryEntry)> {
    ensure!(entry.valid(), Failure::Response);
    if let Some(item) = &entry.item {
        return download(item, root, protected, cancel).map(|pixels| (pixels, entry));
    }
    let restored = restore(root, &entry.id, cancel);
    if is_local_id(&entry.id) {
        let pixels = restored?;
        fs::OpenOptions::new()
            .write(true)
            .open(cached_path(root, &entry.id)?)?
            .set_modified(SystemTime::now())
            .context(Failure::CacheWrite)?;
        let _ = save_preview(root, &entry.id, &pixels, cancel);
        return Ok((pixels, entry));
    }
    if let Ok(pixels) = restored {
        let _ = save_preview(root, &entry.id, &pixels, cancel);
        return Ok((pixels, entry));
    }
    check_cancel(cancel)?;
    #[derive(Deserialize)]
    struct Detail {
        data: Wallpaper,
    }
    let bytes = get(
        &format!("https://wallhaven.cc/api/v1/w/{}", entry.id),
        true,
        MAX_JSON,
        cancel,
    )?;
    let detail: Detail = serde_json::from_slice(&bytes).context(Failure::Response)?;
    ensure!(detail.data.id == entry.id, Failure::Response);
    download(&detail.data, root, protected, cancel).map(|pixels| {
        (
            pixels,
            HistoryEntry {
                item: Some(detail.data),
                ..entry
            },
        )
    })
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    width > 0 && height > 0
}

/// Produces one static, straight-alpha BGRA frame for GPUI, with no UI-thread conversion.
fn decode(bytes: &[u8], thumbnail: bool, cancel: &AtomicBool) -> Result<RgbaImage> {
    decode_reader(Cursor::new(bytes), thumbnail, cancel)
}

fn decode_reader(
    reader: impl BufRead + Seek,
    thumbnail: bool,
    cancel: &AtomicBool,
) -> Result<RgbaImage> {
    let _guard = DECODER.lock().unwrap_or_else(|e| e.into_inner());
    check_cancel(cancel)?;
    let mut reader = ImageReader::new(reader)
        .with_guessed_format()
        .context(Failure::Decode)?;
    ensure!(
        matches!(
            reader.format(),
            Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Bmp)
        ),
        Failure::Format
    );
    reader.no_limits();
    let decoder = reader.into_decoder().context(Failure::Decode)?;
    let (width, height) = decoder.dimensions();
    ensure!(valid_dimensions(width, height), Failure::Decode);
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
    ensure!(valid_cache_id(id), Failure::CacheRead);
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
    decode_reader(BufReader::new(file), false, cancel)
}

/// History previews never access the network, including for upgraded installations.
pub fn history_thumbnail(root: &Path, id: &str, cancel: &AtomicBool) -> Result<RgbaImage> {
    let original = cached_path(root, id)?;
    let preview = original.with_extension("png");
    for path in [preview, original] {
        check_cancel(cancel)?;
        if let Ok(file) = fs::File::open(path)
            && let Ok(image) = decode_reader(BufReader::new(file), true, cancel)
        {
            return Ok(image);
        }
    }
    bail!(Failure::CacheMissing)
}

fn save_preview(root: &Path, id: &str, pixels: &RgbaImage, cancel: &AtomicBool) -> Result<()> {
    let _guard = DECODER.lock().unwrap_or_else(|e| e.into_inner());
    check_cancel(cancel)?;
    let scale = (320. / f64::from(pixels.width()))
        .min(200. / f64::from(pixels.height()))
        .min(1.);
    let mut preview = image::imageops::thumbnail(
        pixels,
        (f64::from(pixels.width()) * scale).max(1.) as u32,
        (f64::from(pixels.height()) * scale).max(1.) as u32,
    );
    // Render pixels use BGRA; files use RGBA.
    for pixel in preview.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let path = cached_path(root, id)?.with_extension("png");
    let temp = path.with_extension("preview-tmp");
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(&temp)?;
        preview.write_to(&mut file, ImageFormat::Png)?;
        file.sync_all()?;
        drop(file);
        check_cancel(cancel)?;
        crate::store::replace_file(&temp, &path)?;
        let mut previews = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("png")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(valid_cache_id)
                && entry.file_type()?.is_file()
            {
                previews.push((entry.metadata()?.modified()?, path));
            }
        }
        previews.sort_by_key(|(modified, _)| *modified);
        let excess = previews.len().saturating_sub(HISTORY_LIMIT);
        for (_, old) in previews
            .into_iter()
            .filter(|(_, old)| old != &path)
            .take(excess)
        {
            fs::remove_file(old)?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Copy without a file-size ceiling; only a small buffer is held alongside the decoder.
fn copy_image(
    mut reader: impl Read,
    writer: &mut impl Write,
    cancel: &AtomicBool,
) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        check_cancel(cancel)?;
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        writer.write_all(&buffer[..count])?;
        hash.update(&buffer[..count]);
    }
    check_cancel(cancel)?;
    Ok(format!("local-{:x}", hash.finalize()))
}

pub fn import_local(
    root: &Path,
    source: &Path,
    protected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(RgbaImage, HistoryEntry)> {
    check_cancel(cancel)?;
    let source_file = fs::File::open(source).context(Failure::LocalRead)?;
    ensure!(
        source_file
            .metadata()
            .context(Failure::LocalRead)?
            .is_file(),
        Failure::LocalRead
    );
    fs::create_dir_all(root).context(Failure::CacheWrite)?;
    let temp = root.join(format!(
        "local-import-{}-{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos()
    ));
    let result = (|| -> Result<(RgbaImage, HistoryEntry)> {
        let mut file = fs::File::create(&temp).context(Failure::CacheWrite)?;
        let id = copy_image(source_file, &mut file, cancel).context(Failure::LocalRead)?;
        file.sync_all().context(Failure::CacheWrite)?;
        drop(file);
        let mut reader = ImageReader::open(&temp)
            .context(Failure::CacheRead)?
            .with_guessed_format()
            .context(Failure::Decode)?;
        reader.no_limits();
        let dimensions = reader.into_dimensions().context(Failure::Decode)?;
        let pixels = decode_reader(BufReader::new(fs::File::open(&temp)?), false, cancel)?;
        check_cancel(cancel)?;
        let path = cached_path(root, &id)?;
        crate::store::replace_file(&temp, &path).context(Failure::CacheWrite)?;
        let _ = save_preview(root, &id, &pixels, cancel);
        prune_local(
            root,
            &[Some(id.as_str()), protected]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )?;
        Ok((
            pixels,
            HistoryEntry {
                id,
                item: None,
                local: Some(LocalImage {
                    name: source
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    width: dimensions.0,
                    height: dimensions.1,
                }),
            },
        ))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

// Local originals cannot be downloaded again; keep as many as the history can retain.
fn prune_local(root: &Path, protected: &[&str]) -> Result<()> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).context(Failure::CacheRead)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("img")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(is_local_id)
            && entry.file_type()?.is_file()
        {
            entries.push((entry.metadata()?.modified()?, path));
        }
    }
    entries.sort_by_key(|(modified, _)| *modified);
    let excess = entries.len().saturating_sub(HISTORY_LIMIT);
    for (_, path) in entries
        .into_iter()
        .filter(|(_, path)| {
            !path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|id| protected.contains(&id))
        })
        .take(excess)
    {
        fs::remove_file(path).context(Failure::CacheWrite)?;
    }
    Ok(())
}

pub fn download(
    item: &Wallpaper,
    root: &Path,
    protected: Option<&str>,
    cancel: &AtomicBool,
) -> Result<RgbaImage> {
    item.validate()?;
    if let Ok(image) = restore(root, &item.id, cancel) {
        let _ = save_preview(root, &item.id, &image, cancel);
        return Ok(image);
    }
    check_cancel(cancel)?;
    fs::create_dir_all(root).context(Failure::CacheWrite)?;
    let path = cached_path(root, &item.id)?;
    let temp = path.with_extension("tmp");
    let saved = (|| -> Result<RgbaImage> {
        let mut response = response(&item.path, false, usize::MAX, cancel)?;
        let mut file = fs::File::create(&temp)?;
        copy_image(response.body_mut().as_reader(), &mut file, cancel).context(Failure::Network)?;
        file.sync_all()?;
        drop(file);
        let pixels = decode_reader(BufReader::new(fs::File::open(&temp)?), false, cancel)?;
        check_cancel(cancel)?;
        crate::store::replace_file(&temp, &path)?;
        prune(
            root,
            &[Some(item.id.as_str()), protected]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
        )?;
        Ok(pixels)
    })();
    if saved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    let pixels = saved.context(Failure::CacheWrite)?;
    // A preview failure must not prevent a successfully saved wallpaper from being applied.
    let _ = save_preview(root, &item.id, &pixels, cancel);
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
    for (path, _, _) in entries {
        if count <= CACHE_FILES {
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
    }
    ensure!(count <= CACHE_FILES, Failure::CacheLimit);
    Ok(())
}

#[cfg(test)]
mod tests;
