//! Explicit remote writes. Preflight failures are separate from uncertain mutation outcomes.
use super::*;
use threads::Thread;

pub const MAX_BODY_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug)]
pub enum Action {
    Comment(String),
    Reply { thread: Thread, body: String },
    Resolve(Thread),
}

pub struct WriteError {
    pub message: String,
    pub uncertain: bool,
}

pub fn validate(expected: &PullRequest, current: &PullRequest, action: &Action) -> Result<()> {
    let number = match expected.forge {
        forge::Forge::Github => threads::identity(&expected.url)?.number,
        forge::Forge::Gitlab => gitlab::identity(&expected.url)?.number,
    };
    if number != expected.number
        || expected.forge != current.forge
        || expected.api_host != current.api_host
        || expected.branch.is_empty()
        || !threads::oid(&expected.head)
        || !threads::oid(&expected.local_head)
        || expected.url != current.url
        || expected.number != current.number
        || expected.head != current.head
        || expected.local_head != current.local_head
        || expected.branch != current.branch
        || expected.base != current.base
        || current.state != "OPEN"
    {
        bail!(conduit::i18n::text("github.write_stale"));
    }
    let body = match action {
        Action::Comment(body) | Action::Reply { body, .. } => Some(body),
        Action::Resolve(_) => None,
    };
    if body.is_some_and(|body| {
        body.trim().is_empty() || body.len() > MAX_BODY_BYTES || body.contains('\0')
    }) {
        bail!(conduit::i18n::text("review.comment_limit"));
    }
    if let Action::Reply { thread, .. } | Action::Resolve(thread) = action {
        for pr in [expected, current] {
            let snapshot = pr.threads.as_ref().context("Missing review snapshot")?;
            if snapshot.pr != pr.url
                || snapshot.head != pr.head
                || snapshot.threads.iter().find(|t| t.id == thread.id) != Some(thread)
                || thread
                    .comments
                    .iter()
                    .any(|c| c.state == threads::CommentState::Pending)
                || (matches!(action, Action::Resolve(_))
                    && (thread.is_resolved || !thread.resolvable))
            {
                bail!(conduit::i18n::text("github.write_stale"));
            }
        }
    }
    Ok(())
}

fn graphql_result(json: &str, action: &Action) -> Result<()> {
    let value: Value = serde_json::from_str(json)?;
    if value
        .get("errors")
        .is_some_and(|e| e.as_array().is_none_or(|a| !a.is_empty()))
    {
        bail!(conduit::i18n::text("github.write_failed"));
    }
    match action {
        Action::Reply { .. } => {
            if value["data"]["addPullRequestReviewThreadReply"]["comment"]["id"]
                .as_str()
                .is_none_or(str::is_empty)
            {
                bail!(conduit::i18n::text("github.write_failed"));
            }
        }
        Action::Resolve(thread) => {
            let result = &value["data"]["resolveReviewThread"]["thread"];
            if result["id"].as_str() != Some(thread.id.as_str()) || result["isResolved"] != true {
                bail!(conduit::i18n::text("github.write_failed"));
            }
        }
        Action::Comment(_) => {}
    }
    Ok(())
}

/// One fully resolved remote write; built during preflight and sent exactly once.
#[derive(Clone, Debug)]
pub(super) struct Request {
    pub host: String,
    pub method: api::Method,
    pub url: String,
    pub body: Value,
}

const REPLY: &str = "mutation($thread:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread,body:$body}){comment{id}}}";
const RESOLVE: &str =
    "mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}";

