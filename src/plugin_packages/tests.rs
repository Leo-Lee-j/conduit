use super::*;
use std::path::PathBuf;
pub(super) struct Temp(pub(super) PathBuf);
impl Temp {
    pub(super) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "conduit-icon-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/outline-icons")
}
fn copy_example(root: &Path) -> PathBuf {
    let id = install(&example(), root).unwrap();
    root.join(id)
}
#[test]
fn package_install_is_a_snapshot_and_duplicate_is_non_destructive() {
    let dir = Temp::new();
    let source = copy_example(&dir.0.join("source"));
    let root = dir.0.join("installed");
    let id = install(&source, &root).unwrap();
    assert!(install(&source, &root).is_err());
    fs::write(source.join("icons/file.svg"), "corrupted source").unwrap();
    let discovery = discover(&root);
    assert!(discovery.errors.is_empty());
    assert_eq!(discovery.packages.len(), 1);
    assert_eq!(discovery.packages[0].manifest.id, id);
    uninstall(&root, &id).unwrap();
    assert!(source.exists());
    assert!(discover(&root).packages.is_empty());
}
#[test]
fn names_compound_extensions_and_expanded_folders_have_deterministic_precedence() {
    let mut theme = read_package(&example()).unwrap().manifest.file_icons;
    theme
        .file_names
        .insert("types.d.ts".into(), "config".into());
    theme
        .file_extensions
        .insert("d.ts".into(), "markdown".into());
    theme.folder_names.insert("src".into(), "code".into());
    theme
        .folder_names_expanded
        .insert("src".into(), "config".into());
    for (path, directory, open, expected) in [
        ("TYPES.D.TS", false, false, "config"),
        ("a.d.ts", false, false, "markdown"),
        ("a.ts", false, false, "code"),
        ("unknown.xyz", false, false, "file"),
        ("src", true, false, "code"),
        ("src", true, true, "config"),
        ("other", true, false, "folder"),
        ("other", true, true, "folder-open"),
    ] {
        assert_eq!(
            theme.resolve(Path::new(path), directory, open),
            Some(expected)
        );
    }
    theme.file = None;
    assert_eq!(theme.resolve(Path::new("unknown.xyz"), false, false), None);
}
#[test]
fn rejects_traversal_reserved_identity_unknown_icons_and_unknown_capabilities() {
    let original = serde_json::to_value(read_package(&example()).unwrap().manifest).unwrap();
    for path in [
        "../outside.svg",
        "/tmp/outside.svg",
        "C:/outside.svg",
        "icons\\outside.svg",
        "icons/a.png",
    ] {
        let mut manifest: Manifest = serde_json::from_value(original.clone()).unwrap();
        manifest.file_icons.icons.get_mut("file").unwrap().path = path.into();
        assert!(validate_manifest(&manifest).is_err(), "{path}");
    }
    for id in ["conduit.fake", "../escape", "bad", "bad.name/child"] {
        let mut manifest: Manifest = serde_json::from_value(original.clone()).unwrap();
        manifest.id = id.into();
        assert!(validate_manifest(&manifest).is_err());
    }
    let mut manifest: Manifest = serde_json::from_value(original.clone()).unwrap();
    manifest.file_icons.file = Some("missing".into());
    assert!(validate_manifest(&manifest).is_err());
    let mut extra = original;
    extra["scripts"] = serde_json::json!(["main.js"]);
    assert!(serde_json::from_value::<Manifest>(extra).is_err());
}
#[test]
fn rejects_active_external_malformed_and_oversized_svg_content() {
    for body in [
        "<script>run()</script>",
        "<image href='http://example.com/icon.png'/>",
        "<path style='fill:red'/>",
        "<path fill='url(http://example.com)'/>",
    ] {
        let svg =
            format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'>{body}</svg>");
        assert!(validate_svg(svg.as_bytes()).is_err());
    }
    assert!(validate_svg(b"<svg>").is_err());
    let dir = Temp::new();
    let source = copy_example(&dir.0);
    fs::write(
        source.join("icons/file.svg"),
        vec![b' '; MAX_ICON as usize + 1],
    )
    .unwrap();
    assert!(read_package(&source).is_err());
}
#[test]
fn corrupt_packages_do_not_hide_healthy_packages_and_staging_is_ignored() {
    let dir = Temp::new();
    copy_example(&dir.0);
    fs::create_dir(dir.0.join("bad.plugin")).unwrap();
    fs::create_dir(dir.0.join(".install-incomplete")).unwrap();
    let result = discover(&dir.0);
    assert_eq!(result.packages.len(), 1);
    assert_eq!(result.errors.len(), 1);
}
#[test]
fn uninstall_rejects_paths_outside_the_installation_root() {
    let dir = Temp::new();
    let root = dir.0.join("plugins");
    fs::create_dir(&root).unwrap();
    fs::write(dir.0.join("keep"), "keep").unwrap();
    for id in ["..", "../keep", "C:/", "conduit.core"] {
        assert!(uninstall(&root, id).is_err());
    }
    assert_eq!(fs::read_to_string(dir.0.join("keep")).unwrap(), "keep");
}
#[test]
fn package_registration_can_be_revoked_and_preferences_roundtrip() {
    let package = read_package(&example()).unwrap();
    let mut registry = crate::plugins::builtins::register(false).unwrap().registry;
    let handle = registry
        .register(package.contributions().unwrap(), Default::default())
        .unwrap();
    assert!(registry.is_registered(&handle));
    assert_eq!(registry.file_icon_themes().count(), 1);
    let removed = registry.unregister(&handle).unwrap();
    assert_eq!(removed.file_icon_themes.len(), 1);
    assert_eq!(registry.file_icon_themes().count(), 0);
    assert!(!registry.is_registered(&handle));
    let mut forged = package.contributions().unwrap();
    forged.file_icon_themes[0].id = crate::plugins::ids::ItemId::new("other.owner.icons").unwrap();
    assert!(registry.register(forged, Default::default()).is_err());
    let preferences = Preferences {
        disabled: [package.manifest.id.clone()].into(),
        file_icon_theme: Some(package.manifest.id),
    };
    assert_eq!(
        serde_json::from_str::<Preferences>(&serde_json::to_string(&preferences).unwrap()).unwrap(),
        preferences
    );
    assert_eq!(
        serde_json::from_str::<Preferences>("{}").unwrap(),
        Preferences::default()
    );
}
