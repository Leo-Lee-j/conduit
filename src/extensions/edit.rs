//! Explicit native configuration edits, isolated from read-only discovery.
use super::*;
use anyhow::{Result, bail, ensure};
use std::io::Write;

pub fn destination(engine: Provider, workspace: Option<&Path>, kind: Kind) -> PathBuf {
    let locations = Locations::environment();
    match (engine, workspace, kind) {
        (Provider::Claude, Some(root), Kind::Skill) => root.join(".claude/skills"),
        (Provider::Codex, Some(root), Kind::Skill) => root.join(".agents/skills"),
        (Provider::OpenCode, Some(root), Kind::Skill) => root.join(".opencode/skills"),
        (Provider::Claude, None, Kind::Skill) => locations.claude.join("skills"),
        (Provider::Codex, None, Kind::Skill) => locations.home.join(".agents/skills"),
        (Provider::OpenCode, None, Kind::Skill) => locations.opencode.join("skills"),
        (Provider::Claude, Some(root), Kind::Mcp) => root.join(".mcp.json"),
        (Provider::Claude, None, Kind::Mcp) => locations.home.join(".claude.json"),
        (Provider::Codex, Some(root), Kind::Mcp) => root.join(".codex/config.toml"),
        (Provider::Codex, None, Kind::Mcp) => locations.codex.join("config.toml"),
        (Provider::OpenCode, workspace, Kind::Mcp) => {
            let root = workspace.unwrap_or(&locations.opencode);
            let jsonc = root.join("opencode.jsonc");
            if jsonc.exists() {
                jsonc
            } else {
                root.join("opencode.json")
            }
        }
    }
}

pub fn skill_path(folder: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        conduit::i18n::text("extensions.edit.names_may_contain_lowercase_letters_digits_and")
    );
    ensure!(
        !matches!(name, "con" | "prn" | "aux" | "nul")
            && !(name.len() == 4
                && (name.starts_with("com") || name.starts_with("lpt"))
                && name.ends_with(|c: char| c.is_ascii_digit())),
        conduit::i18n::text("extensions.edit.this_name_is_reserved_by_windows_choose")
    );
    Ok(folder.join(name).join("SKILL.md"))
}

