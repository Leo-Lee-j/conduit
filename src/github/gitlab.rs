//! Self-managed GitLab REST snapshots through the user's authenticated glab CLI.
use super::*;
use anyhow::ensure;
use forge::{Forge, Target};
use std::collections::HashSet;
use threads::{Author, CommentState, Commit, DiffSide, Snapshot, Subject, Thread};

const PAGE_SIZE: usize = 25;
const MAX_PAGES: usize = 8;

pub(super) fn executable() -> Result<PathBuf> {
    let mut candidates: Vec<_> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("glab.exe"))
                .collect()
        })
        .unwrap_or_default();
    if let Some(root) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(root).join("glab/glab.exe"));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .context(conduit::i18n::text("gitlab.missing_cli"))
}

pub(super) fn encode(value: &str) -> String {
    // Encode a project path as one API path component, including subgroup separators.
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn positive(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .filter(|n| *n > 0)
        .with_context(|| format!("Invalid GitLab {key}"))
}

fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) struct Identity {
    pub project: String,
    pub number: u64,
}

pub(super) fn identity(value: &str) -> Result<Identity> {
    let url = url::Url::parse(value)?;
    ensure!(
        web_url(value).is_some() && url.query().is_none() && url.fragment().is_none(),
        "Invalid MR URL"
    );
    let (project, number) = url
        .path()
        .trim_start_matches('/')
        .rsplit_once("/-/merge_requests/")
        .context("Invalid MR path")?;
    let host = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let parsed = forge::parse_remote(&format!("{}://{host}/{project}", url.scheme()))?;
    let number: u64 = number.parse()?;
    ensure!(number > 0, "Invalid MR number");
    Ok(Identity {
        project: parsed.project,
        number,
    })
}

fn api_args(host: &str, endpoint: &str, method: &str) -> Vec<String> {
    vec![
        "api".into(),
        endpoint.into(),
        "--hostname".into(),
        host.into(),
        "--method".into(),
        method.into(),
    ]
}

struct Api<'a> {
    cli: PathBuf,
    workspace: &'a Path,
    target: &'a Target,
    cancel: &'a AtomicBool,
    started: Instant,
    bytes: usize,
}

impl Api<'_> {
    fn get(&mut self, endpoint: &str) -> Result<Value> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(90),
            "{}",
            conduit::i18n::text("github.timeout")
        );
        let args = api_args(&self.target.host, endpoint, "GET");
        let json = query(
            &self.cli,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            self.workspace,
            self.cancel,
        )?;
        self.bytes = self.bytes.saturating_add(json.len());
        ensure!(
            self.bytes <= 8 * 1024 * 1024,
            "{}",
            conduit::i18n::text("github.output_limit")
        );
        Ok(serde_json::from_str(&json)?)
    }
}

fn pages(
    endpoint: &str,
    max_pages: usize,
    fetch: &mut impl FnMut(&str) -> Result<Value>,
) -> Result<(Vec<Value>, bool)> {
    let mut values = Vec::new();
    let mut ids = HashSet::new();
    for page in 1..=max_pages {
        let join = if endpoint.contains('?') { '&' } else { '?' };
        let response = fetch(&format!("{endpoint}{join}per_page={PAGE_SIZE}&page={page}"))?;
        let items = response.as_array().context("Invalid GitLab list")?;
        ensure!(items.len() <= PAGE_SIZE, "Invalid GitLab page size");
        for item in items {
            let id = item
                .get("id")
                .context("Missing GitLab list identity")?
                .to_string();
            ensure!(ids.insert(id), "GitLab list changed during pagination");
            values.push(item.clone());
        }
        if items.len() < PAGE_SIZE {
            return Ok((values, false));
        }
    }
    Ok((values, true))
}

fn same_site(value: &str, root: &str) -> Option<String> {
    let value = web_url(value)?;
    (url::Url::parse(&value).ok()?.origin() == url::Url::parse(root).ok()?.origin())
        .then_some(value)
}

