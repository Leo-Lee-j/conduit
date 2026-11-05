use super::*;

fn source() -> tempfile::TempDir {
    let source = tempfile::tempdir().unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/development-overview");
    for name in [
        "gpui-shell.json",
        "conduit-plugin.json",
        "navigation.js",
        "summary.js",
    ] {
        fs::copy(fixture.join(name), source.path().join(name)).unwrap();
    }
    source
}

#[test]
fn snapshot_is_independent_digest_covers_every_file_and_drop_cleans_only_owned_copy() {
    let source = source();
    let staging = tempfile::tempdir().unwrap();
    let snapshot = PackageSnapshot::prepare(source.path(), staging.path()).unwrap();
    let first = snapshot.digest().to_owned();
    fs::write(
        source.path().join("summary.js"),
        "throw new Error('source changed')",
    )
    .unwrap();
    snapshot.verify().unwrap();
    assert_ne!(first, PackageSnapshot::inspect(source.path()).unwrap().1);
    fs::write(snapshot.root().join("summary.js"), "changed").unwrap();
    assert!(snapshot.verify().is_err());
    let copied = snapshot.root().to_path_buf();
    drop(snapshot);
    assert!(!copied.exists());
    assert!(source.path().join("summary.js").exists());
    assert!(staging.path().exists());
}

#[test]
fn inspection_never_evaluates_javascript_and_missing_entries_fail() {
    let source = source();
    fs::write(source.path().join("navigation.js"), "while(true) {}").unwrap();
    assert!(PackageSnapshot::inspect(source.path()).is_ok());
    fs::remove_file(source.path().join("summary.js")).unwrap();
    assert!(PackageSnapshot::inspect(source.path()).is_err());
}

#[test]
fn package_count_size_and_manifest_limits_fail_closed() {
    let source = source();
    for i in 0..MAX_ENTRIES {
        fs::write(source.path().join(format!("{i}.js")), "").unwrap();
    }
    assert!(PackageSnapshot::inspect(source.path()).is_err());
    let source = self::source();
    let file = fs::File::create(source.path().join("large.js")).unwrap();
    file.set_len(MAX_FILE_BYTES as u64 + 1).unwrap();
    assert!(PackageSnapshot::inspect(source.path()).is_err());
    let source = self::source();
    let file = fs::File::create(source.path().join("conduit-plugin.json")).unwrap();
    file.set_len(MAX_MANIFEST_BYTES as u64 + 1).unwrap();
    assert!(PackageSnapshot::inspect(source.path()).is_err());
}

#[cfg(windows)]
#[test]
fn junctions_are_rejected_without_reading_their_target() {
    use std::process::Command;
    let source = source();
    let outside = tempfile::tempdir().unwrap();
    let link = source.path().join("linked");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(PackageSnapshot::inspect(source.path()).is_err());
    assert!(PackageSnapshot::inspect(&link).is_err());
    fs::remove_dir(&link).unwrap();
    assert!(outside.path().exists());
}

#[test]
fn digest_uses_explicit_file_boundaries() {
    assert_ne!(
        digest(&BTreeMap::from([("a".into(), b"bc".to_vec())])),
        digest(&BTreeMap::from([("ab".into(), b"c".to_vec())]))
    );
}
