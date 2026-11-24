//! Remote PR/MR description and file patches captured against one verified revision.
use super::*;
use anyhow::ensure;
use std::collections::HashSet;

const PAGE_SIZE: usize = 30;
const MAX_FILES: usize = 1_000;
const MAX_PATCH_BYTES: usize = 512 * 1024;

#[derive(Clone, Debug)]
pub struct File {
    pub path: String,
    pub original: Option<String>,
    pub status: char,
    pub additions: usize,
    pub deletions: usize,
    patch: Option<String>,
    binary: bool,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub pr: String,
    pub head: String,
    pub comparison: String,
    pub description: String,
    pub author: String,
    pub created_at: String,
    pub reviewers: Vec<String>,
    pub commits: Option<u64>,
    pub files: Vec<File>,
    pub limited: bool,
}

fn endpoint(pr: &PullRequest) -> Result<(String, String)> {
    ensure!(threads::oid(&pr.head), "Invalid request revision");
    let (host, endpoint, number) = match pr.forge {
        forge::Forge::Github => {
            let identity = threads::identity(&pr.url)?;
            (
                identity.host,
                format!(
                    "repos/{}/{}/pulls/{}",
                    identity.owner, identity.name, identity.number
                ),
                identity.number,
            )
        }
        forge::Forge::Gitlab => {
            let identity = gitlab::identity(&pr.url)?;
            let url = url::Url::parse(&pr.url)?;
            let host = url.host_str().context("Missing GitLab host")?;
            let host = url
                .port()
                .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));
            (
                host,
                format!(
                    "projects/{}/merge_requests/{}",
                    gitlab::encode(&identity.project),
                    identity.number
                ),
                identity.number,
            )
        }
    };
    ensure!(number == pr.number, "Request identity changed");
    Ok((pr.api_host.clone().unwrap_or(host), endpoint))
}

pub fn load(workspace: &Path, pr: &PullRequest, cancel: &AtomicBool) -> Result<Snapshot> {
    let (host, endpoint) = endpoint(pr)?;
    let mut client = api::Client::new(pr.forge, &host, workspace, cancel);
    capture(pr, &endpoint, &mut |path| client.get(path))
}

fn revision(pr: &PullRequest, value: &Value) -> Result<String> {
    let (number, url, head, base) = match pr.forge {
        forge::Forge::Github => (
            value["number"].as_u64(),
            value["html_url"].as_str(),
            value["head"]["sha"].as_str(),
            value["base"]["sha"].as_str(),
        ),
        forge::Forge::Gitlab => (
            value["iid"].as_u64(),
            value["web_url"].as_str(),
            value["sha"].as_str(),
            value["diff_refs"]["base_sha"].as_str(),
        ),
    };
    ensure!(
        number == Some(pr.number) && url == Some(pr.url.as_str()) && head == Some(pr.head.as_str()),
        "{}",
        conduit::i18n::text("github.detail_revision_changed")
    );
    let base = base
        .filter(|base| threads::oid(base))
        .context("Missing comparison revision")?;
    if pr.forge == forge::Forge::Gitlab {
        ensure!(
            value["diff_refs"]["head_sha"].as_str() == Some(pr.head.as_str()),
            "{}",
            conduit::i18n::text("github.detail_revision_changed")
        );
        let start = value["diff_refs"]["start_sha"]
            .as_str()
            .filter(|oid| threads::oid(oid))
            .context("Missing comparison start revision")?;
        return Ok(format!("{base}:{start}"));
    }
    Ok(base.to_owned())
}

