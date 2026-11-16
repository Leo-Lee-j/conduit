//! Provenance only: no environment variables, headers, or credentials are persisted here.
use super::{Item, Payload, PreparedSkill, digest, fail, now, text};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Receipt {
    schema_version: u8,
    pub item_id: String,
    pub kind: String,
    pub path: PathBuf,
    pub name: String,
    revision: String,
    source_url: String,
    #[serde(default)]
    source_path: Option<String>,
    created_at: u64,
    files: Vec<(String, String)>,
}
impl Receipt {
    pub fn skill(skill: &PreparedSkill, path: &Path, files: Vec<(String, String)>) -> Self {
        Self {
            schema_version: 1,
            item_id: skill.item.id.clone(),
            kind: "skill".into(),
            path: path.into(),
            name: skill.name.clone(),
            revision: skill.commit.clone(),
            source_url: skill.item.source_url.clone(),
            source_path: Some(skill.directory.clone()),
            created_at: now(),
            files,
        }
    }
    pub fn mcp(item: &Item, path: &Path, name: &str) -> Self {
        Self {
            schema_version: 1,
            item_id: item.id.clone(),
            kind: "mcp".into(),
            path: path.into(),
            name: name.into(),
            revision: item.version().into(),
            source_url: item.source_url.clone(),
            source_path: None,
            created_at: now(),
            files: vec![],
        }
    }
    fn filename(&self) -> String {
        format!(
            "{}.json",
            digest(format!("{}:{}:{}", self.kind, path_key(&self.path), self.name).as_bytes())
        )
    }
}
fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .to_lowercase()
}
pub fn root() -> PathBuf {
    crate::store::Store::path().with_file_name("extension-installs")
}
pub fn write(root: &Path, receipt: &Receipt) -> Result<()> {
    let path = root.join(receipt.filename());
    let original = crate::extensions::edit::read(&path)?;
    crate::extensions::edit::save(&path, original.as_deref(), &serde_json::to_string(receipt)?)
}
pub(super) fn remove(root: &Path, receipt: &Receipt) -> Result<()> {
    fs::remove_file(root.join(receipt.filename())).map_err(Into::into)
}
fn read_all(root: &Path) -> Result<Vec<Receipt>> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut result = Vec::new();
    let mut total = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.len() != 69
            || !name.ends_with(".json")
            || !name[..64].bytes().all(|b| b.is_ascii_hexdigit())
        {
            continue;
        }
        ensure!(result.len() < 1000, text("receipt_limit"));
        let mut bytes = Vec::new();
        fs::File::open(entry.path())?
            .take(512 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        total += bytes.len();
        ensure!(
            bytes.len() <= 512 * 1024 && total <= 8 * 1024 * 1024,
            text("receipt_limit")
        );
        let receipt: Receipt =
            serde_json::from_slice(&bytes).map_err(|_| fail("invalid_receipt"))?;
        ensure!(
            receipt.schema_version == 1 && receipt.filename() == name,
            text("invalid_receipt")
        );
        result.push(receipt);
    }
    Ok(result)
}
pub fn manual_mcp(entries: &[crate::extensions::Entry]) -> HashSet<String> {
    let receipts = read_all(&root());
    manual_from(entries, receipts.as_deref())
}
fn manual_from(
    entries: &[crate::extensions::Entry],
    receipts: std::result::Result<&[Receipt], &anyhow::Error>,
) -> HashSet<String> {
    entries
        .iter()
        .filter(|entry| {
            entry.kind == crate::extensions::Kind::Mcp
                && receipts.as_ref().map_or(true, |records| {
                    records.iter().any(|r| {
                        r.kind == "mcp"
                            && path_key(&r.path) == path_key(&entry.path)
                            && r.name == entry.name
                    })
                })
        })
        .map(|e| e.id())
        .collect()
}
pub fn installed(
    root: &Path,
    items: &[Item],
    skill_folder: &Path,
    mcp_path: &Path,
) -> HashSet<String> {
    let Ok(records) = read_all(root) else {
        return HashSet::new();
    };
    items
        .iter()
        .filter(|item| {
            records.iter().any(|receipt| {
                if receipt.item_id != item.id {
                    return false;
                }
                match &item.payload {
                    Payload::Skill(_) => {
                        receipt.kind == "skill"
                            && receipt
                                .path
                                .parent()
                                .is_some_and(|p| path_key(p) == path_key(skill_folder))
                            && (receipt.path.join("SKILL.md").is_file()
                                || receipt
                                    .path
                                    .join(crate::extensions::management::DISABLED_SKILL)
                                    .is_file())
                    }
                    Payload::Mcp(_) => {
                        receipt.kind == "mcp"
                            && path_key(&receipt.path) == path_key(mcp_path)
                            && contains_mcp(mcp_path, &receipt.name)
                    }
                }
            })
        })
        .map(|item| item.id.clone())
        .collect()
}
fn contains_mcp(path: &Path, name: &str) -> bool {
    let Ok(Some(text)) = crate::extensions::edit::read(path) else {
        return false;
    };
    if path.extension().is_some_and(|ext| ext == "toml") {
        toml::from_str::<toml::Value>(&text)
            .ok()
            .is_some_and(|v| v.get("mcp_servers").and_then(|v| v.get(name)).is_some())
    } else {
        crate::extensions::jsonc(&text).is_some_and(|v| {
            ["mcp", "mcpServers"]
                .iter()
                .any(|key| v.get(key).and_then(|v| v.get(name)).is_some())
        })
    }
}

