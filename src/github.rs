//! PR / MR snapshots and explicitly requested mutations over the forge HTTP APIs.
use crate::child::{Child, available};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

mod api;
pub mod auth;
pub mod checks;
pub mod forge;
pub(crate) mod gitlab;
pub mod list;
mod pulls;
pub mod threads;
pub mod write;

#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub state: String,
    pub url: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub url: Option<String>,
}
#[derive(Clone, Debug)]
pub struct PullRequest {
    pub forge: forge::Forge,
    pub api_host: Option<String>,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head: String,
    pub branch: String,
    pub base: String,
    pub state: String,
    pub decision: String,
    pub checks: Vec<Check>,
    pub checks_error: Option<String>,
    pub comments: Vec<Comment>,
    pub limited: bool,
    pub local_head: String,
    pub fetched_at: u64,
    pub threads: Option<threads::Snapshot>,
    pub threads_error: Option<String>,
}

pub fn web_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

pub fn parse(json: &str) -> Result<PullRequest> {
    let value: Value = serde_json::from_str(json)?;
    let number = value["number"].as_u64().context("Missing PR number")?;
    let url = web_url(value["url"].as_str().unwrap_or_default()).context("Invalid PR URL")?;
    let head = text(&value, "headRefOid");
    if head.len() < 40 || head.len() > 64 || !head.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid PR head");
    }
    let checks: Vec<_> = value["statusCheckRollup"]
        .as_array()
        .into_iter()
        .flatten()
        .take(200)
        .map(|check| {
            let conclusion = text(check, "conclusion");
            let state = if !conclusion.is_empty() {
                conclusion
            } else {
                let state = text(check, "state");
                if state.is_empty() {
                    text(check, "status")
                } else {
                    state
                }
            };
            Check {
                name: check["name"]
                    .as_str()
                    .or(check["context"].as_str())
                    .unwrap_or("—")
                    .into(),
                state,
                url: check["detailsUrl"]
                    .as_str()
                    .or(check["targetUrl"].as_str())
                    .and_then(web_url),
            }
        })
        .collect();
    let comments: Vec<_> = value["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .rev()
        .take(100)
        .map(|comment| Comment {
            author: text(&comment["author"], "login"),
            body: text(comment, "body"),
            url: comment["url"].as_str().and_then(web_url),
        })
        .collect();
    Ok(PullRequest {
        forge: forge::Forge::Github,
        api_host: None,
        number,
        title: text(&value, "title"),
        url,
        head,
        branch: text(&value, "headRefName"),
        base: text(&value, "baseRefName"),
        state: text(&value, "state"),
        decision: text(&value, "reviewDecision"),
        limited: value["statusCheckRollup"]
            .as_array()
            .is_some_and(|a| a.len() > checks.len())
            || value["comments"]
                .as_array()
                .is_some_and(|a| a.len() > comments.len()),
        checks,
        checks_error: None,
        comments,
        local_head: String::new(),
        fetched_at: 0,
        threads: None,
        threads_error: None,
    })
}