fn capture(
    pr: &PullRequest,
    endpoint: &str,
    fetch: &mut impl FnMut(&str) -> Result<Value>,
) -> Result<Snapshot> {
    let metadata = fetch(endpoint)?;
    let baseline = revision(pr, &metadata)?;
    let suffix = if pr.forge == forge::Forge::Github {
        "files"
    } else {
        "diffs"
    };
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    let mut complete = false;
    let mut overflow = false;
    for page in 1..=MAX_FILES.div_ceil(PAGE_SIZE) {
        let path = format!("{endpoint}/{suffix}?per_page={PAGE_SIZE}&page={page}");
        let mut legacy = false;
        let response = match fetch(&path) {
            Err(error)
                if pr.forge == forge::Forge::Gitlab
                    && page == 1
                    && error
                        .downcast_ref::<api::Status>()
                        .is_some_and(|status| status.code == 404) =>
            {
                // GitLab before 15.7 has no paginated /diffs endpoint. Its /changes
                // response includes both the MR revision and a bounded file list.
                let changes = fetch(&format!("{endpoint}/changes"))?;
                ensure!(
                    revision(pr, &changes)? == baseline,
                    "{}",
                    conduit::i18n::text("github.detail_revision_changed")
                );
                overflow = changes["overflow"].as_bool().unwrap_or(false);
                legacy = true;
                changes["changes"].clone()
            }
            result => result?,
        };
        let values = response.as_array().context("Invalid changed file list")?;
        ensure!(
            legacy || values.len() <= PAGE_SIZE,
            "Invalid changed file page"
        );
        for value in values.iter().take(MAX_FILES - files.len()) {
            let file = parse_file(pr.forge, value)?;
            ensure!(
                paths.insert(file.path.clone()),
                "Changed file list contains duplicate paths"
            );
            files.push(file);
        }
        if legacy {
            complete = values.len() <= MAX_FILES;
            break;
        }
        if values.len() < PAGE_SIZE {
            complete = true;
            break;
        }
        if files.len() == MAX_FILES {
            break;
        }
    }
    ensure!(
        revision(pr, &fetch(endpoint)?)? == baseline,
        "{}",
        conduit::i18n::text("github.detail_revision_changed")
    );
    let github = pr.forge == forge::Forge::Github;
    let author = if github {
        &metadata["user"]["login"]
    } else {
        &metadata["author"]["username"]
    };
    let reviewer_key = if github {
        "requested_reviewers"
    } else {
        "reviewers"
    };
    let reviewers = metadata[reviewer_key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|user| user[if github { "login" } else { "username" }].as_str())
        .take(100)
        .map(str::to_owned)
        .collect();
    let expected_count = if github {
        metadata["changed_files"].as_u64()
    } else {
        metadata["changes_count"]
            .as_str()
            .and_then(|count| count.parse().ok())
    };
    let limited = overflow
        || !complete
        || expected_count.is_some_and(|count| count > files.len() as u64)
        || metadata["changes_count"]
            .as_str()
            .is_some_and(|count| count.ends_with('+'));
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Snapshot {
        pr: pr.url.clone(),
        head: pr.head.clone(),
        comparison: baseline,
        description: metadata[if github { "body" } else { "description" }]
            .as_str()
            .unwrap_or_default()
            .into(),
        author: author.as_str().unwrap_or_default().into(),
        created_at: text(&metadata, "created_at"),
        reviewers,
        commits: metadata["commits"].as_u64(),
        files,
        limited,
    })
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && path.split('/').count() <= 128
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