/// Write the manual-verification marker before the config. A crash can leave a harmless
/// unused marker, but can never cause a freshly added program to auto-start on rescan.
pub fn save_mcp(
    item: &Item,
    path: &Path,
    name: &str,
    original: Option<&str>,
    preview: &str,
    root: &Path,
) -> Result<()> {
    ensure!(
        matches!(item.payload, Payload::Mcp(_)),
        text("invalid_source")
    );
    write(root, &Receipt::mcp(item, path, name))?;
    crate::extensions::edit::save(path, original, preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::Provider,
        extensions::{Entry, Kind},
    };
    #[test]
    fn receipt_precedes_config_and_remains_manual_across_restart_without_secrets() {
        let root = std::env::temp_dir().join(format!(
            "conduit-receipt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("config.json");
        let ledger = root.join("ledger");
        let item = Item {
            id: "mcp:io.example/test".into(),
            name: "io.example/test".into(),
            description: String::new(),
            source_url: "https://example.com".into(),
            payload: Payload::Mcp(serde_json::json!({"version":"1.0.0"})),
        };
        let preview = crate::extensions::mcp::add_preview(
            Provider::Claude,
            None,
            "test",
            false,
            "never-execute",
            "secret-arg",
        )
        .unwrap();
        save_mcp(&item, &path, "test", None, &preview, &ledger).unwrap();
        let records = read_all(&ledger).unwrap();
        assert!(
            !serde_json::to_string(&records)
                .unwrap()
                .contains("secret-arg")
        );
        let entry = Entry {
            name: "test".into(),
            description: String::new(),
            path: path.clone(),
            scope: "global".into(),
            kind: Kind::Mcp,
            disabled: false,
            transport: "stdio".into(),
            target: "never-execute".into(),
        };
        assert!(manual_from(std::slice::from_ref(&entry), Ok(&records)).contains(&entry.id()));
        assert!(
            manual_from(
                std::slice::from_ref(&entry),
                Err(&anyhow::anyhow!("bad receipt"))
            )
            .contains(&entry.id())
        );
        assert!(save_mcp(&item, &path, "test", None, "{}", &ledger).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), preview);
        assert!(
            installed(&ledger, &[item], &root.join("skills"), &path)
                .contains("mcp:io.example/test")
        );
        fs::remove_dir_all(root).unwrap();
    }
}
