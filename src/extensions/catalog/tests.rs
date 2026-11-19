use super::*;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::{Mutex, atomic::AtomicBool},
};

pub(super) struct Fake {
    data: HashMap<String, Vec<u8>>,
    requests: Mutex<Vec<String>>,
}
impl Fake {
    pub(super) fn new() -> Self {
        Self {
            data: HashMap::new(),
            requests: Mutex::new(vec![]),
        }
    }
    pub(super) fn add(&mut self, url: &str, data: impl AsRef<[u8]>) {
        self.data.insert(url.into(), data.as_ref().to_vec());
    }
}
impl http::Transport for Fake {
    fn get(&self, url: &str, limit: usize) -> anyhow::Result<Vec<u8>> {
        self.requests.lock().unwrap().push(url.into());
        let data = self
            .data
            .get(url)
            .ok_or_else(|| anyhow::anyhow!("missing fixture: {url}"))?;
        anyhow::ensure!(data.len() <= limit, "limit");
        Ok(data.clone())
    }
}
fn folder() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "conduit-catalog-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
#[test]
fn blank_github_query_lists_public_skills_without_network_and_supports_filtering() {
    let client = Fake::new();
    assert!(
        fetch(&client, Source::Featured, "", None)
            .unwrap()
            .items
            .len()
            > 4
    );
    let page = fetch(&client, Source::Github, "", None).unwrap();
    assert!(page.items.len() >= 10);
    assert!(
        page.items
            .iter()
            .all(|item| matches!(&item.payload, Payload::Skill(skill) if skill.path.is_some()))
    );
    let filtered = fetch(&client, Source::Github, "playwright", None).unwrap();
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].name, "webapp-testing");
    assert!(client.requests.lock().unwrap().is_empty());
}

#[test]
fn blank_skills_sh_query_loads_ranked_links_and_falls_back_when_unavailable() {
    let mut client = Fake::new();
    client.add(
        "https://www.skills.sh/trending",
        r#"
        <link href="/not/a/skill">
        <a href="/example/skills/first">First</a>
        <a class="row" href='/example/skills/first/'>Duplicate</a>
        <a href="https://example.net/other/repo/skill">External</a>
        <a href="/other/repository/second">Second</a>
        <a href="/not/a/skill/child">Not a skill</a>
    "#,
    );
    let root = folder();
    let ranked = search_with(&client, Source::SkillsSh, "  ", None, &root, false, true).unwrap();
    assert_eq!(
        ranked
            .items
            .iter()
            .map(|i| i.name.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert!(ranked.warning.is_none());
    let down = Fake::new();
    let cached = search_with(&down, Source::SkillsSh, "", None, &root, true, false).unwrap();
    assert!(cached.cached && cached.warning.is_some());
    assert_eq!(cached.items[0].name, "first");
    fs::remove_dir_all(&root).unwrap();
    let fallback = search_with(&down, Source::SkillsSh, "", None, &root, false, true).unwrap();
    assert!(!fallback.cached && fallback.warning.is_some());
    assert!(fallback.items.len() >= 10);
    assert!(!root.exists());
    assert!(
        search_with(
            &down,
            Source::SkillsSh,
            "keyword",
            None,
            &root,
            false,
            false
        )
        .is_err()
    );
}
#[test]
fn source_failures_and_mcp_pagination_are_explicit_and_do_not_execute_packages() {
    let mut client = Fake::new();
    client.add("https://registry.modelcontextprotocol.io/v0.1/servers?limit=30&version=latest&search=git", serde_json::to_vec(&json!({
        "servers": [{"server":{"name":"io.example/git","version":"1.0.0","remotes":[{"type":"streamable-http","url":"https://example.com/mcp"}]},"_meta":{"io.modelcontextprotocol.registry/official":{"status":"active"}}},
        {"server":{"name":"io.example/deleted"},"_meta":{"io.modelcontextprotocol.registry/official":{"status":"deleted"}}}],
        "metadata":{"nextCursor":"next:1"}
    })).unwrap());
    let page = fetch(&client, Source::McpRegistry, "git", None).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.next.as_deref(), Some("next:1"));
    assert_eq!(mcp::options(&page.items[0]).len(), 1);
    assert!(fetch(&client, Source::SkillsSh, "missing", None).is_err());
    assert!(
        client
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|u| u.starts_with("https://"))
    );
}
#[test]
fn cancellation_prevents_network_and_stale_cache_has_bounded_fallback() {
    let root = folder();
    let cancel = AtomicBool::new(true);
    assert!(search(Source::Featured, "", None, &root, true, true, &cancel).is_err());
    assert!(!root.exists());
    let cancel = AtomicBool::new(false);
    let page = search(Source::Featured, "react", None, &root, true, true, &cancel).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(
        search(
            Source::Featured,
            "react",
            None,
            &root,
            false,
            false,
            &cancel
        )
        .unwrap()
        .cached
    );
    let invalid_query = "invalid/repository/path";
    let mut expired = page;
    expired.fetched_at = now().saturating_sub(901);
    let key = digest(
        format!(
            "v1:{:?}:{invalid_query}:{:?}",
            Source::Github,
            Option::<&str>::None
        )
        .as_bytes(),
    );
    write_cache(
        &root.join("extension-catalog").join(format!("{key}.json")),
        &expired,
    )
    .unwrap();
    // Invalid source fails before HTTP; every source error uses the same stale-cache fallback.
    let fallback = search(
        Source::Github,
        invalid_query,
        None,
        &root,
        false,
        false,
        &cancel,
    )
    .unwrap();
    assert!(fallback.cached);
    assert!(fallback.warning.is_some());
    assert_eq!(fallback.fetched_at, expired.fetched_at);
    assert_eq!(fallback.items[0].id, expired.items[0].id);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "explicit public catalog connectivity and temporary Skill download; requires network"]
fn public_catalog_connectivity_and_temporary_skill_install() {
    let cancel = AtomicBool::new(false);
    let client = Client::new(&cancel);
    let trending = fetch(&client, Source::SkillsSh, "", None).unwrap();
    assert!(trending.items.len() >= 10);
    let github = fetch(&client, Source::Github, "obra/superpowers", None).unwrap();
    assert!(
        github
            .items
            .iter()
            .any(|item| item.name == "systematic-debugging")
    );
    let skills = fetch(&client, Source::SkillsSh, "react", None).unwrap();
    assert!(!skills.items.is_empty());
    let servers = fetch(&client, Source::McpRegistry, "github", None).unwrap();
    assert!(!servers.items.is_empty());
    assert!(
        servers
            .items
            .iter()
            .any(|item| !mcp::options(item).is_empty())
    );
    let item = skill_item(
        "vercel-labs/agent-skills",
        "web-design-guidelines",
        Some("skills/web-design-guidelines"),
        "",
    );
    let prepared = prepare_skill(&client, &item).unwrap();
    let root = folder();
    let result = install_skill(
        &Client::new(&cancel),
        &prepared,
        &root.join("skills"),
        &root.join("ledger"),
    );
    let verified = result.as_ref().is_ok_and(|path| {
        fs::read_to_string(path.join("SKILL.md")).is_ok_and(|content| content == prepared.markdown)
    });
    if root.exists() {
        fs::remove_dir_all(&root).unwrap();
    }
    result.unwrap();
    assert!(verified);
    eprintln!(
        "public sources: {} trending, {} GitHub, {} search results, {} MCP servers; installed {} files at {}",
        trending.items.len(),
        github.items.len(),
        skills.items.len(),
        servers.items.len(),
        prepared.files.len(),
        prepared.commit
    );
}