fn parse_file(forge: forge::Forge, value: &Value) -> Result<File> {
    let github = forge == forge::Forge::Github;
    let path = text(value, if github { "filename" } else { "new_path" });
    ensure!(valid_path(&path), "Invalid changed file path");
    let old = text(
        value,
        if github {
            "previous_filename"
        } else {
            "old_path"
        },
    );
    let original = (!old.is_empty() && old != path).then_some(old);
    ensure!(
        original.as_deref().is_none_or(valid_path),
        "Invalid original file path"
    );
    let status = if github {
        match value["status"].as_str() {
            Some("added") => 'A',
            Some("removed") => 'D',
            Some("renamed") => 'R',
            Some("copied") => 'C',
            _ => 'M',
        }
    } else if value["new_file"] == true {
        'A'
    } else if value["deleted_file"] == true {
        'D'
    } else if value["renamed_file"] == true {
        'R'
    } else {
        'M'
    };
    let raw = value[if github { "patch" } else { "diff" }].as_str();
    let changes = raw
        .unwrap_or_default()
        .lines()
        .fold((0, 0), |(a, d), line| {
            (
                a + usize::from(line.starts_with('+')),
                d + usize::from(line.starts_with('-')),
            )
        });
    let (additions, deletions) = if github {
        (
            value["additions"].as_u64().unwrap_or(0) as usize,
            value["deletions"].as_u64().unwrap_or(0) as usize,
        )
    } else {
        changes
    };
    let patch = raw
        .filter(|patch| patch.len() <= MAX_PATCH_BYTES && complete_hunks(patch))
        // GitLab does not supply line totals; an empty diff can also mean binary content.
        .filter(|patch| github || !patch.is_empty())
        // A patch may be cut at a hunk boundary, so hunk syntax alone is insufficient.
        .filter(|_| !github || changes == (additions, deletions))
        .filter(|_| value["too_large"] != true && value["collapsed"] != true)
        .map(str::to_owned);
    Ok(File {
        path,
        original,
        status,
        additions,
        deletions,
        patch,
        binary: value["binary"] == true,
    })
}

// Do not present an omitted or truncated API hunk as a complete, reviewable patch.
fn complete_hunks(patch: &str) -> bool {
    let mut remaining = None;
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            if remaining.is_some_and(|counts| counts != (0, 0)) {
                return false;
            }
            let Some(header) = line
                .strip_prefix("@@ ")
                .and_then(|s| s.split_once(" @@").map(|(h, _)| h))
            else {
                return false;
            };
            let mut ranges = header.split_whitespace();
            let range = |value: Option<&str>, sign| -> Option<usize> {
                let value = value?.strip_prefix(sign)?;
                let (start, count) = value.split_once(',').unwrap_or((value, "1"));
                start.parse::<usize>().ok()?;
                count.parse().ok()
            };
            let Some(old) = range(ranges.next(), '-') else {
                return false;
            };
            let Some(new) = range(ranges.next(), '+') else {
                return false;
            };
            if ranges.next().is_some() {
                return false;
            }
            remaining = Some((old, new));
        } else if line.starts_with("\\ No newline at end of file") {
            if remaining.is_none() {
                return false;
            }
        } else {
            let Some((old, new)) = remaining.as_mut() else {
                return false;
            };
            let consume = match line.as_bytes().first() {
                Some(b' ') => (1, 1),
                Some(b'-') => (1, 0),
                Some(b'+') => (0, 1),
                _ => return false,
            };
            let Some(old_left) = old.checked_sub(consume.0) else {
                return false;
            };
            let Some(new_left) = new.checked_sub(consume.1) else {
                return false;
            };
            (*old, *new) = (old_left, new_left);
        }
    }
    remaining.is_some_and(|counts| counts == (0, 0)) || patch.is_empty()
}

impl File {
    pub fn changed_file(&self) -> crate::git::ChangedFile {
        crate::git::ChangedFile {
            path: self.path.clone(),
            original: self.original.clone(),
            index: self.status,
            worktree: ' ',
        }
    }