fn executable() -> Option<PathBuf> {
    let mut candidates: Vec<_> = std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|p| p.is_absolute())
                .map(|p| p.join("gh.exe"))
                .collect()
        })
        .unwrap_or_default();
    if let Some(root) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(root).join("GitHub CLI/gh.exe"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn drain(pipe: &mut File, output: &mut Vec<u8>, limit: usize) -> Result<bool> {
    let available = available(pipe).unwrap_or(0) as usize;
    if available == 0 {
        return Ok(false);
    }
    let mut buffer = vec![0; available.min(32 * 1024)];
    let count = pipe.read(&mut buffer)?;
    if output.len() + count > limit {
        bail!(conduit::i18n::text("github.output_limit"));
    }
    output.extend_from_slice(&buffer[..count]);
    Ok(count > 0)
}

fn query(
    executable: &Path,
    args: &[&str],
    workspace: &Path,
    cancel: &AtomicBool,
) -> Result<String> {
    query_with_input(executable, args, workspace, cancel, None)
}

/// Run a non-interactive child process; optional `input` is written to stdin before it closes.
fn query_with_input(
    executable: &Path,
    args: &[&str],
    workspace: &Path,
    cancel: &AtomicBool,
    input: Option<&[u8]>,
) -> Result<String> {
    if cancel.load(Ordering::Acquire) {
        bail!(conduit::i18n::text("github.cancelled"));
    }
    let environment = BTreeMap::from([
        ("GH_PROMPT_DISABLED".into(), "1".into()),
        ("GH_NO_UPDATE_NOTIFIER".into(), "1".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        // Git Credential Manager must never open a sign-in window from a background refresh.
        ("GCM_INTERACTIVE".into(), "never".into()),
        // Git falls back to askpass programs before honoring GIT_TERMINAL_PROMPT.
        ("GIT_ASKPASS".into(), "".into()),
        ("SSH_ASKPASS".into(), "".into()),
        ("GH_PAGER".into(), "cat".into()),
        ("GLAB_CHECK_UPDATE".into(), "false".into()),
        ("GLAB_DEBUG_HTTP".into(), "".into()),
        ("NO_COLOR".into(), "1".into()),
    ]);
    let args: Vec<_> = args.iter().map(|s| (*s).to_owned()).collect();
    let (child, mut pipes) = Child::spawn_with_env(executable, &args, workspace, &environment)?;
    if let Some(input) = input {
        use std::io::Write;
        if let Err(error) = pipes.stdin.write_all(input) {
            child.terminate()?;
            return Err(error.into());
        }
    }
    drop(pipes.stdin);
    let result = (|| {
        let started = Instant::now();
        let mut stdout = vec![];
        let mut stderr = vec![];
        loop {
            if cancel.load(Ordering::Acquire) {
                bail!(conduit::i18n::text("github.cancelled"));
            }
            if started.elapsed() >= Duration::from_secs(45) {
                bail!(conduit::i18n::text("github.timeout"));
            }
            let progressed = drain(&mut pipes.stdout, &mut stdout, 2 * 1024 * 1024)?
                | drain(&mut pipes.stderr, &mut stderr, 32 * 1024)?;
            if let Some(code) = child.exit_code()? {
                while drain(&mut pipes.stdout, &mut stdout, 2 * 1024 * 1024)? {}
                while drain(&mut pipes.stderr, &mut stderr, 32 * 1024)? {}
                if code != 0 {
                    bail!("{}: {}", code, String::from_utf8_lossy(&stderr));
                }
                return String::from_utf8(stdout).context("Invalid UTF-8 response");
            }
            if !progressed {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    })();
    child.terminate()?;
    result
}

pub fn load(workspace: &Path, cancel: &AtomicBool) -> Result<PullRequest> {
    let target = forge::target(workspace, cancel)?;
    if target.forge == forge::Forge::Gitlab {
        return gitlab::load(workspace, &target, cancel);
    }
    load_github(workspace, None, cancel)
}

pub fn load_request(
    workspace: &Path,
    forge: forge::Forge,
    number: u64,
    url: &str,
    cancel: &AtomicBool,
) -> Result<PullRequest> {
    anyhow::ensure!(number > 0, "Invalid request number");
    let target = forge::target(workspace, cancel)?;
    anyhow::ensure!(target.forge == forge, "Repository provider changed");
    let pr = if target.forge == forge::Forge::Gitlab {
        gitlab::load_request(workspace, &target, Some(number), cancel)?
    } else {
        load_github(workspace, Some(number), cancel)?
    };
    anyhow::ensure!(
        pr.url == url,
        "Request repository changed; refresh the list"
    );
    Ok(pr)
}

fn load_github(workspace: &Path, number: Option<u64>, cancel: &AtomicBool) -> Result<PullRequest> {
    let git = crate::git::resolve_git().context(conduit::i18n::text("github.missing_git"))?;
    let head = query(&git, &["rev-parse", "HEAD"], workspace, cancel)?;
    let branch = query(
        &git,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        workspace,
        cancel,
    )?;
    let target = forge::target(workspace, cancel)?;
    let mut client = api::Client::new(forge::Forge::Github, &target.host, workspace, cancel);
    let mut pr = pulls::load(&mut client, &target, branch.trim(), number)?;
    match threads::load(&mut client, &pr) {
        Ok(snapshot) => pr.threads = Some(snapshot),
        Err(error) => pr.threads_error = Some(format!("{error:#}")),
    }
    let current = query(&git, &["rev-parse", "HEAD"], workspace, cancel)?;
    let current_branch = query(
        &git,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        workspace,
        cancel,
    )?;
    if current != head || current_branch != branch {
        bail!(conduit::i18n::text("github.branch_changed"));
    }
    if (number.is_none() && pr.branch != branch.trim())
        || number.is_some_and(|number| pr.number != number)
        || forge::target(workspace, cancel)? != target
    {
        bail!(conduit::i18n::text("github.branch_changed"));
    }
    pr.local_head = head.trim().to_owned();
    pr.fetched_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    Ok(pr)
}

impl PullRequest {
    pub fn feedback(&self, check: usize) -> Option<String> {
        let check = self.checks.get(check)?;
        Some(conduit::i18n::format(
            "github.feedback",
            &[
                ("url", self.url.clone()),
                ("head", self.head.clone()),
                ("name", check.name.clone()),
                ("state", check.state.clone()),
                ("details", check.url.clone().unwrap_or_default()),
            ],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checks_preserve_pending_unknown_and_unsafe_links_are_not_opened() {
        let value = serde_json::json!({"number": 7, "url":"https://github.com/example/repo/pull/7", "headRefOid":"a".repeat(40), "statusCheckRollup":[{"name":"test","status":"IN_PROGRESS","conclusion":"","detailsUrl":"file:///C:/secret"},{"context":"legacy","state":"ERROR","targetUrl":"https://ci.example/job"}],"comments":[{"author":{"login":"reviewer"},"body":"check empty input","url":"javascript:alert(1)"}]});
        let pr = parse(&value.to_string()).unwrap();
        assert_eq!(pr.checks[0].state, "IN_PROGRESS");
        assert!(pr.checks[0].url.is_none());
        assert_eq!(pr.checks[1].state, "ERROR");
        assert!(pr.comments[0].url.is_none());
        assert!(pr.feedback(1).unwrap().contains("legacy"));
        assert!(parse("{}").is_err());
    }
}
