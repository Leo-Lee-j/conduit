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
            Self::All => "全部",
            Self::Active => "未禁用",
            Self::Disabled => "已停用",
            Self::Managed => "插件托管",
            Self::Duplicates => "同名定义",
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
        "不是可管理的 Skill 文件"
    );
    for item in [
        path,
        path.parent().ok_or_else(|| anyhow::anyhow!("无效路径"))?,
    ] {
        let metadata = fs::symlink_metadata(item)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "链接 Skill 请在原始位置管理"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "链接 Skill 请在原始位置管理"
            );
        }
    }
    ensure!(fs::metadata(path)?.is_file(), "Skill 来源不是普通文件");
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
        .map_err(|_| anyhow::anyhow!("此 Skill 正在编辑，请稍后重试"))?;
    let result = (|| -> Result<()> {
        ensure!(
            edit::read(path)?.as_deref() == Some(original),
            "Skill 已被外部修改，请刷新后重试"
        );
        ensure!(
            fs::symlink_metadata(&target).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "目标文件已存在或不可访问，不能覆盖"
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
