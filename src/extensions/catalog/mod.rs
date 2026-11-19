//! Bounded remote discovery. Fetching metadata never executes an extension.
mod github;
mod http;
pub mod mcp;
pub mod receipts;
mod skills_sh;
mod starters;
#[cfg(test)]
mod tests;

use anyhow::{Result, ensure};
pub use github::{PreparedSkill, install_skill, prepare_skill};
pub use http::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    sync::atomic::AtomicBool,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn text(key: &str) -> String {
    conduit::i18n::text(&format!("catalog.{key}"))
}
pub fn is_github_reference(query: &str) -> bool {
    // Plain keywords belong to the selected search catalog, not GitHub.
    query.contains('/') && github::source(query).is_ok()
}
fn fail(key: &str) -> anyhow::Error {
    anyhow::anyhow!(text(key))
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Featured,
    SkillsSh,
    Github,
    McpRegistry,
}
impl Source {
    pub fn label(self) -> String {
        match self {
            Self::Featured => text("featured"),
            Self::SkillsSh => "skills.sh".into(),
            Self::Github => "GitHub".into(),
            Self::McpRegistry => text("mcp_registry"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub repository: String,
    pub revision: String,
    pub path: Option<String>,
    pub skill_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Payload {
    Skill(Skill),
    Mcp(serde_json::Value),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_url: String,
    pub payload: Payload,
}
impl Item {
    pub fn version(&self) -> &str {
        match &self.payload {
            Payload::Skill(skill) => &skill.revision,
            Payload::Mcp(server) => server["version"].as_str().unwrap_or(""),
        }
    }
}
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: Option<String>,
    pub fetched_at: u64,
    #[serde(skip)]
    pub cached: bool,
    #[serde(skip)]
    pub warning: Option<String>,
}

/// Cache is keyed by source/query/cursor, so partial pages and source failures stay isolated.
pub fn search(
    source: Source,
    query: &str,
    cursor: Option<&str>,
    root: &Path,
    refresh: bool,
    writable: bool,
    cancel: &AtomicBool,
) -> Result<Page> {
    let client = Client::new(cancel);
    search_with(&client, source, query, cursor, root, refresh, writable)
}

fn search_with(
    client: &impl http::Transport,
    source: Source,
    query: &str,
    cursor: Option<&str>,
    root: &Path,
    refresh: bool,
    writable: bool,
) -> Result<Page> {
    client.check()?;
    let query = query.trim();
    ensure!(
        query.len() <= 200 && cursor.is_none_or(|c| c.len() <= 1024),
        text("query_limit")
    );
    let version = if source == Source::Featured {
        "v2"
    } else {
        "v1"
    };
    let key = digest(format!("{version}:{source:?}:{query}:{cursor:?}").as_bytes());
    let path = root.join("extension-catalog").join(format!("{key}.json"));
    let cached = fs::File::open(&path)
        .ok()
        .and_then(|file| {
            use std::io::Read;
            let mut data = Vec::new();
            file.take(2 * 1024 * 1024 + 1).read_to_end(&mut data).ok()?;
            (data.len() <= 2 * 1024 * 1024)
                .then(|| serde_json::from_slice::<Page>(&data).ok())
                .flatten()
        })
        .filter(|p| p.items.len() <= 100);
    if !refresh
        && let Some(mut page) = cached
            .clone()
            .filter(|p| now().saturating_sub(p.fetched_at) < 900)
    {
        page.cached = true;
        return Ok(page);
    }
    let result = fetch(client, source, query, cursor);
    client.check()?;
    match result {
        Ok(mut page) => {
            page.fetched_at = now();
            if writable {
                let _ = write_cache(&path, &page);
            }
            Ok(page)
        }
        Err(error) => match cached {
            Some(mut page) => {
                page.cached = true;
                page.warning = Some(error.to_string());
                Ok(page)
            }
            None if source == Source::SkillsSh && query.is_empty() && cursor.is_none() => {
                Ok(Page {
                    warning: Some(text("trending_fallback")),
                    ..starters::github("")
                })
            }
            None => Err(error),
        },
    }
}
fn write_cache(path: &Path, page: &Page) -> Result<()> {
    let parent = path.parent().ok_or_else(|| fail("invalid_path"))?;
    fs::create_dir_all(parent)?;
    let mut old: Vec<_> = fs::read_dir(parent)?
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.len() == 69
                && name.ends_with(".json")
                && name[..64].bytes().all(|b| b.is_ascii_hexdigit())
        })
        .collect();
    old.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());
    let evict = old.len().saturating_sub(63);
    for entry in old.into_iter().take(evict) {
        let _ = fs::remove_file(entry.path().with_extension("json.conduit-backup"));
        let _ = fs::remove_file(entry.path());
    }
    let data = serde_json::to_string(page)?;
    ensure!(data.len() <= 2 * 1024 * 1024, text("response_limit"));
    let original = crate::extensions::edit::read(path)?;
    crate::extensions::edit::save(path, original.as_deref(), &data)?;
    let _ = fs::remove_file(path.with_extension("json.conduit-backup"));
    Ok(())
}
fn fetch(
    client: &impl http::Transport,
    source: Source,
    query: &str,
    cursor: Option<&str>,
) -> Result<Page> {
    match source {
        Source::Featured => {
            let mut items = vec![
                skill_item(
                    "vercel-labs/agent-skills",
                    "vercel-react-best-practices",
                    Some("skills/react-best-practices"),
                    "React / Next.js",
                ),
                skill_item(
                    "vercel-labs/agent-skills",
                    "web-design-guidelines",
                    Some("skills/web-design-guidelines"),
                    "Web UI / accessibility",
                ),
                skill_item("anthropics/skills", "pdf", Some("skills/pdf"), "PDF"),
                skill_item(
                    "anthropics/skills",
                    "skill-creator",
                    Some("skills/skill-creator"),
                    "Agent Skills",
                ),
                skill_item(
                    "anthropics/skills",
                    "docx",
                    Some("skills/docx"),
                    "Microsoft Word / DOCX",
                ),
                skill_item(
                    "anthropics/skills",
                    "xlsx",
                    Some("skills/xlsx"),
                    "Excel / spreadsheets",
                ),
                skill_item(
                    "anthropics/skills",
                    "pptx",
                    Some("skills/pptx"),
                    "PowerPoint / presentations",
                ),
                skill_item(
                    "obra/superpowers",
                    "systematic-debugging",
                    Some("skills/systematic-debugging"),
                    "Debugging workflow",
                ),
            ];
            let query = query.to_lowercase();
            items.retain(|i| {
                format!("{} {} {}", i.name, i.description, i.id)
                    .to_lowercase()
                    .contains(&query)
            });
            Ok(Page {
                items,
                ..Default::default()
            })
        }
        Source::SkillsSh if query.is_empty() => skills_sh::trending(client),
        Source::SkillsSh => skills_sh::search(client, query),
        Source::Github if cursor.is_none() && !query.contains(['/', '@', '#', ':']) => {
            Ok(starters::github(query))
        }
        Source::Github => github::discover(client, query, cursor),
        Source::McpRegistry => mcp::search(client, query, cursor),
    }
}
fn skill_item(repository: &str, name: &str, path: Option<&str>, description: &str) -> Item {
    Item {
        id: format!("github:{repository}:{name}"),
        name: name.into(),
        description: description.into(),
        source_url: format!("https://github.com/{repository}"),
        payload: Payload::Skill(Skill {
            repository: repository.into(),
            revision: "HEAD".into(),
            path: path.map(str::to_owned),
            skill_id: Some(name.into()),
        }),
    }
}