pub fn import_skill(source: &Path, destination: &Path) -> Result<PathBuf> {
    let name = source.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        anyhow::anyhow!(conduit::i18n::text(
            "extensions.edit.invalid_skill_directory"
        ))
    })?;
    let target = skill_path(destination, name)?
        .parent()
        .unwrap()
        .to_path_buf();
    let text = read(&source.join("SKILL.md"))?.ok_or_else(|| {
        anyhow::anyhow!(conduit::i18n::text(
            "extensions.edit.selected_folder_does_not_contain_skill_md"
        ))
    })?;
    validate(Path::new("SKILL.md"), &text)?;
    ensure!(
        !target.exists(),
        conduit::i18n::text("extensions.edit.a_skill_with_this_name_already_exists")
    );
    // Inventory first: do not recursively walk our own temporary copy if the
    // destination happens to be below the chosen source.
    let mut files = vec![];
    let mut pending = vec![source.to_path_buf()];
    let mut total = 0u64;
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        ensure!(
            visited <= 2000,
            conduit::i18n::text("extensions.edit.skill_exceeds_2_000_files_or_directories")
        );
        let meta = fs::symlink_metadata(&path)?;
        ensure!(
            !meta.file_type().is_symlink(),
            conduit::i18n::text("extensions.edit.skill_contains_symbolic_links_replace_them_with")
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                meta.file_attributes() & 0x400 == 0,
                conduit::i18n::text("extensions.edit.skill_contains_links_or_reparse_points_and")
            );
        }
        if meta.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else {
            ensure!(
                meta.is_file(),
                conduit::i18n::text("extensions.edit.skill_contains_an_unsupported_file_type")
            );
            total += meta.len();
            ensure!(
                total <= 20 * 1024 * 1024,
                conduit::i18n::text("extensions.edit.skill_exceeds_20_mib_and_cannot_be")
            );
            files.push(path);
        }
    }
    fs::create_dir_all(destination)?;
    let temp = destination.join(format!(
        ".conduit-import-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&temp)?;
    let result = (|| -> Result<()> {
        for path in files {
            let out = temp.join(path.strip_prefix(source)?);
            fs::create_dir_all(out.parent().unwrap())?;
            fs::copy(path, out)?;
        }
        ensure!(
            !target.exists(),
            conduit::i18n::text("extensions.edit.a_skill_with_this_name_already_exists_be36aa")
        );
        fs::rename(&temp, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temp);
    }
    result?;
    Ok(target)
}

pub fn read(path: &Path) -> Result<Option<String>> {
    match fs::File::open(path) {
        Ok(file) => {
            let mut bytes = vec![];
            file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= FILE_LIMIT,
                conduit::i18n::text("extensions.edit.file_exceeds_2_mib_use_an_external")
            );
            Ok(Some(String::from_utf8(bytes).map_err(|_| {
                anyhow::anyhow!(conduit::i18n::text("extensions.edit.file_is_not_utf_8"))
            })?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!(conduit::i18n::text(
            "extensions.edit.unable_to_read_configuration_file"
        )),
    }
}

pub fn validate(path: &Path, text: &str) -> Result<()> {
    ensure!(
        text.len() as u64 <= FILE_LIMIT,
        conduit::i18n::text("extensions.edit.content_exceeds_2_mib")
    );
    if path
        .file_name()
        .is_some_and(|n| n == "SKILL.md" || n == management::DISABLED_SKILL)
    {
        let (name, description) = skill_metadata(text);
        ensure!(
            name.as_ref().is_some_and(|name| !name.trim().is_empty())
                && !description.trim().is_empty(),
            conduit::i18n::text("extensions.edit.skill_md_requires_name_and_description_in")
        );
    } else if path.extension().is_some_and(|e| e == "toml") {
        ensure!(
            toml::from_str::<toml::Table>(text).is_ok(),
            conduit::i18n::text("extensions.edit.invalid_toml_format_check_quotes_and_table")
        );
    } else {
        ensure!(
            jsonc(text).is_some_and(|v| v.is_object()),
            conduit::i18n::text("extensions.edit.configuration_must_be_a_valid_json_jsonc")
        );
    }
    Ok(())
}

/// Refuse stale writes and keep the previous bytes alongside the native file.
/// A per-file lock prevents two Conduit editors from racing each other.
pub fn save(path: &Path, original: Option<&str>, text: &str) -> Result<()> {
    validate(path, text)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            !metadata.file_type().is_symlink(),
            conduit::i18n::text("extensions.edit.this_file_is_a_link_edit_it")
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                conduit::i18n::text("extensions.edit.this_file_is_a_reparse_point_edit")
            );
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!(conduit::i18n::text("extensions.edit.invalid_path")))?;
    fs::create_dir_all(parent)?;
    let lock_source = if path
        .file_name()
        .is_some_and(|n| n == management::DISABLED_SKILL)
    {
        path.with_file_name("SKILL.md")
    } else {
        path.to_path_buf()
    };
    let temp = companion_path(&lock_source, ".conduit-edit-lock");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.edit.this_file_is_being_edited_or_has"
            ))
        })?;
    let result = (|| -> Result<()> {
        ensure!(
            read(path)?.as_deref() == original,
            conduit::i18n::text("extensions.edit.file_changed_externally_close_and_reopen_it")
        );
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Some(original) = original {
            fs::write(companion_path(path, ".conduit-backup"), original)?;
        }
        ensure!(
            read(path)?.as_deref() == original,
            conduit::i18n::text("extensions.edit.file_changed_while_saving_reopen_it_and")
        );
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    #[cfg(windows)]
    let moved = {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<_> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    };
    #[cfg(not(windows))]
    let moved = fs::rename(&temp, path);
    if moved.is_err() {
        let _ = fs::remove_file(&temp);
    }
    moved.map_err(|_| {
        anyhow::anyhow!(conduit::i18n::text(
            "extensions.edit.unable_to_replace_configuration_file_original_preserved"
        ))
    })
}

fn companion_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn template(engine: Provider, kind: Kind) -> &'static str {
    if kind == Kind::Skill {
        return conduit::i18n::text_static(
            "extensions.edit.name_my_skill_description_describe_when_to",
        );
    }
    match engine {
        Provider::Codex => conduit::i18n::text_static(
            "extensions.edit.edit_the_command_and_arguments_before_saving",
        ),
        Provider::Claude => {
            "{\n  \"mcpServers\": {\n    \"my_server\": {\n      \"command\": \"my-server\",\n      \"args\": []\n    }\n  }\n}\n"
        }
        Provider::OpenCode => {
            "{\n  \"mcp\": {\n    \"my_server\": {\n      \"type\": \"local\",\n      \"command\": [\"my-server\"],\n      \"enabled\": false\n    }\n  }\n}\n"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_preserves_original_and_rejects_stale_or_invalid_writes() {
        let _locale = crate::test_support::chinese();
        let dir = std::env::temp_dir().join(format!(
            "conduit-config-edit-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("config.jsonc");
        save(&path, None, "{\n// keep comment\n\"mcp\": {}\n}").unwrap();
        let original = read(&path).unwrap().unwrap();
        assert!(save(&path, None, "{}").is_err());
        assert!(save(&path, Some(&original), "broken").is_err());
        assert_eq!(read(&path).unwrap().unwrap(), original);
        save(&path, Some(&original), "{\"mcp\":{},\"unrelated\":42}").unwrap();
        assert_eq!(
            fs::read_to_string(companion_path(&path, ".conduit-backup")).unwrap(),
            original
        );
        assert!(save(&path, Some(&original), "{}").is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn templates_validate_and_skill_names_cannot_escape_directory() {
        let _locale = crate::test_support::chinese();
        for engine in Provider::ALL {
            let name = if engine == Provider::Codex {
                "config.toml"
            } else {
                "config.json"
            };
            validate(Path::new(name), template(engine, Kind::Mcp)).unwrap();
        }
        validate(
            Path::new("SKILL.md"),
            template(Provider::Codex, Kind::Skill),
        )
        .unwrap();
        for name in ["../escape", "CON", "a/b", "a\\b", "", "name."] {
            assert!(skill_path(Path::new("skills"), name).is_err());
        }
        assert_eq!(
            skill_path(Path::new("skills"), "my-skill").unwrap(),
            Path::new("skills/my-skill/SKILL.md")
        );
    }
}
