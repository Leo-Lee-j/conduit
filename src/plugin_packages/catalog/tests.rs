use super::*;
use std::cell::RefCell;
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const PIN: &str = "abcdef0123456789abcdef0123456789abcdef01";
#[derive(Default)]
struct Fake {
    responses: BTreeMap<String, Vec<u8>>,
    requests: RefCell<Vec<String>>,
}
impl Transport for Fake {
    fn get(&self, url: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        self.requests.borrow_mut().push(url.into());
        let value = self.responses.get(url).cloned();
        ensure!(
            value.as_ref().is_none_or(|v| v.len() <= limit),
            "Response too large"
        );
        Ok(value)
    }
}
impl Fake {
    fn add(&mut self, url: String, value: impl AsRef<[u8]>) {
        self.responses.insert(url, value.as_ref().to_vec());
    }
    fn repo(&mut self, name: &str) {
        self.add(api_url(name, "HEAD"), format!(r#"{{"sha":"{SHA}"}}"#));
    }
    fn package(&mut self, name: &str, commit: &str, path: &str) {
        let package = bundled_package().unwrap();
        self.add(
            raw_url(name, commit, &package_path(path, MANIFEST)),
            serde_json::to_vec(&package.manifest).unwrap(),
        );
        for (key, icon) in &package.manifest.file_icons.icons {
            self.add(
                raw_url(name, commit, &package_path(path, &icon.path)),
                &package.assets[key],
            );
        }
    }
}
#[test]
fn official_package_is_installable_offline_with_a_receipt_and_survives_source_removal() {
    let temp = super::super::tests::Temp::new();
    let entry = bundled_entry();
    let package = download(&Fake::default(), &entry).unwrap();
    let id = install_package(package, &temp.0).unwrap();
    let loaded = discover(&temp.0);
    assert!(loaded.errors.is_empty());
    assert!(matches!(
        loaded.packages[0].source,
        Some(Source::Bundled { .. })
    ));
    assert!(install(&entry, &temp.0).is_err());
    uninstall(&temp.0, &id).unwrap();
    assert!(discover(&temp.0).packages.is_empty());
}
#[test]
fn topic_search_is_bounded_and_cannot_be_replaced_with_user_qualifiers() {
    let url = url::Url::parse(&search_url("icon topic:other OR stars:>50", 2)).unwrap();
    let query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    assert!(query["q"].starts_with("topic:conduit-plugin fork:false archived:false "));
    assert!(!query["q"].contains("topic:other"));
    assert_eq!(query["page"], "2");
    assert_eq!(query["per_page"], "6");
    assert!(fetch_with(&Fake::default(), "", 168).is_err());
}
#[test]
fn single_plugin_repository_and_download_use_one_immutable_commit() {
    let mut fake = Fake::default();
    fake.repo("someone/icons");
    fake.package("someone/icons", SHA, "");
    let entries = repository(&fake, "https://github.com/someone/icons.git").unwrap();
    assert_eq!(entries.len(), 1);
    assert!(
        !entries[0].official(),
        "a copied official ID does not confer official status"
    );
    let package = download(&fake, &entries[0]).unwrap();
    assert_eq!(package.assets.len(), 6);
    assert!(matches!(package.source, Some(Source::Github { ref commit, .. }) if commit == SHA));
    assert!(
        fake.requests
            .borrow()
            .iter()
            .filter(|u| u.contains("raw.githubusercontent.com"))
            .all(|u| u.contains(SHA))
    );
}
#[test]
fn index_supports_subdirectories_and_independent_version_tags() {
    let mut fake = Fake::default();
    fake.repo("someone/catalog");
    let mut declaration = bundled_entry().plugin;
    declaration.revision = Some("icons/v1.0.0".into());
    fake.add(
        raw_url("someone/catalog", SHA, INDEX),
        serde_json::to_vec(&serde_json::json!({"schema_version":1,"plugins":[declaration]}))
            .unwrap(),
    );
    fake.add(
        api_url("someone/catalog", "icons/v1.0.0"),
        format!(r#"{{"sha":"{PIN}"}}"#),
    );
    fake.package("someone/catalog", PIN, "plugins/outline-icons");
    let entries = repository(&fake, "someone/catalog").unwrap();
    assert_eq!(entries[0].commit, PIN);
    download(&fake, &entries[0]).unwrap();
    assert!(
        fake.requests
            .borrow()
            .iter()
            .filter(|u| u.ends_with(".svg"))
            .all(|u| u.contains(PIN))
    );
}
#[test]
fn bad_repositories_do_not_hide_valid_search_results_and_pagination_is_explicit() {
    let mut fake = Fake::default();
    fake.add(
        search_url("icons", 2),
        br#"{"total_count":13,"items":[{"full_name":"valid/icons"},{"full_name":"broken/icons"}]}"#,
    );
    fake.repo("valid/icons");
    fake.package("valid/icons", SHA, "");
    let page = fetch_with(&fake, "icons", 2).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.errors.len(), 1);
    assert!(page.has_more);
    fake.add(search_url("icons", 3), br#"{"total_count":13,"items":[]}"#);
    assert!(!fetch_with(&fake, "icons", 3).unwrap().has_more);
}
#[test]
fn malformed_sources_paths_and_catalogs_are_rejected_before_installation() {
    for name in [
        "https://evil.test/u/r",
        "https://github.com@evil.test/u/r",
        "../x",
        "u/r/tree/main",
        "https://github.com/u/r?ref=x",
    ] {
        assert!(repository_name(name).is_err(), "{name}");
    }
    let mut entry = bundled_entry();
    entry.bundled = false;
    entry.commit = SHA.into();
    for path in ["../escape", "/absolute", "a/../../b", "a\\b", "a/%2e%2e/b"] {
        entry.plugin.path = path.into();
        assert!(download(&Fake::default(), &entry).is_err());
    }
    let mut fake = Fake::default();
    fake.repo("u/r");
    fake.add(
        raw_url("u/r", SHA, INDEX),
        br#"{"schema_version":9,"plugins":[]}"#,
    );
    assert!(repository(&fake, "u/r").is_err());
}
#[test]
fn identity_mismatch_invalid_svg_and_incompatible_versions_never_publish_a_package() {
    let mut fake = Fake::default();
    fake.repo("u/r");
    fake.package("u/r", SHA, "");
    let mut entry = repository(&fake, "u/r").unwrap().remove(0);
    entry.plugin.version = "9.0.0".into();
    assert!(download(&fake, &entry).is_err());
    entry.plugin.version = "1.0.0".into();
    entry.plugin.min_app_version = Some("999.0.0".into());
    assert!(!entry.compatible());
    assert!(download(&fake, &entry).is_err());
    entry.plugin.min_app_version = None;
    fake.add(raw_url("u/r", SHA, "icons/file.svg"), b"<script/>" as &[u8]);
    assert!(download(&fake, &entry).is_err());
}
#[test]
fn cached_pages_are_scoped_by_query_and_invalid_receipts_are_ignored() {
    let temp = super::super::tests::Temp::new();
    let root = temp.0.join("plugins");
    let page = Page {
        entries: vec![bundled_entry()],
        fetched_at: 123,
        ..Default::default()
    };
    save_cache(&root, "icons", 1, &page).unwrap();
    assert_eq!(cached(&root, "ICONS", 1).unwrap().fetched_at, 123);
    assert!(cached(&root, "other", 1).is_none());
    assert!(cached(&root, "icons", 2).is_none());
    fs::write(cache_path(&root), b"broken").unwrap();
    assert!(cached(&root, "icons", 1).is_none());
    fs::write(temp.0.join(".conduit-source.json"), br#"{"kind":"github","repository":"https://evil.test/u/r","commit":"bad","path":"../x","sha256":"bad"}"#).unwrap();
    assert!(read_receipt(&temp.0, &bundled_package().unwrap()).is_none());
}

#[test]
#[ignore = "explicit public GitHub connectivity smoke; requires network and consumes API quota"]
fn github_public_catalog_connectivity() {
    let transport = Github::new();
    let url = search_url("", 1);
    let result = transport.get(&url, MAX_INDEX);
    println!(
        "GitHub search: {}",
        match &result {
            Ok(Some(bytes)) => format!("{} bytes", bytes.len()),
            Ok(None) => "not found".into(),
            Err(error) => format!("{error:#}"),
        }
    );
    let bytes = result.unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value["items"].is_array());
    let head = commit(&transport, OFFICIAL_REPOSITORY, "HEAD").unwrap();
    println!("Official repository commit: {head}");
    let raw = transport
        .get(&raw_url(OFFICIAL_REPOSITORY, &head, "README.md"), MAX_INDEX)
        .unwrap()
        .unwrap();
    assert!(!raw.is_empty());
    let page = fetch("", 1).unwrap();
    println!(
        "Catalog: {} entries, {} source warnings",
        page.entries.len(),
        page.errors.len()
    );
    assert!(page.entries.iter().any(Entry::official));
}

#[test]
fn network_retry_is_bounded_and_does_not_retry_rate_limits_or_invalid_content() {
    let mut calls = 0;
    let value = retry_once(|| {
        calls += 1;
        if calls == 1 {
            Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into())
        } else {
            Ok(42)
        }
    })
    .unwrap();
    assert_eq!(value, 42);
    assert_eq!(calls, 2);
    let mut calls = 0;
    assert!(
        retry_once::<()>(|| {
            calls += 1;
            Err(ureq::Error::StatusCode(429).into())
        })
        .is_err()
    );
    assert_eq!(calls, 1);
    let mut calls = 0;
    assert!(
        retry_once::<()>(|| {
            calls += 1;
            anyhow::bail!("invalid manifest")
        })
        .is_err()
    );
    assert_eq!(calls, 1);
    let mut calls = 0;
    assert!(
        retry_once::<()>(|| {
            calls += 1;
            Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into())
        })
        .is_err()
    );
    assert_eq!(calls, 2);
}