fn parse_mr(value: &Value, project_url: &str, branch: &str, source: u64) -> Result<PullRequest> {
    let number = positive(value, "iid")?;
    let url = text(value, "web_url");
    ensure!(
        url == format!(
            "{}/-/merge_requests/{number}",
            project_url.trim_end_matches('/')
        ),
        "MR repository does not match selected project"
    );
    let repository = identity(&url)?;
    ensure!(
        repository.number == number
            && text(value, "source_branch") == branch
            && positive(value, "source_project_id")? == source,
        "MR source does not match current branch"
    );
    let head = text(value, "sha");
    ensure!(threads::oid(&head), "Invalid MR head");
    Ok(PullRequest {
        forge: Forge::Gitlab,
        cli_host: None,
        number,
        title: text(value, "title"),
        url,
        head,
        branch: branch.into(),
        base: text(value, "target_branch"),
        state: match value["state"].as_str() {
            Some("opened") => "OPEN".into(),
            Some("merged") => "MERGED".into(),
            Some("closed") => "CLOSED".into(),
            _ => text(value, "state"),
        },
        decision: String::new(),
        checks: vec![],
        checks_error: None,
        comments: vec![],
        limited: false,
        local_head: String::new(),
        fetched_at: 0,
        threads: None,
        threads_error: None,
    })
}

fn check_state(value: &str) -> String {
    match value {
        "success" => "SUCCESS",
        "failed" => "FAILURE",
        "canceled" => "CANCELLED",
        "skipped" => "SKIPPED",
        "created" | "waiting_for_resource" | "preparing" | "pending" | "scheduled" => "PENDING",
        "running" => "IN_PROGRESS",
        "manual" => "ACTION_REQUIRED",
        _ => value,
    }
    .into()
}

fn checks(
    value: &Value,
    pr: &mut PullRequest,
    fetch: &mut impl FnMut(&str) -> Result<Value>,
) -> Result<()> {
    let pipeline = &value["head_pipeline"];
    if pipeline.is_null() {
        return Ok(());
    }
    let id = positive(pipeline, "id")?;
    let project = positive(pipeline, "project_id")?;
    let sha = text(pipeline, "sha");
    ensure!(threads::oid(&sha), "Invalid pipeline commit");
    let url = same_site(&text(pipeline, "web_url"), &pr.url).context("Invalid pipeline URL")?;
    // MR merged-result pipelines can use a synthetic SHA. Always retain that exact SHA.
    pr.checks.push(Check {
        name: format!("Pipeline #{id} · {}", &sha[..12]),
        state: check_state(&text(pipeline, "status")),
        url: Some(url),
    });
    let (jobs, limited) = pages(
        &format!("projects/{project}/pipelines/{id}/jobs?include_retried=false"),
        MAX_PAGES,
        fetch,
    )?;
    pr.limited |= limited || jobs.len() >= 200;
    for job in jobs.into_iter().take(199) {
        ensure!(
            positive(&job["pipeline"], "id")? == id && text(&job["pipeline"], "sha") == sha,
            "Job does not match MR pipeline"
        );
        pr.checks.push(Check {
            name: format!(
                "{} / {} · {}",
                text(&job, "stage"),
                text(&job, "name"),
                &sha[..12]
            ),
            state: check_state(&text(&job, "status")),
            url: same_site(&text(&job, "web_url"), &pr.url),
        });
    }
    Ok(())
}

fn line(value: &Value) -> Result<Option<u32>> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(
        value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .context("Invalid discussion line")?,
    ))
}

