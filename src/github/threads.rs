//! Bounded, read-only GraphQL review-thread snapshots. No mutations or implicit imports.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_THREADS: usize = 200;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const PAGE_SIZE: usize = 25;
const RECENT_COMMENTS: usize = 20;

// Keep the first comment (original code evidence) and the most recent replies.
// Cursor pagination is explicit so neither gh nor a large PR can bypass our bounds.
const QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!,$cursor:String) {
  repository(owner:$owner,name:$name) { pullRequest(number:$number) {
    url headRefOid reviewThreads(first:25,after:$cursor) {
      pageInfo { hasNextPage endCursor }
      nodes {
        id path diffSide startDiffSide line startLine originalLine originalStartLine
        subjectType isResolved isOutdated
        first:comments(first:1) { totalCount nodes { ...comment } }
        recent:comments(last:20) { totalCount nodes { ...comment } }
      }
    }
  } }
}
fragment comment on PullRequestReviewComment {
  id body url updatedAt diffHunk state author { login }
  commit { oid } originalCommit { oid }
}"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DiffSide {
    Left,
    Right,
}

impl DiffSide {
    fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Left => "review.original",
            Self::Right => "review.modified",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Subject {
    Line,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub login: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub oid: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CommentState {
    Pending,
    Submitted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub author: Option<Author>,
    pub body: String,
    pub url: String,
    pub updated_at: String,
    pub diff_hunk: String,
    pub commit: Option<Commit>,
    pub original_commit: Option<Commit>,
    pub state: CommentState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    pub path: String,
    pub diff_side: DiffSide,
    pub start_diff_side: Option<DiffSide>,
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    pub original_line: Option<u32>,
    pub original_start_line: Option<u32>,
    pub subject_type: Subject,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub comments: Vec<Comment>,
    pub total_comments: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub pr: String,
    pub head: String,
    pub fetched_at: u64,
    pub requested_at: u64,
    pub threads: Vec<Thread>,
    pub limited: bool,
}

struct Repository {
    host: String,
    owner: String,
    name: String,
    number: u64,
}

fn identity(value: &str) -> Result<Repository> {
    let parsed = url::Url::parse(value)?;
    let segments: Vec<_> = parsed.path_segments().context("Invalid PR path")?.collect();
    if web_url(value).is_none()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.port().is_some()
        || segments.len() != 4
        || segments[2] != "pull"
        || segments[..2].iter().any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        bail!("Invalid PR identity");
    }
    let number = segments[3].parse::<u64>()?;
    if number == 0 || number > i32::MAX as u64 {
        bail!("Invalid PR number");
    }
    Ok(Repository {
        host: parsed.host_str().unwrap().into(),
        owner: segments[0].into(),
        name: segments[1].into(),
        number,
    })
}

fn oid(value: &str) -> bool {
    (40..=64).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

#[derive(Deserialize)]
struct Connection {
    nodes: Vec<Comment>,
    #[serde(rename = "totalCount")]
    total: usize,
}

fn thread(value: &Value, pr: &str) -> Result<Thread> {
    let first: Connection = serde_json::from_value(value["first"].clone())?;
    let recent: Connection = serde_json::from_value(value["recent"].clone())?;
    if first.total != recent.total
        || first.total == 0
        || first.nodes.len() != 1
        || recent.nodes.len() != recent.total.min(RECENT_COMMENTS)
    {
        bail!("Incomplete review comments");
    }
    let mut comments = first.nodes;
    let mut ids = HashSet::from([comments[0].id.clone()]);
    for comment in recent.nodes {
        if ids.insert(comment.id.clone()) {
            comments.push(comment);
        } else if comments.iter().find(|c| c.id == comment.id) != Some(&comment) {
            bail!("Changed review comments");
        }
    }
    if comments.len() != first.total.min(RECENT_COMMENTS + 1) {
        bail!("Incomplete review comments");
    }
    let mut normalized = value.clone();
    normalized["comments"] = serde_json::to_value(comments)?;
    normalized["totalComments"] = serde_json::json!(first.total);
    let result: Thread = serde_json::from_value(normalized)?;
    if !valid_text(&result.id, 256)
        || !valid_text(&result.path, 4096)
        || [
            result.line,
            result.start_line,
            result.original_line,
            result.original_start_line,
        ]
        .contains(&Some(0))
    {
        bail!("Invalid review location");
    }
    for comment in &result.comments {
        let link = url::Url::parse(&comment.url)?;
        let mut without_fragment = link.clone();
        without_fragment.set_fragment(None);
        if !valid_text(&comment.id, 256)
            || comment.body.len() > 64 * 1024
            || comment.diff_hunk.len() > 64 * 1024
            || (without_fragment.as_str() != pr
                && without_fragment.as_str() != format!("{pr}/files"))
            || link.fragment().is_none_or(|f| {
                !f.starts_with("discussion_r")
                    && !f
                        .strip_prefix('r')
                        .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            })
            || comment
                .author
                .as_ref()
                .is_some_and(|a| !valid_text(&a.login, 256))
            || comment.updated_at.len() > 64
            || chrono::DateTime::parse_from_rfc3339(&comment.updated_at).is_err()
            || comment
                .commit
                .iter()
                .chain(comment.original_commit.iter())
                .any(|c| !oid(&c.oid))
        {
            bail!("Invalid review comment");
        }
    }
    Ok(result)
}

struct Page {
    threads: Vec<Thread>,
    next: Option<String>,
}

fn page(json: &str, pr: &str, head: &str) -> Result<Page> {
    let value: Value = serde_json::from_str(json)?;
    if value
        .get("errors")
        .is_some_and(|errors| errors.as_array().is_none_or(|e| !e.is_empty()))
    {
        bail!(conduit::i18n::text("github.threads_failed"));
    }
    let source = &value["data"]["repository"]["pullRequest"];
    if source["url"].as_str() != Some(pr) || source["headRefOid"].as_str() != Some(head) {
        bail!(conduit::i18n::text("github.threads_changed"));
    }
    let connection = &source["reviewThreads"];
    let nodes = connection["nodes"]
        .as_array()
        .context("Missing review threads")?;
    if nodes.len() > PAGE_SIZE {
        bail!("Invalid review page size");
    }
    let more = connection["pageInfo"]["hasNextPage"]
        .as_bool()
        .context("Missing review page info")?;
    let next = if more {
        let cursor = connection["pageInfo"]["endCursor"]
            .as_str()
            .context("Missing review cursor")?;
        if nodes.is_empty() || !valid_text(cursor, 1024) {
            bail!("Invalid review cursor");
        }
        Some(cursor.into())
    } else {
        None
    };
    let threads = nodes
        .iter()
        .map(|value| thread(value, pr))
        .collect::<Result<Vec<_>>>()?;
    Ok(Page { threads, next })
}

fn collect(
    pr: &str,
    head: &str,
    mut fetch: impl FnMut(Option<&str>) -> Result<String>,
) -> Result<Snapshot> {
    identity(pr)?;
    if !oid(head) {
        bail!("Invalid PR head");
    }
    let mut snapshot = Snapshot {
        pr: pr.into(),
        head: head.into(),
        fetched_at: 0,
        requested_at: 0,
        threads: vec![],
        limited: false,
    };
    let mut cursor = None;
    let mut cursors = HashSet::new();
    let mut ids = HashSet::new();
    let mut bytes = 0usize;
    for _ in 0..MAX_THREADS / PAGE_SIZE {
        let json = fetch(cursor.as_deref())?;
        bytes = bytes.saturating_add(json.len());
        if bytes > MAX_BYTES {
            bail!(conduit::i18n::text("github.output_limit"));
        }
        let page = page(&json, pr, head)?;
        for thread in page.threads {
            if !ids.insert(thread.id.clone()) {
                bail!(conduit::i18n::text("github.threads_changed"));
            }
            snapshot.limited |= thread.total_comments > thread.comments.len();
            snapshot.threads.push(thread);
        }
        let Some(next) = page.next else {
            return Ok(snapshot);
        };
        if !cursors.insert(next.clone()) {
            bail!("Repeated review cursor");
        }
        cursor = Some(next);
    }
    snapshot.limited = true;
    Ok(snapshot)
}

pub(super) fn load(
    gh: &Path,
    workspace: &Path,
    pr: &PullRequest,
    cancel: &AtomicBool,
) -> Result<Snapshot> {
    let repository = identity(&pr.url)?;
    if repository.number != pr.number {
        bail!("Invalid PR identity");
    }
    let started = Instant::now();
    let requested_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
        .min(u64::MAX as u128) as u64;
    let mut snapshot = collect(&pr.url, &pr.head, |cursor| {
        if started.elapsed() >= Duration::from_secs(90) {
            bail!(conduit::i18n::text("github.timeout"));
        }
        let mut args = vec![
            "api".into(),
            "graphql".into(),
            "--hostname".into(),
            repository.host.clone(),
            "-f".into(),
            format!("query={QUERY}"),
            "-f".into(),
            format!("owner={}", repository.owner),
            "-f".into(),
            format!("name={}", repository.name),
            "-F".into(),
            format!("number={}", repository.number),
        ];
        if let Some(cursor) = cursor {
            args.extend(["-f".into(), format!("cursor={cursor}")]);
        }
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        query(gh, &args, workspace, cancel)
    })?;
    snapshot.fetched_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    snapshot.requested_at = requested_at;
    Ok(snapshot)
}

impl Thread {
    pub fn location(&self) -> String {
        if self.subject_type == Subject::File {
            return conduit::i18n::text("github.thread_file");
        }
        let show = |line: Option<u32>| line.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        let range = |start: Option<u32>, end: Option<u32>| {
            format!(
                "{} {} → {} {}",
                self.start_diff_side.unwrap_or(self.diff_side).label(),
                show(start.or(end)),
                self.diff_side.label(),
                show(end)
            )
        };
        conduit::i18n::format(
            "github.thread_location",
            &[
                ("current", range(self.start_line, self.line)),
                (
                    "original",
                    range(self.original_start_line, self.original_line),
                ),
            ],
        )
    }

    pub fn remote_status(&self) -> String {
        let resolved = conduit::i18n::text_static(if self.is_resolved {
            "github.thread_resolved"
        } else {
            "github.thread_unresolved"
        });
        if self.is_outdated {
            format!(
                "{resolved} · {}",
                conduit::i18n::text_static("github.thread_outdated")
            )
        } else {
            resolved.into()
        }
    }

    pub fn feedback(&self, pr: &str, head: &str) -> Result<String> {
        let mut output = conduit::i18n::format(
            "github.thread_feedback",
            &[
                ("pr", pr.into()),
                ("head", head.into()),
                ("path", self.path.clone()),
                ("location", self.location()),
                ("status", self.remote_status()),
            ],
        );
        if self.total_comments > self.comments.len() {
            output.push_str(&conduit::i18n::text("github.threads_limited"));
            output.push('\n');
        }
        for comment in &self.comments {
            let author = comment
                .author
                .as_ref()
                .map(|a| a.login.as_str())
                .unwrap_or("?");
            output.push_str(&format!(
                "\n{author} · {}\n{}\n",
                comment.updated_at, comment.url
            ));
            if comment.state == CommentState::Pending {
                output.push_str(&conduit::i18n::text("github.thread_pending"));
                output.push('\n');
            }
            for line in comment.body.lines() {
                output.push_str(&format!("    {line}\n"));
            }
        }
        if let Some(first) = self.comments.first() {
            output.push_str(&conduit::i18n::format(
                "github.thread_evidence",
                &[(
                    "commit",
                    first
                        .original_commit
                        .as_ref()
                        .map(|c| c.oid.clone())
                        .unwrap_or_else(|| "?".into()),
                )],
            ));
            for line in first.diff_hunk.lines() {
                output.push_str(&format!("    {line}\n"));
            }
        }
        if output.len() > crate::review::MAX_FEEDBACK_BYTES {
            bail!(conduit::i18n::text("github.output_limit"));
        }
        Ok(output)
    }
}

#[cfg(test)]
pub(crate) mod tests;
