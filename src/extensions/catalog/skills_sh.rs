use super::{Item, Page, fail, github, http::Transport, skill_item, text};
use anyhow::{Result, ensure};
use regex::Regex;
use std::{collections::HashSet, sync::LazyLock};

const TRENDING: &str = "https://www.skills.sh/trending";
const MAX_ITEMS: usize = 30;

fn item(repo: &str, slug: &str) -> Option<Item> {
    if github::repository_name(repo).is_err() || slug.is_empty() || slug.len() > 160 {
        return None;
    }
    Some(skill_item(repo, slug, None, "skills.sh"))
}

pub(super) fn search(client: &impl Transport, query: &str) -> Result<Page> {
    ensure!(query.chars().take(2).count() == 2, text("skills_query_min"));
    let mut url = url::Url::parse("https://skills.sh/api/search")?;
    url.query_pairs_mut()
        .append_pair("q", query)
        .append_pair("limit", &MAX_ITEMS.to_string());
    let data = client.json(url.as_str(), 1024 * 1024)?;
    let entries = data["skills"]
        .as_array()
        .ok_or_else(|| fail("invalid_response"))?;
    ensure!(entries.len() <= 100, text("response_limit"));
    let items = entries
        .iter()
        .filter_map(|entry| item(entry["source"].as_str()?, entry["skillId"].as_str()?))
        .collect();
    Ok(Page {
        items,
        ..Page::default()
    })
}

pub(super) fn trending(client: &impl Transport) -> Result<Page> {
    // The anonymous search endpoint requires a query. Read only skill links from the
    // public leaderboard, never its scripts or arbitrary link destinations.
    static LINKS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"<a\b[^>]*?\shref\s*=\s*["']/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/([A-Za-z0-9_-]+)/?["']"#,
        )
        .unwrap()
    });
    let bytes = client.get(TRENDING, 2 * 1024 * 1024)?;
    let html = std::str::from_utf8(&bytes).map_err(|_| fail("invalid_response"))?;
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for link in LINKS.captures_iter(html) {
        client.check()?;
        if let Some(item) = item(&link[1], &link[2])
            && seen.insert(item.id.clone())
        {
            items.push(item);
            if items.len() == MAX_ITEMS {
                break;
            }
        }
    }
    ensure!(!items.is_empty(), text("invalid_response"));
    Ok(Page {
        items,
        ..Page::default()
    })
}