fn discussion(value: &Value, pr: &PullRequest) -> Result<Option<Thread>> {
    let id = text(value, "id");
    ensure!(
        !id.is_empty() && id.len() <= 256 && id.bytes().all(|b| b.is_ascii_alphanumeric()),
        "Invalid discussion ID"
    );
    let notes = value["notes"]
        .as_array()
        .context("Missing discussion notes")?;
    let notes: Vec<_> = notes.iter().filter(|n| n["system"] != true).collect();
    let Some(first) = notes.first() else {
        return Ok(None);
    };
    let position = &first["position"];
    let position_head = position["head_sha"].as_str();
    if let Some(head) = position_head {
        ensure!(threads::oid(head), "Invalid discussion commit");
    }
    let end = &position["line_range"]["end"];
    let new_line = line(&position["new_line"])?;
    let old_line = line(&position["old_line"])?;
    let side = if end["type"] == "old" {
        DiffSide::Left
    } else if end["type"] == "new" || new_line.is_some() {
        DiffSide::Right
    } else {
        DiffSide::Left
    };
    let original_line = if side == DiffSide::Right {
        line(&end["new_line"])?.or(new_line)
    } else {
        line(&end["old_line"])?.or(old_line)
    };
    let outdated = position_head.is_some_and(|head| head != pr.head);
    let path = if side == DiffSide::Right {
        text(position, "new_path")
    } else {
        text(position, "old_path")
    };
    ensure!(
        path.len() <= 4096 && !path.chars().any(char::is_control),
        "Invalid discussion path"
    );
    let mut comments = Vec::new();
    let mut ids = HashSet::new();
    for (index, note) in notes.iter().enumerate() {
        let note_id = positive(note, "id")?;
        ensure!(ids.insert(note_id), "Duplicate discussion note");
        let body = text(note, "body");
        ensure!(body.len() <= 64 * 1024, "Discussion note exceeds limit");
        let updated = text(note, "updated_at");
        chrono::DateTime::parse_from_rfc3339(&updated)?;
        if index != 0 && index < notes.len().saturating_sub(20) {
            continue;
        }
        let commit = note["position"]["head_sha"]
            .as_str()
            .or_else(|| note["commit_id"].as_str());
        if let Some(commit) = commit {
            ensure!(threads::oid(commit), "Invalid note commit");
        }
        let commit = commit.map(|oid| Commit { oid: oid.into() });
        comments.push(threads::Comment {
            id: note_id.to_string(),
            author: note["author"]["username"]
                .as_str()
                .map(|name| Author { login: name.into() }),
            body,
            url: format!("{}#note_{note_id}", pr.url),
            updated_at: updated,
            diff_hunk: String::new(),
            commit: commit.clone(),
            original_commit: commit,
            state: CommentState::Submitted,
        });
    }
    let resolvable = notes.iter().any(|note| note["resolvable"] == true);
    let is_resolved = resolvable
        && notes
            .iter()
            .filter(|note| note["resolvable"] == true)
            .all(|note| note["resolved"] == true);
    let start = &position["line_range"]["start"];
    let start_side = match start["type"].as_str() {
        Some("new") => Some(DiffSide::Right),
        Some("old") => Some(DiffSide::Left),
        _ => None,
    };
    let start_line = if start_side == Some(DiffSide::Right) {
        line(&start["new_line"])?
    } else {
        line(&start["old_line"])?
    };
    Ok(Some(Thread {
        id,
        path,
        diff_side: side,
        start_diff_side: start_side,
        line: if outdated { None } else { original_line },
        start_line: if outdated { None } else { start_line },
        original_line,
        original_start_line: start_line,
        subject_type: if position.is_null() {
            Subject::Discussion
        } else if original_line.is_none() {
            Subject::File
        } else {
            Subject::Line
        },
        is_resolved,
        resolvable,
        is_outdated: outdated,
        total_comments: notes.len(),
        comments,
    }))
}

fn discussions(
    base: &str,
    pr: &PullRequest,
    requested_at: u64,
    fetch: &mut impl FnMut(&str) -> Result<Value>,
) -> Result<Snapshot> {
    let (values, mut limited) = pages(&format!("{base}/discussions"), MAX_PAGES, fetch)?;
    let mut threads = Vec::new();
    for value in values {
        if value["individual_note"] == true {
            continue;
        }
        if let Some(thread) = discussion(&value, pr)? {
            limited |= thread.total_comments > thread.comments.len();
            threads.push(thread);
        }
    }
    Ok(Snapshot {
        pr: pr.url.clone(),
        head: pr.head.clone(),
        fetched_at: timestamp(),
        requested_at,
        threads,
        limited,
    })
}

fn snapshot(
    target: &Target,
    branch: &str,
    local_head: &str,
    fetch: &mut impl FnMut(&str) -> Result<Value>,
) -> Result<PullRequest> {
    snapshot_request(target, branch, local_head, None, fetch)
}

