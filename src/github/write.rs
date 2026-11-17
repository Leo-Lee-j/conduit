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
        || expected.cli_host != current.cli_host
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

fn arguments(expected: &PullRequest, action: &Action) -> Result<Vec<String>> {
    if expected.forge == forge::Forge::Gitlab {
        return gitlab::write_arguments(expected, action);
    }
    let repository = threads::identity(&expected.url)?;
    if let Action::Comment(body) = action {
        return Ok(vec![
            "pr".into(),
            "comment".into(),
            expected.url.clone(),
            "--body".into(),
            body.clone(),
        ]);
    }
    let mut args = vec![
        "api".into(),
        "graphql".into(),
        "--hostname".into(),
        repository.host,
        "-f".into(),
    ];
    match action {
        Action::Reply { thread, body } => {
            args.extend([
                "query=mutation($thread:ID!,$body:String!){addPullRequestReviewThreadReply(input:{pullRequestReviewThreadId:$thread,body:$body}){comment{id}}}".into(),
                "-f".into(), format!("thread={}", thread.id), "-f".into(), format!("body={body}")
            ]);
        }
        Action::Resolve(thread) => {
            args.extend([
                "query=mutation($thread:ID!){resolveReviewThread(input:{threadId:$thread}){thread{id isResolved}}}".into(),
                "-f".into(), format!("thread={}", thread.id)
            ]);
        }
        Action::Comment(_) => unreachable!(),
    }
    Ok(args)
}

pub struct Prepared {
    forge: forge::Forge,
    gh: PathBuf,
    args: Vec<String>,
    action: Action,
    workspace: PathBuf,
}

pub fn prepare(
    workspace: &Path,
    expected: &PullRequest,
    action: &Action,
    cancel: &AtomicBool,
) -> std::result::Result<Prepared, WriteError> {
    let prepare = || -> Result<(PathBuf, Vec<String>)> {
        validate(expected, expected, action)?;
        let current = load(workspace, cancel)?;
        validate(expected, &current, action)?;
        if cancel.load(Ordering::Acquire) {
            bail!(conduit::i18n::text("github.cancelled"));
        }
        Ok((
            match expected.forge {
                forge::Forge::Github => {
                    executable().context(conduit::i18n::text("github.missing_cli"))?
                }
                forge::Forge::Gitlab => gitlab::executable()?,
            },
            arguments(expected, action)?,
        ))
    };
    let (gh, args) = prepare().map_err(|e| WriteError {
        message: format!("{e:#}"),
        uncertain: false,
    })?;
    Ok(Prepared {
        forge: expected.forge,
        gh,
        args,
        action: action.clone(),
        workspace: workspace.to_owned(),
    })
}

pub fn perform(prepared: Prepared, cancel: &AtomicBool) -> std::result::Result<(), WriteError> {
    if cancel.load(Ordering::Acquire) {
        return Err(WriteError {
            message: conduit::i18n::text("github.cancelled"),
            uncertain: false,
        });
    }
    let args: Vec<_> = prepared.args.iter().map(String::as_str).collect();
    // No retries: timeout/cancellation can happen after the server accepted a write.
    query(&prepared.gh, &args, &prepared.workspace, cancel)
        .and_then(|json| {
            if prepared.forge == forge::Forge::Gitlab {
                gitlab::write_result(&json, &prepared.action)
            } else if matches!(prepared.action, Action::Comment(_)) {
                Ok(())
            } else {
                graphql_result(&json, &prepared.action)
            }
        })
        .map_err(|e| WriteError {
            message: format!("{e:#}"),
            uncertain: true,
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
        let args = arguments(
            &expected,
            &Action::Reply {
                thread,
                body: body.into(),
            },
        )
        .unwrap();
        assert_eq!(
            &args[..4],
            ["api", "graphql", "--hostname", "git.example.com"]
        );
        assert_eq!(args.last().unwrap(), &format!("body={body}"));
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