    pub fn diff(&self) -> Result<crate::git::FileDiff> {
        let quote = |prefix: &str, path: &str| format!("\"{prefix}{}\"", path.replace('"', "\\\""));
        let old = self.original.as_deref().unwrap_or(&self.path);
        let before = quote("a/", old);
        let after = quote("b/", &self.path);
        let mut patch = format!("diff --git {before} {after}\n");
        if self.status == 'A' {
            patch.push_str("new file mode 100644\n");
        }
        if self.status == 'D' {
            patch.push_str("deleted file mode 100644\n");
        }
        if self.status == 'R' {
            patch.push_str(&format!(
                "rename from {}\nrename to {}\n",
                quote("", old),
                quote("", &self.path)
            ));
        }
        if self.binary {
            patch.push_str(&format!("Binary files {before} and {after} differ\n"));
        } else {
            let body = self
                .patch
                .as_deref()
                .context(conduit::i18n::text("github.detail_patch_unavailable"))?;
            ensure!(
                !body.is_empty() || (self.additions == 0 && self.deletions == 0),
                "{}",
                conduit::i18n::text("github.detail_patch_unavailable")
            );
            let before = if self.status == 'A' {
                "/dev/null"
            } else {
                &before
            };
            let after = if self.status == 'D' {
                "/dev/null"
            } else {
                &after
            };
            patch.push_str(&format!("--- {before}\n+++ {after}\n{body}"));
            if !patch.ends_with('\n') {
                patch.push('\n');
            }
        }
        Ok(crate::git::FileDiff {
            patch,
            notes: Vec::new(),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(feature = "ui-tests")]
    pub(crate) fn loaded(pr: &PullRequest) -> Snapshot {
        let (_, endpoint) = endpoint(pr).unwrap();
        capture(pr, &endpoint, &mut |path| {
            Ok(if path == endpoint {
                let mut value = metadata(pr);
                let key = if pr.forge == forge::Forge::Github { "number" } else { "iid" };
                value[key] = json!(pr.number);
                value
            } else if pr.forge == forge::Forge::Github {
                json!([{"filename":"src/新 文件.rs","previous_filename":"src/old.rs","status":"renamed","additions":1,"deletions":1,"patch":"@@ -1 +1 @@\n-old remote\n+new remote"}])
            } else {
                json!([{"new_path":"src/新 文件.rs","old_path":"src/old.rs","renamed_file":true,"diff":"@@ -1 +1 @@\n-old remote\n+new remote"}])
            })
        }).unwrap()
    }

    fn request(forge: forge::Forge) -> PullRequest {
        let mut pr = super::super::parse(
            &json!({
                "number":7, "url":"https://github.com/owner/repo/pull/7",
                "headRefOid":"a".repeat(40), "headRefName":"topic"
            })
            .to_string(),
        )
        .unwrap();
        pr.forge = forge;
        if forge == forge::Forge::Gitlab {
            pr.url = "https://gitlab.example/team/repo/-/merge_requests/7".into();
        }
        pr
    }

    fn metadata(pr: &PullRequest) -> Value {
        match pr.forge {
            forge::Forge::Github => json!({
                "number":7,"html_url":pr.url,"head":{"sha":pr.head},"base":{"sha":"b".repeat(40)},
                "body":"Description","user":{"login":"alice"},"changed_files":1,"commits":17
            }),
            forge::Forge::Gitlab => json!({
                "iid":7,"web_url":pr.url,"sha":pr.head,
                "diff_refs":{"base_sha":"b".repeat(40),"start_sha":"c".repeat(40),"head_sha":pr.head},
                "description":"Description","author":{"username":"alice"},"changes_count":"1"
            }),
        }
    }

    #[test]
    fn remote_files_preserve_renames_unicode_and_patch_sides_for_both_forges() {
        for forge in [forge::Forge::Github, forge::Forge::Gitlab] {
            let pr = request(forge);
            let (_, endpoint) = endpoint(&pr).unwrap();
            let value = match forge {
                forge::Forge::Github => {
                    json!({"filename":"src/新 文件.rs","previous_filename":"src/old.rs","status":"renamed","additions":1,"deletions":1,"patch":"@@ -1 +1 @@\n-old\n+new"})
                }
                forge::Forge::Gitlab => {
                    json!({"new_path":"src/新 文件.rs","old_path":"src/old.rs","renamed_file":true,"diff":"@@ -1 +1 @@\n-old\n+new"})
                }
            };
            let snapshot = capture(&pr, &endpoint, &mut |path| {
                Ok(if path == endpoint {
                    metadata(&pr)
                } else {
                    json!([value])
                })
            })
            .unwrap();
            assert_eq!(snapshot.author, "alice");
            assert_eq!(snapshot.description, "Description");
            assert!(!snapshot.limited);
            let file = &snapshot.files[0];
            assert_eq!(file.original.as_deref(), Some("src/old.rs"));
            assert_eq!((file.additions, file.deletions), (1, 1));
            let patch = file.diff().unwrap().patch;
            let parsed = gpui_kit::component::diff::DiffFile::parse(&patch).unwrap();
            assert_eq!(parsed.len(), 1);
            assert_eq!(parsed[0].additions(), 1);
            assert_eq!(parsed[0].deletions(), 1);
        }
    }

    #[test]
    fn old_gitlab_diffs_404_falls_back_to_changes_on_the_repository_host() {
        let mut pr = request(forge::Forge::Gitlab);
        pr.url = "https://git.example.test:8443/team/subgroup/repo/-/merge_requests/7".into();
        pr.api_host = Some("git.example.test:8443".into());
        let (host, endpoint) = endpoint(&pr).unwrap();
        assert_eq!(host, "git.example.test:8443");
        assert_eq!(endpoint, "projects/team%2Fsubgroup%2Frepo/merge_requests/7");
        let mut reads = Vec::new();
        let snapshot = capture(&pr, &endpoint, &mut |path| {
            reads.push(path.to_owned());
            if path == endpoint {
                return Ok(metadata(&pr));
            }
            if path.ends_with("/diffs?per_page=30&page=1") {
                return Err(api::Status {
                    code: 404,
                    message: "404 Not Found".into(),
                }
                .into());
            }
            assert_eq!(path, format!("{endpoint}/changes"));
            let mut value = metadata(&pr);
            value["overflow"] = json!(false);
            value["changes"] = json!([{
                "old_path":"src/main.rs","new_path":"src/main.rs",
                "diff":"@@ -1 +1 @@\n-old\n+new"
            }]);
            Ok(value)
        })
        .unwrap();
        assert_eq!(reads.len(), 4);
        assert_eq!(reads.first(), reads.last());
        assert_eq!(snapshot.files.len(), 1);
        assert!(snapshot.files[0].diff().unwrap().patch.contains("+new"));
        assert!(!snapshot.limited);
    }

    #[test]
    fn legacy_gitlab_changes_preserve_overflow_limits_and_revision_checks() {
        let pr = request(forge::Forge::Gitlab);
        let (_, endpoint) = endpoint(&pr).unwrap();
        for (count, overflow, changed) in [(1, true, false), (1001, false, false), (1, false, true)]
        {
            let result =
                capture(&pr, &endpoint, &mut |path| {
                    if path == endpoint {
                        return Ok(metadata(&pr));
                    }
                    if path.contains("/diffs?") {
                        return Err(api::Status {
                            code: 404,
                            message: "Not Found".into(),
                        }
                        .into());
                    }
                    let mut value = metadata(&pr);
                    value["overflow"] = json!(overflow);
                    value["changes"] =
                        json!((0..count).map(|i| json!({
                    "new_path":format!("src/{i}.rs"), "old_path":format!("src/{i}.rs"),
                    "diff":"@@ -1 +1 @@\n-old\n+new"
                })).collect::<Vec<_>>());
                    if changed {
                        value["diff_refs"]["start_sha"] = json!("d".repeat(40));
                    }
                    Ok(value)
                });
            if changed {
                assert!(result.is_err());
            } else {
                let snapshot = result.unwrap();
                assert!(snapshot.limited);
                assert_eq!(snapshot.files.len(), count.min(MAX_FILES));
            }
        }
    }

    #[test]
    fn fallback_does_not_hide_metadata_errors_or_other_http_failures() {
        for forge in [forge::Forge::Gitlab, forge::Forge::Github] {
            let pr = request(forge);
            let (_, endpoint) = endpoint(&pr).unwrap();
            for (code, metadata_error) in [(404, true), (403, false), (500, false)] {
                let mut reads = 0;
                let error = capture(&pr, &endpoint, &mut |path| {
                    reads += 1;
                    assert!(!path.ends_with("/changes"));
                    if path == endpoint && !metadata_error {
                        return Ok(metadata(&pr));
                    }
                    Err(api::Status {
                        code,
                        message: "Rejected".into(),
                    }
                    .into())
                })
                .unwrap_err();
                assert_eq!(error.downcast_ref::<api::Status>().unwrap().code, code);
                assert_eq!(reads, if metadata_error { 1 } else { 2 });
            }
        }
    }

    #[test]
    fn capture_rejects_head_base_and_repository_changes_during_pagination() {
        for forge in [forge::Forge::Github, forge::Forge::Gitlab] {
            let pr = request(forge);
            let (_, endpoint) = endpoint(&pr).unwrap();
            for change in ["head", "base", "url"] {
                let mut reads = 0;
                let result = capture(&pr, &endpoint, &mut |path| {
                    if path != endpoint {
                        return Ok(json!([]));
                    }
                    reads += 1;
                    let mut value = metadata(&pr);
                    if reads == 2 {
                        match (forge, change) {
                            (forge::Forge::Github, "head") => {
                                value["head"]["sha"] = json!("d".repeat(40))
                            }
                            (forge::Forge::Github, "base") => {
                                value["base"]["sha"] = json!("d".repeat(40))
                            }
                            (forge::Forge::Github, _) => {
                                value["html_url"] = json!("https://github.com/other/repo/pull/7")
                            }
                            (forge::Forge::Gitlab, "head") => value["sha"] = json!("d".repeat(40)),
                            (forge::Forge::Gitlab, "base") => {
                                value["diff_refs"]["start_sha"] = json!("d".repeat(40))
                            }
                            (forge::Forge::Gitlab, _) => {
                                value["web_url"] =
                                    json!("https://gitlab.example/other/repo/-/merge_requests/7")
                            }
                        }
                    }
                    Ok(value)
                });
                assert!(result.is_err(), "{forge:?} {change}");
            }
        }
    }

    #[test]
    fn incomplete_missing_and_collapsed_patches_are_not_presented_as_empty_changes() {
        for value in [
            json!({"filename":"a.rs","patch":"@@ -1 +1,2 @@\n-old\n+new"}),
            json!({"filename":"a.rs","additions":1}),
            json!({"filename":"a.rs","patch":"","additions":1}),
            json!({"filename":"a.rs","patch":"@@ -1 +1 @@\n-old\n+new","additions":1,"deletions":1,"collapsed":true}),
            json!({"filename":"a.rs","patch":"@@ -1 +1 @@\n-old\n+new","additions":2,"deletions":2}),
        ] {
            assert!(
                parse_file(forge::Forge::Github, &value)
                    .unwrap()
                    .diff()
                    .is_err()
            );
        }
        assert!(
            parse_file(
                forge::Forge::Gitlab,
                &json!({"new_path":"image.png","old_path":"image.png","diff":""})
            )
            .unwrap()
            .diff()
            .is_err()
        );
        let binary = parse_file(
            forge::Forge::Gitlab,
            &json!({"new_path":"image.png","old_path":"image.png","diff":"","binary":true}),
        )
        .unwrap();
        assert!(binary.diff().unwrap().patch.contains("Binary files"));
        assert!(parse_file(forge::Forge::Github, &json!({"filename":"../escape"})).is_err());
        assert!(
            parse_file(
                forge::Forge::Gitlab,
                &json!({"new_path":"/absolute","old_path":"old"})
            )
            .is_err()
        );
    }
}