fn snapshot_request(
    target: &Target,
    branch: &str,
    local_head: &str,
    number: Option<u64>,
    fetch: &mut impl FnMut(&str) -> Result<Value>,
) -> Result<PullRequest> {
    let project = fetch(&format!("projects/{}", encode(&target.project)))?;
    let source = positive(&project, "id")?;
    ensure!(
        text(&project, "path_with_namespace") == target.project,
        "GitLab project path mismatch"
    );
    let mut destination = project.clone();
    let find = |project: u64, fetch: &mut dyn FnMut(&str) -> Result<Value>| -> Result<Value> {
        fetch(&format!(
            "projects/{project}/merge_requests?scope=all&state=all&source_branch={}&source_project_id={source}&order_by=updated_at&sort=desc&per_page=100",
            encode(branch)
        ))
    };
    let mut candidates = if let Some(number) = number {
        serde_json::json!([fetch(&format!(
            "projects/{source}/merge_requests/{number}"
        ))?])
    } else {
        find(source, fetch)?
    };
    if candidates.as_array().is_some_and(Vec::is_empty)
        && let Some(parent) = project["forked_from_project"]["id"].as_u64()
    {
        destination = fetch(&format!("projects/{parent}"))?;
        candidates = find(parent, fetch)?;
    }
    let target_id = positive(&destination, "id")?;
    let candidates = candidates.as_array().context("Invalid MR list")?;
    ensure!(
        candidates.len() < 100,
        "{}",
        conduit::i18n::text("gitlab.ambiguous_mr")
    );
    let matching: Vec<_> = candidates
        .iter()
        .filter(|mr| {
            number.is_some()
                || (mr["source_branch"].as_str() == Some(branch)
                    && mr["source_project_id"].as_u64() == Some(source)
                    && mr["target_project_id"].as_u64() == Some(target_id))
        })
        .collect();
    let open: Vec<_> = matching
        .iter()
        .copied()
        .filter(|mr| mr["state"] == "opened")
        .collect();
    ensure!(
        open.len() <= 1,
        "{}",
        conduit::i18n::text("gitlab.ambiguous_mr")
    );
    let selected = open
        .first()
        .or_else(|| matching.first())
        .context(conduit::i18n::text("gitlab.no_mr"))?;
    let selected_number = positive(selected, "iid")?;
    ensure!(
        number.is_none_or(|number| number == selected_number),
        "MR identity changed"
    );
    let base = format!("projects/{target_id}/merge_requests/{selected_number}");
    let value = fetch(&base)?;
    let project_url = text(&destination, "web_url");
    let selected_branch = text(&value, "source_branch");
    let mut pr = parse_mr(
        &value,
        &project_url,
        if number.is_some() {
            &selected_branch
        } else {
            branch
        },
        if number.is_some() {
            positive(&value, "source_project_id")?
        } else {
            source
        },
    )?;
    ensure!(
        pr.number == selected_number && positive(&value, "target_project_id")? == target_id,
        "MR identity changed"
    );
    let requested_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
        .min(u64::MAX as u128) as u64;
    if let Err(error) = checks(&value, &mut pr, fetch) {
        pr.checks_error = Some(format!("{error:#}"));
    }
    let (notes, limited) = pages(
        &format!("{base}/notes?sort=desc&order_by=created_at"),
        4,
        fetch,
    )?;
    pr.limited |= limited;
    for note in notes
        .into_iter()
        .filter(|note| note["system"] != true && note["type"] != "DiffNote")
    {
        let id = positive(&note, "id")?;
        let body = text(&note, "body");
        ensure!(body.len() <= 64 * 1024, "GitLab comment exceeds limit");
        pr.comments.push(Comment {
            author: text(&note["author"], "username"),
            body,
            url: Some(format!("{}#note_{id}", pr.url)),
        });
    }
    match discussions(&base, &pr, requested_at, fetch) {
        Ok(snapshot) => pr.threads = Some(snapshot),
        Err(error) => pr.threads_error = Some(format!("{error:#}")),
    }
    let latest = fetch(&base)?;
    ensure!(
        latest["sha"] == value["sha"]
            && latest["source_branch"] == value["source_branch"]
            && latest["target_branch"] == value["target_branch"]
            && latest["state"] == value["state"]
            && latest["web_url"] == value["web_url"]
            && latest["head_pipeline"]["id"] == value["head_pipeline"]["id"]
            && latest["head_pipeline"]["sha"] == value["head_pipeline"]["sha"]
            && latest["source_project_id"] == value["source_project_id"]
            && latest["target_project_id"] == value["target_project_id"],
        "{}",
        conduit::i18n::text("github.threads_changed")
    );
    pr.local_head = local_head.into();
    pr.cli_host = Some(target.host.clone());
    pr.fetched_at = timestamp();
    Ok(pr)
}

