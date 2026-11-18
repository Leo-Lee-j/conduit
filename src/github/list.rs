//! Bounded repository request summaries for the commit sidebar.
use super::*;
use anyhow::ensure;

#[derive(Clone, Debug)]
pub struct Entry {
    pub forge: forge::Forge,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub branch: String,
    pub state: String,
    pub author: String,
    pub created_at: String,
    pub assignees: Vec<String>,
    pub reviewers: Vec<String>,
    pub labels: Vec<String>,
    pub comments: u64,
}

fn people(value: &Value, key: &str) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|person| {
            person[key]
                .as_str()
                .or_else(|| person["name"].as_str())
                .map(str::to_owned)
        })
        .collect()
}

pub(super) fn parse(value: &Value, forge: forge::Forge) -> Result<Vec<Entry>> {
    let items = value.as_array().context("Invalid request list")?;
    ensure!(items.len() <= 100, "Request list exceeds limit");
    items
        .iter()
        .map(|item| {
            let gitlab = forge == forge::Forge::Gitlab;
            let number = item[if gitlab { "iid" } else { "number" }]
                .as_u64()
                .filter(|number| *number > 0)
                .context("Invalid request number")?;
            let url = web_url(
                item[if gitlab { "web_url" } else { "url" }]
                    .as_str()
                    .unwrap_or_default(),
            )
            .context("Invalid request URL")?;
            let state = match item["state"].as_str().unwrap_or_default() {
                "opened" => "OPEN",
                "merged" => "MERGED",
                "closed" => "CLOSED",
                state => state,
            }
            .to_owned();
            let mut reviewers = people(
                &item[if gitlab {
                    "reviewers"
                } else {
                    "reviewRequests"
                }],
                if gitlab { "username" } else { "login" },
            );
            if !gitlab {
                reviewers.extend(
                    item["reviews"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|review| review["author"]["login"].as_str().map(str::to_owned)),
                );
                reviewers.sort();
                reviewers.dedup();
            }
            Ok(Entry {
                forge,
                number,
                url,
                state,
                title: text(item, "title"),
                branch: text(
                    item,
                    if gitlab {
                        "source_branch"
                    } else {
                        "headRefName"
                    },
                ),
                author: text(&item["author"], if gitlab { "username" } else { "login" }),
                created_at: text(item, if gitlab { "created_at" } else { "createdAt" }),
                assignees: people(
                    &item["assignees"],
                    if gitlab { "username" } else { "login" },
                ),
                reviewers,
                labels: item["labels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|label| {
                        if gitlab {
                            label.as_str()
                        } else {
                            label["name"].as_str()
                        }
                        .map(str::to_owned)
                    })
                    .collect(),
                comments: if gitlab {
                    item["user_notes_count"].as_u64().unwrap_or(0)
                } else {
                    item["comments"]["totalCount"].as_u64().unwrap_or_else(|| {
                        item["comments"]
                            .as_array()
                            .map_or(0, |comments| comments.len() as u64)
                    })
                },
            })
        })
        .collect()
}

pub fn load(workspace: &Path, cancel: &AtomicBool) -> Result<Vec<Entry>> {
    let target = forge::target(workspace, cancel)?;
    let mut client = api::Client::new(target.forge, &target.host, workspace, cancel);
    let value = if target.forge == forge::Forge::Gitlab {
        client.get(&format!(
            "projects/{}/merge_requests?scope=all&state=all&order_by=created_at&sort=desc&per_page=100",
            gitlab::encode(&target.project)
        ))?
    } else {
        pulls::list(&mut client, &target)?
    };
    ensure!(
        forge::target(workspace, cancel)? == target,
        "Repository changed during request listing"
    );
    parse(&value, target.forge)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_normalize_forges_and_reject_invalid_links_and_ids() {
        let github = serde_json::json!([{"number":7,"url":"https://github.com/a/b/pull/7","state":"MERGED","author":{"login":"alice"},"reviewRequests":[{"login":"bob"}],"reviews":[{"author":{"login":"carol"}},{"author":{"login":"bob"}}],"labels":[{"name":"bug"}],"comments":[{}]}]);
        let entries = parse(&github, forge::Forge::Github).unwrap();
        assert_eq!(entries[0].reviewers, ["bob", "carol"]);
        assert_eq!(entries[0].labels, ["bug"]);
        assert_eq!(entries[0].comments, 1);
        let mut gitlab = serde_json::json!([{"iid":8,"web_url":"https://gitlab.example/a/b/-/merge_requests/8","state":"opened","author":{"username":"alice"},"reviewers":[{"username":"bob"}],"labels":["bug"],"user_notes_count":3}]);
        let entries = parse(&gitlab, forge::Forge::Gitlab).unwrap();
        assert_eq!(entries[0].state, "OPEN");
        assert_eq!(entries[0].reviewers, ["bob"]);
        assert_eq!(entries[0].comments, 3);
        gitlab[0]["web_url"] = serde_json::json!("javascript:alert(1)");
        assert!(parse(&gitlab, forge::Forge::Gitlab).is_err());
        gitlab[0]["web_url"] = serde_json::json!("https://gitlab.example/a/b/-/merge_requests/8");
        gitlab[0]["iid"] = serde_json::json!(0);
        assert!(parse(&gitlab, forge::Forge::Gitlab).is_err());
    }
}
