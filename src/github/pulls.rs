//! GitHub pull request lookups over GraphQL, shaped like `gh pr view/list --json` output.
use super::*;
use anyhow::ensure;

const FIELDS: &str = r#"number title url headRefName baseRefName headRefOid state reviewDecision
  headRepository { nameWithOwner }
  commits(last:1) { nodes { commit { statusCheckRollup { contexts(first:100) {
    totalCount
    nodes {
      __typename
      ... on CheckRun { name status conclusion detailsUrl }
      ... on StatusContext { context state targetUrl }
    }
  } } } } }
  comments(last:100) { totalCount nodes { author { login } body url } }"#;

fn by_number() -> String {
    format!(
        "query($owner:String!,$name:String!,$number:Int!) {{
  repository(owner:$owner,name:$name) {{ pullRequest(number:$number) {{ {FIELDS} }} }}
}}"
    )
}

fn by_branch() -> String {
    format!(
        "query($owner:String!,$name:String!,$branch:String!) {{
  repository(owner:$owner,name:$name) {{
    parent {{ nameWithOwner }}
    pullRequests(headRefName:$branch,first:30,states:[OPEN,CLOSED,MERGED],
      orderBy:{{field:CREATED_AT,direction:DESC}}) {{ nodes {{ {FIELDS} }} }}
  }}
}}"
    )
}

const LIST: &str = r#"query($owner:String!,$name:String!) {
  repository(owner:$owner,name:$name) {
    pullRequests(first:100,states:[OPEN,CLOSED,MERGED],orderBy:{field:CREATED_AT,direction:DESC}) {
      nodes {
        number title url headRefName state createdAt author { login }
        assignees(first:20) { nodes { login } }
        # Team reviewers need read:org, which Git Credential Manager tokens do not grant.
        reviewRequests(first:20) { nodes { requestedReviewer {
          ... on User { login } ... on Bot { login } ... on Mannequin { login }
        } } }
        reviews(first:50) { nodes { author { login } } }
        labels(first:20) { nodes { name } }
        comments { totalCount }
      }
    }
  }
}"#;

fn owner_name(project: &str) -> Result<(&str, &str)> {
    let (owner, name) = project
        .split_once('/')
        .context("Invalid GitHub repository")?;
    ensure!(!name.contains('/'), "Invalid GitHub repository");
    Ok((owner, name))
}

fn nodes(value: &Value) -> Vec<Value> {
    value["nodes"].as_array().cloned().unwrap_or_default()
}

/// Convert one GraphQL pull request node into the shape [`parse`] reads, plus truncation.
pub(super) fn reshape(node: &Value) -> (Value, bool) {
    let contexts = &node["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"];
    let checks = nodes(contexts);
    let comments = nodes(&node["comments"]);
    let limited = contexts["totalCount"].as_u64().unwrap_or(0) > checks.len() as u64
        || node["comments"]["totalCount"].as_u64().unwrap_or(0) > comments.len() as u64;
    let mut value = node.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("commits");
        object.insert("statusCheckRollup".into(), Value::Array(checks));
        object.insert("comments".into(), Value::Array(comments));
    }
    (value, limited)
}

/// Pick the PR whose head is `head_repo:branch`, preferring an open one, newest first.
pub(super) fn select(candidates: &[Value], head_repo: &str, branch: &str) -> Option<Value> {
    let matching: Vec<_> = candidates
        .iter()
        .filter(|pr| {
            pr["headRefName"].as_str() == Some(branch)
                && pr["headRepository"]["nameWithOwner"]
                    .as_str()
                    .is_some_and(|repo| repo.eq_ignore_ascii_case(head_repo))
        })
        .collect();
    matching
        .iter()
        .find(|pr| pr["state"] == "OPEN")
        .or_else(|| matching.first())
        .map(|pr| (*pr).clone())
}

fn finish(node: &Value) -> Result<PullRequest> {
    let (value, limited) = reshape(node);
    let mut pr = parse(&value.to_string())?;
    pr.limited |= limited;
    Ok(pr)
}