pub(super) fn load(workspace: &Path, target: &Target, cancel: &AtomicBool) -> Result<PullRequest> {
    load_request(workspace, target, None, cancel)
}

pub(super) fn load_request(
    workspace: &Path,
    target: &Target,
    number: Option<u64>,
    cancel: &AtomicBool,
) -> Result<PullRequest> {
    let cli = executable()?;
    let git = crate::git::resolve_git().context(conduit::i18n::text("github.missing_git"))?;
    let head = query(&git, &["rev-parse", "HEAD"], workspace, cancel)?;
    let branch = query(
        &git,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        workspace,
        cancel,
    )?;
    let mut api = Api {
        cli,
        workspace,
        target,
        cancel,
        started: Instant::now(),
        bytes: 0,
    };
    let mut fetch = |endpoint: &str| api.get(endpoint);
    let pr = if number.is_some() {
        snapshot_request(target, branch.trim(), head.trim(), number, &mut fetch)?
    } else {
        snapshot(target, branch.trim(), head.trim(), &mut fetch)?
    };
    ensure!(
        query(&git, &["rev-parse", "HEAD"], workspace, cancel)? == head
            && query(
                &git,
                &["symbolic-ref", "--quiet", "--short", "HEAD"],
                workspace,
                cancel
            )? == branch
            && forge::target(workspace, cancel)? == *target,
        "{}",
        conduit::i18n::text("github.branch_changed")
    );
    Ok(pr)
}

pub(super) fn write_arguments(
    expected: &PullRequest,
    action: &write::Action,
) -> Result<Vec<String>> {
    let target = identity(&expected.url)?;
    let base = format!(
        "projects/{}/merge_requests/{}",
        encode(&target.project),
        target.number
    );
    let (path, method, body) = match action {
        write::Action::Comment(body) => (format!("{base}/notes"), "POST", Some(body)),
        write::Action::Reply { thread, body } => {
            ensure!(
                !thread.id.is_empty() && thread.id.bytes().all(|b| b.is_ascii_alphanumeric()),
                "Invalid discussion ID"
            );
            (
                format!("{base}/discussions/{}/notes", thread.id),
                "POST",
                Some(body),
            )
        }
        write::Action::Resolve(thread) => {
            ensure!(
                thread.resolvable
                    && !thread.id.is_empty()
                    && thread.id.bytes().all(|b| b.is_ascii_alphanumeric()),
                "Discussion cannot be resolved"
            );
            (format!("{base}/discussions/{}", thread.id), "PUT", None)
        }
    };
    let mut args = api_args(
        expected
            .cli_host
            .as_deref()
            .context("Missing GitLab authentication host")?,
        &path,
        method,
    );
    if let Some(body) = body {
        args.extend(["--raw-field".into(), format!("body={body}")]);
    } else {
        args.extend(["--field".into(), "resolved=true".into()]);
    }
    Ok(args)
}

pub(super) fn write_result(json: &str, action: &write::Action) -> Result<()> {
    let value: Value = serde_json::from_str(json)?;
    match action {
        write::Action::Comment(body) | write::Action::Reply { body, .. } => {
            positive(&value, "id")?;
            ensure!(
                value["body"].as_str() == Some(body),
                "GitLab did not confirm the submitted comment"
            );
        }
        write::Action::Resolve(thread) => {
            let notes = value["notes"]
                .as_array()
                .context("Missing resolved discussion")?;
            ensure!(
                value["id"] == thread.id
                    && notes.iter().any(|n| n["resolvable"] == true)
                    && notes
                        .iter()
                        .filter(|n| n["resolvable"] == true)
                        .all(|n| n["resolved"] == true),
                "GitLab did not confirm resolution"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
