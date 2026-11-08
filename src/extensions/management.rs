//! Listing filters and reversible local Skill management.
use super::*;
use anyhow::{Result, ensure};

pub const DISABLED_SKILL: &str = "SKILL.md.conduit-disabled";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Active,
    Disabled,
    Managed,
    Duplicates,
}
impl Filter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Active,
        Self::Disabled,
        Self::Managed,
        Self::Duplicates,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => conduit::i18n::text_static("notifications.all"),
            Self::Active => conduit::i18n::text_static("extensions.management.not_disabled"),
            Self::Disabled => conduit::i18n::text_static("extensions.management.disabled"),
            Self::Managed => conduit::i18n::text_static("settings.extensions.plugin_managed"),
            Self::Duplicates => conduit::i18n::text_static("settings.extensions.duplicate_name"),
        }
    }
    pub fn matches(self, entry: &Entry, duplicates: &HashSet<String>, query: &str) -> bool {
        let state = match self {
            Self::All => true,
            Self::Active => !entry.disabled,
            Self::Disabled => entry.disabled,
            Self::Managed => entry.managed(),
            Self::Duplicates => duplicates.contains(&entry.name),
        };
        let text = format!(
            "{} {} {} {} {} {}",
            entry.name,
            entry.description,
            entry.scope,
            entry.path.display(),
            entry.transport,
            entry.target
        )
        .to_lowercase();
        state
            && query
                .to_lowercase()
                .split_whitespace()
                .all(|term| text.contains(term))
    }
}

pub fn duplicates(catalog: &Catalog, kind: Kind) -> HashSet<String> {
    let mut seen = HashSet::new();
    catalog
        .entries
        .iter()
        .filter(|e| e.kind == kind)
        .filter_map(|e| (!seen.insert(e.name.clone())).then_some(e.name.clone()))
        .collect()
}

/// Hold the same lock as the editor and refuse replacement, including races.
pub fn toggle_skill(path: &Path, original: &str) -> Result<PathBuf> {
    let filename = path.file_name().and_then(|name| name.to_str());
    ensure!(
        matches!(filename, Some("SKILL.md" | DISABLED_SKILL)),
        conduit::i18n::text("extensions.management.not_a_manageable_skill_file")
    );
    for item in [
        path,
        path.parent()
            .ok_or_else(|| anyhow::anyhow!(conduit::i18n::text("extensions.edit.invalid_path")))?,
    ] {
        let metadata = fs::symlink_metadata(item)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            conduit::i18n::text(
                "extensions.management.manage_linked_skills_at_their_original_location"
            )
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                conduit::i18n::text(
                    "extensions.management.manage_linked_skills_at_their_original_location"
                )
            );
        }
    }
    ensure!(
        fs::metadata(path)?.is_file(),
        conduit::i18n::text("extensions.management.skill_source_is_not_a_regular_file")
    );
    let active = path.with_file_name("SKILL.md");
    let target = path.with_file_name(if filename == Some("SKILL.md") {
        DISABLED_SKILL
    } else {
        "SKILL.md"
    });
    let lock = active.with_file_name("SKILL.md.conduit-edit-lock");
    let handle = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|_| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.management.this_skill_is_being_edited_try_again"
            ))
        })?;
    let result = (|| -> Result<()> {
        ensure!(
            edit::read(path)?.as_deref() == Some(original),
            conduit::i18n::text("extensions.management.skill_changed_externally_refresh_and_retry")
        );
        ensure!(
            fs::symlink_metadata(&target).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            conduit::i18n::text(
                "extensions.management.target_exists_or_is_inaccessible_and_cannot"
            )
        );
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
            let from: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<_> = target.as_os_str().encode_wide().chain(Some(0)).collect();
            // No REPLACE_EXISTING: an external writer cannot be overwritten.
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        {
            fs::hard_link(path, &target)?;
            fs::remove_file(path)?;
        }
        Ok(())
    })();
    drop(handle);
    let _ = fs::remove_file(lock);
    result?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn skill_disable_restore_rejects_conflicts_and_stale_content() {
        let _locale = crate::test_support::chinese();
        let folder = std::env::temp_dir().join(format!(
            "conduit-skill-toggle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("SKILL.md");
        let text = "---\nname: sample\ndescription: Test\n---\nBody";
        fs::write(&path, text).unwrap();
        fs::write(folder.join("helper.txt"), "keep").unwrap();
        assert!(toggle_skill(&path, "stale").is_err());
        let disabled = toggle_skill(&path, text).unwrap();
        assert!(!path.exists());
        assert_eq!(edit::read(&disabled).unwrap().as_deref(), Some(text));
        let cancel = AtomicBool::new(false);
        let mut scan = Scan::new(&cancel);
        scan.skills(&folder, "全局", 0);
        assert_eq!(scan.catalog.entries.len(), 1);
        assert!(scan.catalog.entries[0].disabled);
        fs::write(&path, "conflict").unwrap();
        assert!(toggle_skill(&disabled, text).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "conflict");
        fs::remove_file(&path).unwrap();
        toggle_skill(&disabled, text).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("helper.txt")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn filters_search_paths_and_keep_same_named_sources() {
        let _locale = crate::test_support::chinese();
        let entry = Entry {
            name: "same".into(),
            description: "Test Skill".into(),
            path: "C:/global/skills/same/SKILL.md".into(),
            scope: "全局".into(),
            kind: Kind::Skill,
            disabled: false,
            transport: String::new(),
            target: String::new(),
        };
        let mut other = entry.clone();
        other.scope = "插件".into();
        other.disabled = true;
        let catalog = Catalog {
            entries: vec![entry.clone(), other.clone()],
            ..Default::default()
        };
        let duplicates = duplicates(&catalog, Kind::Skill);
        assert!(Filter::Duplicates.matches(&entry, &duplicates, "GLOBAL test"));
        assert!(!Filter::Active.matches(&other, &duplicates, ""));
        assert!(Filter::Managed.matches(&other, &duplicates, "same"));
    }
}