fn request(expected: &PullRequest, action: &Action) -> Result<Request> {
    if expected.forge == forge::Forge::Gitlab {
        return gitlab::write_request(expected, action);
    }
    let repository = threads::identity(&expected.url)?;
    let (method, url, body) = match action {
        Action::Comment(body) => (
            api::Method::Post,
            format!(
                "{}/repos/{}/{}/issues/{}/comments",
                api::rest_base(forge::Forge::Github, &repository.host),
                repository.owner,
                repository.name,
                repository.number
            ),
            serde_json::json!({ "body": body }),
        ),
        Action::Reply { thread, body } => (
            api::Method::Post,
            api::graphql_url(&repository.host),
            serde_json::json!({
                "query": REPLY,
                "variables": { "thread": thread.id, "body": body },
            }),
        ),
        Action::Resolve(thread) => (
            api::Method::Post,
            api::graphql_url(&repository.host),
            serde_json::json!({ "query": RESOLVE, "variables": { "thread": thread.id } }),
        ),
    };
    Ok(Request {
        host: repository.host,
        method,
        url,
        body,
    })
}

fn rest_comment_result(json: &str, body: &str) -> Result<()> {
    let value: Value = serde_json::from_str(json)?;
    if value["id"].as_u64().is_none_or(|id| id == 0) || value["body"].as_str() != Some(body) {
        bail!(conduit::i18n::text("github.write_failed"));
    }
    Ok(())
}

pub struct Prepared {
    forge: forge::Forge,
    request: Request,
    action: Action,
    workspace: PathBuf,
}

pub fn prepare(
    workspace: &Path,
    expected: &PullRequest,
    action: &Action,
    cancel: &AtomicBool,
) -> std::result::Result<Prepared, WriteError> {
    let prepare = || -> Result<Request> {
        validate(expected, expected, action)?;
        let current = load(workspace, cancel)?;
        validate(expected, &current, action)?;
        if cancel.load(Ordering::Acquire) {
            bail!(conduit::i18n::text("github.cancelled"));
        }
        request(expected, action)
    };
    let request = prepare().map_err(|e| WriteError {
        message: format!("{e:#}"),
        uncertain: false,
    })?;
    Ok(Prepared {
        forge: expected.forge,
        request,
        action: action.clone(),
        workspace: workspace.to_owned(),
    })
}

