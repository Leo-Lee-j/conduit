use super::*;

fn item() -> Wallpaper {
    Wallpaper {
        id: "gw1rqe".into(),
        path: "https://w.wallhaven.cc/full/gw/wallhaven-gw1rqe.jpg".into(),
        purity: "sfw".into(),
        dimension_x: 3840,
        dimension_y: 2160,
        file_size: 1000,
        file_type: "image/jpeg".into(),
        thumbs: Thumbs {
            small: "https://th.wallhaven.cc/small/gw/gw1rqe.jpg".into(),
        },
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba([240, 20, 80, 255]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    bytes.into_inner()
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "conduit-wallpaper-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn legacy_settings_and_untrusted_preferences_are_safe() {
    let ui: crate::store::UiSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(ui.background, Settings::default());
    let mut awaiting_selection = Settings {
        enabled: true,
        ..Default::default()
    };
    awaiting_selection.sanitize();
    assert!(awaiting_selection.enabled);
    assert!(awaiting_selection.id.is_none());
    let mut settings: Settings =
        serde_json::from_str(r#"{"enabled":true,"id":"../../other","strength":1000}"#).unwrap();
    settings.sanitize();
    assert!(!settings.enabled);
    assert!(settings.id.is_none());
    assert_eq!(settings.strength, MAX_STRENGTH);
    let mut settings: Settings =
        serde_json::from_str(r#"{"enabled":true,"id":"gw1rqe","strength":-10}"#).unwrap();
    settings.sanitize();
    assert!(settings.enabled);
    assert_eq!(settings.strength, MIN_STRENGTH);
    assert!(cached_path(Path::new("cache"), "../tmp").is_err());
}

#[test]
fn query_encodes_user_text_preserves_sfw_and_random_page_seed() {
    let query = Query {
        text: "mountains &purity=111 / 雪".into(),
        category: Category::Anime,
        sort: Sort::Random,
        page: 2,
        seed: Some("7Oo8tO".into()),
        ..Default::default()
    };
    let url = url::Url::parse(&query.url().unwrap()).unwrap();
    let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(pairs["q"], query.text);
    assert_eq!(pairs["purity"], "100");
    assert_eq!(pairs["categories"], "010");
    assert_eq!(pairs["seed"], "7Oo8tO");
    assert_eq!(pairs["page"], "2");
    assert!(!url.as_str().contains("apikey"));
    assert!(
        Query {
            seed: Some("bad&seed".into()),
            ..query.clone()
        }
        .url()
        .is_err()
    );
    assert!(
        Query {
            page: 0,
            ..query.clone()
        }
        .url()
        .is_err()
    );
    let filtered = Query {
        color: Some("0066cc".into()),
        solid_only: true,
        ..query.clone()
    };
    let filtered_url = url::Url::parse(&filtered.url().unwrap()).unwrap();
    let pairs: std::collections::HashMap<_, _> = filtered_url.query_pairs().into_owned().collect();
    assert_eq!(pairs["colors"], "0066cc");
    assert_eq!(pairs["q"], format!("+\"solid color\" {}", query.text));
    assert_eq!(pairs["purity"], "100");
    assert_eq!(pairs["seed"], "7Oo8tO");
    assert_eq!(pairs["page"], "2");
    assert!(
        Query {
            color: Some("bad&purity=111".into()),
            ..query.clone()
        }
        .url()
        .is_err()
    );
    assert!(
        Query {
            text: "id:123".into(),
            solid_only: true,
            ..query
        }
        .url()
        .is_err()
    );
}

#[test]
fn response_urls_cannot_escape_wallhaven_or_bypass_sfw() {
    let mut value = item();
    value.validate().unwrap();
    for url in [
        "http://w.wallhaven.cc/full/gw/wallhaven-gw1rqe.jpg",
        "https://w.wallhaven.cc.evil.test/full/gw/wallhaven-gw1rqe.jpg",
        "https://user@w.wallhaven.cc/full/gw/wallhaven-gw1rqe.jpg",
        "https://w.wallhaven.cc/full/gw/wallhaven-gw1rqe.jpg?redirect=x",
        "https://w.wallhaven.cc/full/gw/wallhaven-abcdef.jpg",
    ] {
        value.path = url.into();
        assert!(value.validate().is_err(), "{url}");
    }
    value = item();
    value.purity = "nsfw".into();
    assert!(value.validate().is_err());
    value = item();
    value.thumbs.small = "https://localhost/test.jpg".into();
    assert!(value.validate().is_err());
    value = item();
    value.dimension_x = 8192;
    value.dimension_y = 8192;
    assert!(!value.supported());
    value = item();
    value.file_size = MAX_FILE as u64 + 1;
    assert!(!value.supported());
    assert!(
        parse_page(
            br#"{"data":[],"meta":{"current_page":2,"last_page":2,"seed":null}}"#,
            &Query {
                page: 1,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn image_work_is_bounded_and_outputs_gpui_channels() {
    let cancel = AtomicBool::new(false);
    let bytes = png(640, 400);
    let thumb = decode(&bytes, true, &cancel).unwrap();
    assert_eq!(thumb.dimensions(), (320, 200));
    assert_eq!(thumb.get_pixel(0, 0).0, [80, 20, 240, 255]);
    let original = decode(&bytes, false, &cancel).unwrap();
    assert_eq!(original.dimensions(), (640, 400));
    assert!(decode(b"GIF89a", true, &cancel).is_err());
    assert!(!valid_dimensions(8192, 8192));
    assert!(!valid_dimensions(0, 200));
    assert!(valid_dimensions(3840, 2160));
    assert!(read_bounded(Cursor::new([0; 30]), 20, &cancel).is_err());
    cancel.store(true, Ordering::Release);
    assert!(decode(&bytes, true, &cancel).is_err());
    assert!(read_bounded(Cursor::new([]), 20, &cancel).is_err());
}

#[test]
fn cache_restores_offline_and_pruning_preserves_current_wallpaper() {
    let root = Directory::new();
    for id in ["aaaaaa", "bbbbbb", "cccccc", "dddddd", "eeeeee", "ffffff"] {
        fs::write(cached_path(&root.0, id).unwrap(), png(12, 8)).unwrap();
    }
    fs::write(root.0.join("unrelated.txt"), "leave alone").unwrap();
    prune(&root.0, &["aaaaaa", "ffffff"]).unwrap();
    let images: Vec<_> = fs::read_dir(&root.0)
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "img"))
        .collect();
    assert_eq!(images.len(), CACHE_FILES);
    assert!(root.0.join("unrelated.txt").exists());
    assert_eq!(
        restore(&root.0, "aaaaaa", &AtomicBool::new(false))
            .unwrap()
            .dimensions(),
        (12, 8)
    );
    assert!(root.0.join("ffffff.img").exists());
    fs::write(root.0.join("aaaaaa.img"), "broken").unwrap();
    assert!(restore(&root.0, "aaaaaa", &AtomicBool::new(false)).is_err());
}

#[test]
fn rate_limit_recovers_after_a_window_without_retry_storms() {
    let now = Instant::now();
    let mut rate = Rate::default();
    for _ in 0..30 {
        rate.take(now).unwrap();
    }
    assert!(rate.take(now).is_err());
    rate.take(now + Duration::from_secs(60)).unwrap();
    rate.blocked_until = Some(now + Duration::from_secs(120));
    assert!(rate.take(now + Duration::from_secs(90)).is_err());
    rate.take(now + Duration::from_secs(121)).unwrap();
}

#[test]
fn failures_translate_when_displayed_and_keep_specific_causes() {
    let _locale = crate::test_support::scoped("en");
    let cancel = AtomicBool::new(false);
    let root = Directory::new();
    let cases = [
        (
            Query {
                page: 1,
                text: "x".repeat(201),
                ..Default::default()
            }
            .url()
            .unwrap_err(),
            Failure::Query,
        ),
        (
            Query {
                page: 1,
                text: "id:50884".into(),
                solid_only: true,
                ..Default::default()
            }
            .url()
            .unwrap_err(),
            Failure::SolidTagConflict,
        ),
        (
            Query {
                page: 1,
                color: Some("bad-color".into()),
                ..Default::default()
            }
            .url()
            .unwrap_err(),
            Failure::Color,
        ),
        (
            parse_page(
                b"not JSON",
                &Query {
                    page: 1,
                    ..Default::default()
                },
            )
            .err()
            .unwrap(),
            Failure::Response,
        ),
        (
            decode(b"GIF89a", false, &cancel).unwrap_err(),
            Failure::Format,
        ),
        (
            read_bounded(Cursor::new([0; 30]), 20, &cancel)
                .unwrap_err()
                .context(Failure::Network),
            Failure::DownloadLimit,
        ),
        (
            restore(&root.0, "aaaaaa", &cancel).unwrap_err(),
            Failure::CacheMissing,
        ),
        (
            anyhow::Error::new(std::io::Error::other("raw transport diagnostic"))
                .context(Failure::Network),
            Failure::Network,
        ),
        (
            anyhow::anyhow!("raw unexpected diagnostic"),
            Failure::Unknown,
        ),
    ];
    let messages: Vec<_> = cases
        .iter()
        .map(|(error, failure)| {
            let message = error_message(error);
            let english = message.render();
            assert_eq!(english, conduit::i18n::text(failure.key()));
            assert_ne!(english, failure.key(), "missing English translation");
            assert!(!english.contains("raw "));
            (message, english, *failure)
        })
        .collect();
    conduit::i18n::set_locale(&conduit::i18n::Locale::legacy());
    for (message, english, failure) in messages {
        let chinese = message.render();
        assert_eq!(chinese, conduit::i18n::text(failure.key()));
        assert_ne!(
            chinese, english,
            "stored errors must follow language changes"
        );
        assert_ne!(chinese, failure.key(), "missing Chinese translation");
    }
}

#[test]
#[ignore = "Explicit live Wallhaven check; anonymous network access only"]
fn live_wallhaven_search_download_and_offline_restore() {
    let cancel = AtomicBool::new(false);
    let page = search(
        &Query {
            page: 1,
            category: Category::General,
            color: Some("0066cc".into()),
            solid_only: true,
            sort: Sort::Latest,
            ..Default::default()
        },
        &cancel,
    )
    .unwrap();
    let item = page
        .data
        .iter()
        .find(|item| item.supported())
        .expect("supported wallpaper");
    let thumb = thumbnail(item, &cancel).unwrap();
    assert!(thumb.width() <= 320 && thumb.height() <= 200);
    let root = Directory::new();
    let download_start = Instant::now();
    let image = download(item, &root.0, None, &cancel).unwrap();
    let download_time = download_start.elapsed();
    assert!(image.width() <= 3840 && image.height() <= 2160);
    let restore_start = Instant::now();
    let restored = restore(&root.0, &item.id, &cancel).unwrap();
    let restore_time = restore_start.elapsed();
    assert_eq!(restored, image);
    eprintln!(
        "wallpaper={} thumbnail={}x{} background={}x{} download_ms={} offline_decode_ms={} pixel_bytes={}",
        item.id,
        thumb.width(),
        thumb.height(),
        image.width(),
        image.height(),
        download_time.as_millis(),
        restore_time.as_millis(),
        image.as_raw().len()
    );
}