/// Load the PR for `branch` (searching the fork parent too), or PR `number` in the target.
pub(super) fn load(
    client: &mut api::Client,
    target: &forge::Target,
    branch: &str,
    number: Option<u64>,
) -> Result<PullRequest> {
    let (owner, name) = owner_name(&target.project)?;
    if let Some(number) = number {
        let data = client.graphql_data(
            &by_number(),
            serde_json::json!({ "owner": owner, "name": name, "number": number }),
        )?;
        let node = &data["repository"]["pullRequest"];
        ensure!(node.is_object(), "{}", conduit::i18n::text("github.no_pr"));
        return finish(node);
    }
    let query = by_branch();
    let data = client.graphql_data(
        &query,
        serde_json::json!({ "owner": owner, "name": name, "branch": branch }),
    )?;
    let repository = &data["repository"];
    ensure!(
        repository.is_object(),
        "{}",
        conduit::i18n::text("github.no_pr")
    );
    if let Some(pr) = select(&nodes(&repository["pullRequests"]), &target.project, branch) {
        return finish(&pr);
    }
    // A branch pushed to a fork usually has its PR in the parent repository.
    if let Some(parent) = repository["parent"]["nameWithOwner"].as_str() {
        let (owner, name) = owner_name(parent)?;
        let data = client.graphql_data(
            &query,
            serde_json::json!({ "owner": owner, "name": name, "branch": branch }),
        )?;
        if let Some(pr) = select(
            &nodes(&data["repository"]["pullRequests"]),
            &target.project,
            branch,
        ) {
            return finish(&pr);
        }
    }
    bail!(conduit::i18n::text("github.no_pr"))
}

/// The newest 100 PRs, shaped like `gh pr list --json` for [`list::parse`].
pub(super) fn list(client: &mut api::Client, target: &forge::Target) -> Result<Value> {
    let (owner, name) = owner_name(&target.project)?;
    let data = client.graphql_data(LIST, serde_json::json!({ "owner": owner, "name": name }))?;
    let items = nodes(&data["repository"]["pullRequests"])
        .into_iter()
        .map(|mut pr| {
            let assignees = nodes(&pr["assignees"]);
            let requests: Vec<_> = nodes(&pr["reviewRequests"])
                .into_iter()
                .map(|request| request["requestedReviewer"].clone())
                .collect();
            let reviews = nodes(&pr["reviews"]);
            let labels = nodes(&pr["labels"]);
            pr["assignees"] = Value::Array(assignees);
            pr["reviewRequests"] = Value::Array(requests);
            pr["reviews"] = Value::Array(reviews);
            pr["labels"] = Value::Array(labels);
            pr
        })
        .collect();
    Ok(Value::Array(items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(number: u64, state: &str, repo: &str) -> Value {
        json!({
            "number": number, "state": state, "headRefName": "topic",
            "url": format!("https://github.com/up/repo/pull/{number}"),
            "headRefOid": "a".repeat(40), "headRepository": {"nameWithOwner": repo},
            "commits": {"nodes": [{"commit": {"statusCheckRollup": {"contexts": {
                "totalCount": 3,
                "nodes": [{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://ci.example/1"},
                          {"__typename":"StatusContext","context":"legacy","state":"PENDING","targetUrl":null}]
            }}}}]},
            "comments": {"totalCount": 1, "nodes": [{"author":{"login":"bob"},"body":"hi","url":"https://github.com/up/repo/pull/1#c"}]}
        })
    }

    #[test]
    fn graphql_nodes_parse_like_gh_json_and_report_truncation() {
        let pr = finish(&node(7, "OPEN", "me/repo")).unwrap();
        assert_eq!(pr.number, 7);
        assert_eq!(pr.checks.len(), 2);
        assert_eq!(pr.checks[0].state, "SUCCESS");
        assert_eq!(pr.checks[1].state, "PENDING");
        assert_eq!(pr.comments[0].author, "bob");
        assert!(pr.limited, "3 contexts reported, 2 returned");
        let mut no_rollup = node(8, "OPEN", "me/repo");
        no_rollup["commits"]["nodes"][0]["commit"]["statusCheckRollup"] = Value::Null;
        assert!(finish(&no_rollup).unwrap().checks.is_empty());
    }

    #[test]
    fn branch_selection_requires_the_pushed_head_repository_and_prefers_open() {
        let candidates = [
            node(9, "CLOSED", "me/repo"),
            node(8, "OPEN", "other/repo"),
            node(7, "OPEN", "Me/Repo"),
        ];
        assert_eq!(
            select(&candidates, "me/repo", "topic").unwrap()["number"],
            7
        );
        assert_eq!(
            select(&candidates[..2], "me/repo", "topic").unwrap()["number"],
            9
        );
        assert!(select(&candidates, "me/repo", "other").is_none());
        assert!(owner_name("a/b/c").is_err());
    }
}