pub fn perform(prepared: Prepared, cancel: &AtomicBool) -> std::result::Result<(), WriteError> {
    let certain = |e: anyhow::Error| WriteError {
        message: format!("{e:#}"),
        uncertain: false,
    };
    if cancel.load(Ordering::Acquire) {
        return Err(certain(anyhow::anyhow!(conduit::i18n::text(
            "github.cancelled"
        ))));
    }
    let Request {
        host,
        method,
        url,
        body,
    } = &prepared.request;
    let mut client = api::Client::new(prepared.forge, host, &prepared.workspace, cancel);
    // Resolve credentials before sending so a missing sign-in is not reported as uncertain.
    client.authenticate().map_err(certain)?;
    // Only a 401 is retried, with the next credential: the server rejected it before acting.
    // Nothing else is resent: timeout/cancellation can happen after the server accepted a write.
    client
        .send(*method, url, Some(body))
        .and_then(|json| match (&prepared.action, prepared.forge) {
            (_, forge::Forge::Gitlab) => gitlab::write_result(&json, &prepared.action),
            (Action::Comment(body), forge::Forge::Github) => rest_comment_result(&json, body),
            (_, forge::Forge::Github) => graphql_result(&json, &prepared.action),
        })
        .map_err(|e| WriteError {
            message: format!("{e:#}"),
            // A 4xx or rejected credential means the server did not apply the write.
            uncertain: !api::rejected(&e),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pr() -> PullRequest {
        let mut pr = parse(
            &json!({
                "number":7, "url":threads::tests::PR, "headRefOid":"a".repeat(40),
                "headRefName":"topic", "state":"OPEN"
            })
            .to_string(),
        )
        .unwrap();
        pr.local_head = "b".repeat(40);
        pr.threads = Some(threads::tests::snapshot());
        pr
    }

    #[test]
    fn preflight_rejects_changed_repository_branch_head_and_review_evidence() {
        let expected = pr();
        let thread = expected.threads.as_ref().unwrap().threads[0].clone();
        let action = Action::Reply {
            thread: thread.clone(),
            body: "Reviewed.\nPlease update.".into(),
        };
        assert!(validate(&expected, &expected, &action).is_ok());
        let mut changed = expected.clone();
        changed.url = "https://github.com/other/repo/pull/7".into();
        assert!(validate(&expected, &changed, &action).is_err());
        changed = expected.clone();
        changed.head = "c".repeat(40);
        assert!(validate(&expected, &changed, &action).is_err());
        changed = expected.clone();
        changed.local_head = "c".repeat(40);
        assert!(validate(&expected, &changed, &action).is_err());
        changed = expected.clone();
        changed.branch = "other".into();
        assert!(validate(&expected, &changed, &action).is_err());
        changed = expected.clone();
        changed.threads.as_mut().unwrap().threads[0].comments[0].body = "Edited by reviewer".into();
        assert!(validate(&expected, &changed, &action).is_err());
        changed = expected.clone();
        changed.threads.as_mut().unwrap().threads.clear();
        assert!(validate(&expected, &changed, &action).is_err());
        assert!(validate(&expected, &expected, &Action::Resolve(thread)).is_ok());
        changed = expected.clone();
        changed.state = "MERGED".into();
        assert!(validate(&expected, &changed, &action).is_err());
    }

    #[test]
    fn replies_preserve_literal_content_and_route_to_explicit_host() {
        let mut expected = pr();
        expected.url = "https://git.example.com/example/repo/pull/7".into();
        let body = "中文\n$(echo secret) 'literal' \"quoted\"";
        let thread = expected.threads.as_ref().unwrap().threads[0].clone();
        let request = request(
            &expected,
            &Action::Reply {
                thread,
                body: body.into(),
            },
        )
        .unwrap();
        assert_eq!(request.host, "git.example.com");
        assert_eq!(request.url, "https://git.example.com/api/graphql");
        assert_eq!(request.body["variables"]["body"], body);
        let comment = super::request(&expected, &Action::Comment(body.into())).unwrap();
        assert_eq!(
            comment.url,
            "https://git.example.com/api/v3/repos/example/repo/issues/7/comments"
        );
        assert_eq!(comment.body["body"], body);
        assert!(rest_comment_result(&json!({"id": 1, "body": body}).to_string(), body).is_ok());
        assert!(rest_comment_result(&json!({"id": 1, "body": "other"}).to_string(), body).is_err());
        for body in [
            "".to_owned(),
            " \n".into(),
            "a\0b".into(),
            "x".repeat(MAX_BODY_BYTES + 1),
        ] {
            assert!(validate(&expected, &expected, &Action::Comment(body)).is_err());
        }
    }

    #[test]
    fn malformed_or_partial_graphql_success_never_confirms_a_write() {
        let pr = pr();
        let thread = pr.threads.unwrap().threads.remove(0);
        let resolve = Action::Resolve(thread.clone());
        for response in [
            "{}",
            r#"{"errors":"unexpected"}"#,
            r#"{"data":{"resolveReviewThread":{"thread":{"id":"other","isResolved":true}}}}"#,
            r#"{"data":{"resolveReviewThread":{"thread":{"id":"T1","isResolved":false}}}}"#,
        ] {
            assert!(graphql_result(response, &resolve).is_err());
        }
        assert!(
            graphql_result(
                r#"{"data":{"resolveReviewThread":{"thread":{"id":"T1","isResolved":true}}}}"#,
                &resolve
            )
            .is_ok()
        );
        let reply = Action::Reply {
            thread,
            body: "reply".into(),
        };
        assert!(graphql_result(r#"{"data":{"addPullRequestReviewThreadReply":{"comment":{"id":"new"}}},"errors":[{"message":"partial"}]}"#, &reply).is_err());
        assert!(
            graphql_result(
                r#"{"data":{"addPullRequestReviewThreadReply":{"comment":{"id":"new"}}}}"#,
                &reply
            )
            .is_ok()
        );
    }
}
