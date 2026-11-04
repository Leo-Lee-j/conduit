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
        "名称只允许小写字母、数字和连字符（最多 80 字符）"
    );
    ensure!(
        !matches!(name, "con" | "prn" | "aux" | "nul")
            && !(name.len() == 4
                && (name.starts_with("com") || name.starts_with("lpt"))
                && name.ends_with(|c: char| c.is_ascii_digit())),
        "此名称被 Windows 保留，请使用其他名称"
    );
    Ok(folder.join(name).join("SKILL.md"))
}

pub fn import_skill(source: &Path, destination: &Path) -> Result<PathBuf> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("无效的技能目录"))?;
    let target = skill_path(destination, name)?
        .parent()
        .unwrap()
        .to_path_buf();
    let text = read(&source.join("SKILL.md"))?
        .ok_or_else(|| anyhow::anyhow!("所选文件夹中没有 SKILL.md"))?;
    validate(Path::new("SKILL.md"), &text)?;
    ensure!(!target.exists(), "同名技能已存在，请更改文件夹名称后重试");
    // Inventory first: do not recursively walk our own temporary copy if the
    // destination happens to be below the chosen source.
    let mut files = vec![];
    let mut pending = vec![source.to_path_buf()];
    let mut total = 0u64;
    let mut visited = 0;
    while let Some(path) = pending.pop() {
        visited += 1;
        ensure!(visited <= 2000, "技能超过 2000 个文件或目录，无法导入");
        let meta = fs::symlink_metadata(&path)?;
        ensure!(
            !meta.file_type().is_symlink(),
            "技能包含符号链接，请使用普通文件后导入"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                meta.file_attributes() & 0x400 == 0,
                "技能包含链接或重解析点，无法导入"
            );
        }
        if meta.is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        } else {
            ensure!(meta.is_file(), "技能包含不支持的文件类型");
            total += meta.len();
            ensure!(total <= 20 * 1024 * 1024, "技能超过 20 MiB，无法导入");
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
        ensure!(!target.exists(), "同名技能已存在");
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
                "文件超过 2 MiB，请使用外部编辑器"
            );
            Ok(Some(
                String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("文件不是 UTF-8"))?,
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => bail!("无法读取配置文件"),
    }
}

pub fn validate(path: &Path, text: &str) -> Result<()> {
    ensure!(text.len() as u64 <= FILE_LIMIT, "内容超过 2 MiB");
    if path.file_name().is_some_and(|n| n == "SKILL.md") {
        let (name, description) = skill_metadata(text);
        ensure!(
            name.is_some() && !description.is_empty(),
            "SKILL.md 需要 YAML 头部中的 name 和 description"
        );
    } else if path.extension().is_some_and(|e| e == "toml") {
        ensure!(
            toml::from_str::<toml::Table>(text).is_ok(),
            "TOML 格式无效，请检查引号和表定义"
        );
    } else {
        ensure!(
            jsonc(text).is_some_and(|v| v.is_object()),
            "配置必须是有效的 JSON / JSONC 对象"
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
            "此文件是链接，请在原始文件位置编辑"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.file_attributes() & 0x400 == 0,
                "此文件是重解析点，请在原始文件位置编辑"
            );
        }
    }
    let parent = path.parent().ok_or_else(|| anyhow::anyhow!("无效路径"))?;
    fs::create_dir_all(parent)?;
    let temp = companion_path(path, ".conduit-edit-lock");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| anyhow::anyhow!("该文件正在编辑或有未清理的保存文件，请稍后重试"))?;
    let result = (|| -> Result<()> {
        ensure!(
            read(path)?.as_deref() == original,
            "文件已被外部修改，请关闭并重新打开，避免覆盖其他修改"
        );
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Some(original) = original {
            fs::write(companion_path(path, ".conduit-backup"), original)?;
        }
        ensure!(
            read(path)?.as_deref() == original,
            "文件在保存期间被修改，请重新打开后重试"
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
    moved.map_err(|_| anyhow::anyhow!("无法替换配置文件；原文件仍保留"))
}

fn companion_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn template(engine: Provider, kind: Kind) -> &'static str {
    if kind == Kind::Skill {
        return "---\nname: my-skill\ndescription: 描述使用场景\n---\n\n在这里编写技能指令。\n";
    }
    match engine {
        Provider::Codex => {
            "# 修改命令与参数后保存\n[mcp_servers.my_server]\ncommand = \"my-server\"\nargs = []\nenabled = false\n"
        }
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
