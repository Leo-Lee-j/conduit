use super::*;
use serde_json::json;

const ROOT: &str = "https://code.example.test:8443/team/sub/repo";
const MR: &str = "https://code.example.test:8443/team/sub/repo/-/merge_requests/7";

fn target() -> Target {
    Target {
        forge: Forge::Gitlab,
        host: "code.example.test".into(),
        project: "team/sub/repo".into(),
    }
}

fn merge_request() -> Value {
    json!({"id":70,"iid":7,"web_url":MR,"title":"修复界面","sha":"a".repeat(40),"state":"opened",
        "source_branch":"feature/中文","target_branch":"main","source_project_id":12,"target_project_id":12,
        "head_pipeline":{"id":90,"project_id":12,"sha":"c".repeat(40),"status":"success","web_url":format!("{ROOT}/-/pipelines/90")}})
}

fn discussion_value() -> Value {
    json!({"id":"abc123","individual_note":false,"notes":[{
        "id":31,"system":false,"body":"请检查边界","updated_at":"2026-11-17T08:00:00Z","author":{"username":"reviewer"},
        "resolvable":true,"resolved":false,"position":{"head_sha":"b".repeat(40),"old_path":"src/旧.rs","new_path":"src/新.rs","old_line":null,"new_line":22,
        "line_range":{"start":{"type":"new","new_line":20}}}
    }]})
}

fn fetch(endpoint: &str) -> Result<Value> {
    if endpoint == "projects/team%2Fsub%2Frepo" {
        return Ok(json!({"id":12,"path_with_namespace":"team/sub/repo","web_url":ROOT}));
    }
    if endpoint.starts_with("projects/12/merge_requests?") {
        assert!(endpoint.contains("source_branch=feature%2F%E4%B8%AD%E6%96%87"));
        assert!(endpoint.contains("source_project_id=12"));
        return Ok(json!([merge_request()]));
    }
    if endpoint == "projects/12/merge_requests/7" {
        return Ok(merge_request());
    }
    if endpoint.starts_with("projects/12/pipelines/90/jobs?") {
        return Ok(json!([
            {"id":91,"stage":"test","name":"unit","status":"failed","web_url":format!("{ROOT}/-/jobs/91"),"pipeline":{"id":90,"sha":"c".repeat(40)}},
            {"id":92,"stage":"deploy","name":"release","status":"manual","web_url":"javascript:alert(1)","pipeline":{"id":90,"sha":"c".repeat(40)}}
        ]));
    }
    if endpoint.starts_with("projects/12/merge_requests/7/notes?") {
        return Ok(json!([
            {"id":34,"system":true,"body":"system"},
            {"id":33,"type":"DiffNote","system":false,"body":"line"},
            {"id":32,"system":false,"body":"整体评论","author":{"username":"leo"}}
        ]));
    }
    if endpoint.starts_with("projects/12/merge_requests/7/discussions?") {
        return Ok(json!([discussion_value(),{"id":"individual","individual_note":true}]));
    }
    bail!("Unexpected endpoint: {endpoint}")
}

pub(crate) fn loaded() -> PullRequest {
    snapshot(&target(), "feature/中文", &"a".repeat(40), &mut fetch).unwrap()
}

#[test]
fn explicit_sidebar_request_loads_without_changing_local_branch_identity() {
    let head = "d".repeat(40);
    let pr = snapshot_request(
        &target(),
        "unrelated-local-branch",
        &head,
        Some(7),
        &mut fetch,
    )
    .unwrap();
    assert_eq!(pr.number, 7);
    assert_eq!(pr.branch, "feature/中文");
    assert_eq!(pr.local_head, head);
    assert!(
        snapshot_request(
            &target(),
            "feature/中文",
            &head,
            Some(8),
            &mut |endpoint| {
                if endpoint == "projects/12/merge_requests/8" {
                    Ok(merge_request())
                } else {
                    fetch(endpoint)
                }
            }
        )
        .is_err()
    );
}

#[test]
fn self_hosted_snapshot_keeps_pipeline_sha_comments_and_outdated_discussion_evidence() {
    let pr = loaded();
    assert_eq!(pr.forge, Forge::Gitlab);
    assert_eq!(pr.forge.request_name(pr.number), "MR !7");
    assert_eq!(pr.api_host.as_deref(), Some("code.example.test"));
    assert_eq!(pr.checks.len(), 3);
    assert!(pr.checks[0].name.contains("cccccccccccc"));
    assert_eq!(pr.checks[1].state, "FAILURE");
    assert!(pr.checks[1].needs_attention());
    assert_eq!(pr.checks[2].state, "ACTION_REQUIRED");
    assert!(pr.checks[2].url.is_none());
    assert_eq!(pr.comments.len(), 1);
    assert_eq!(pr.comments[0].body, "整体评论");
    let thread = &pr.threads.as_ref().unwrap().threads[0];
    assert!(thread.is_outdated);
    assert!(thread.resolvable);
    assert_eq!(thread.line, None);
    assert_eq!(thread.original_line, Some(22));
    assert_eq!(thread.original_start_line, Some(20));
    assert_eq!(thread.path, "src/新.rs");
    assert_eq!(
        thread.comments[0].original_commit.as_ref().unwrap().oid,
        "b".repeat(40)
    );
    assert!(thread.comments[0].diff_hunk.is_empty());
    assert_eq!(thread.comments[0].url, format!("{MR}#note_31"));
    let task = pr.check_task(1, Path::new("C:/repo"), 1).unwrap();
    assert!(task.goal.contains(MR));
}

#[test]
fn mr_selection_rejects_ambiguous_open_requests_wrong_sources_and_changed_heads() {
    type Transform = Box<dyn FnMut(&str, Value) -> Value>;
    let run = |mut transform: Transform| {
        snapshot(&target(), "feature/中文", &"a".repeat(40), &mut |path| {
            Ok(transform(path, fetch(path)?))
        })
    };
    assert!(
        run(Box::new(|path, mut value| {
            if path.starts_with("projects/12/merge_requests?") {
                let mut second = value[0].clone();
                second["iid"] = json!(8);
                value.as_array_mut().unwrap().push(second);
            }
            value
        }))
        .is_err()
    );
    assert!(
        run(Box::new(|path, mut value| {
            if path == "projects/12/merge_requests/7" {
                value["source_project_id"] = json!(99);
            }
            value
        }))
        .is_err()
    );
    let mut reads = 0;
    assert!(
        run(Box::new(move |path, mut value| {
            if path == "projects/12/merge_requests/7" {
                reads += 1;
                if reads == 2 {
                    value["sha"] = json!("d".repeat(40));
                }
            }
            value
        }))
        .is_err()
    );
    assert!(
        run(Box::new(|path, mut value| {
            if path == "projects/12/merge_requests/7" {
                value["web_url"] = json!("https://elsewhere.test/x/y/-/merge_requests/7");
            }
            value
        }))
        .is_err()
    );
}

#[test]
fn fork_requests_use_the_source_project_and_target_project_endpoints() {
    let mut paths = Vec::new();
    let result = snapshot(&target(), "feature/中文", &"a".repeat(40), &mut |path| {
        paths.push(path.to_owned());
        if path == "projects/team%2Fsub%2Frepo" {
            return Ok(json!({"id":15,"path_with_namespace":"team/sub/repo","web_url":"https://code.example.test/team/sub/repo","forked_from_project":{"id":12}}));
        }
        if path.starts_with("projects/15/merge_requests?") { return Ok(json!([])); }
        if path == "projects/12" { return Ok(json!({"id":12,"web_url":ROOT})); }
        if path.starts_with("projects/12/merge_requests?") {
            assert!(path.contains("source_project_id=15"));
            let mut mr = merge_request(); mr["source_project_id"] = json!(15);
            return Ok(json!([mr]));
        }
        let mut value = fetch(path)?;
        if path == "projects/12/merge_requests/7" { value["source_project_id"] = json!(15); }
        Ok(value)
    }).unwrap();
    assert_eq!(result.url, MR);
    assert!(paths.contains(&"projects/12".into()));
}

#[test]
fn partial_jobs_and_discussion_failures_are_explicit_without_losing_mr() {
    let pr = snapshot(&target(), "feature/中文", &"a".repeat(40), &mut |path| {
        if path.contains("/jobs?") || path.contains("/discussions?") {
            bail!("403 Forbidden");
        }
        fetch(path)
    })
    .unwrap();
    assert!(pr.checks_error.unwrap().contains("403"));
    assert!(pr.threads_error.unwrap().contains("403"));
    assert!(pr.threads.is_none());
    assert_eq!(pr.comments.len(), 1);
}

#[test]
fn pagination_is_bounded_and_duplicates_are_not_accepted() {
    let mut calls = 0;
    let (values, limited) = pages("projects/12/notes", 2, &mut |path| {
        calls += 1;
        assert!(path.contains(&format!("page={calls}")));
        Ok(json!(
            (0..25)
                .map(|n| json!({"id":calls*25+n}))
                .collect::<Vec<_>>()
        ))
    })
    .unwrap();
    assert_eq!(values.len(), 50);
    assert!(limited);
    assert_eq!(calls, 2);
    assert!(
        pages("notes", 2, &mut |_| Ok(json!(
            (0..25).map(|n| json!({"id":n})).collect::<Vec<_>>()
        )))
        .is_err()
    );
    assert!(pages("notes", 2, &mut |_| Ok(json!({"message":"Unauthorized"}))).is_err());
}

#[test]
fn non_resolvable_and_truncated_discussions_preserve_first_and_latest_notes() {
    let pr = loaded();
    let mut value = discussion_value();
    let mut note = value["notes"][0].clone();
    note["resolvable"] = json!(false);
    note["position"] = Value::Null;
    value["notes"] = json!(
        (1..=30)
            .map(|id| {
                let mut n = note.clone();
                n["id"] = json!(id);
                n
            })
            .collect::<Vec<_>>()
    );
    let thread = discussion(&value, &pr).unwrap().unwrap();
    assert!(!thread.resolvable);
    assert_eq!(thread.subject_type, Subject::Discussion);
    assert_eq!(thread.total_comments, 30);
    assert_eq!(thread.comments.len(), 21);
    assert_eq!(thread.comments[0].id, "1");
    assert_eq!(thread.comments[1].id, "11");
    assert_eq!(thread.comments[20].id, "30");
    value["notes"][0]["body"] = json!("a".repeat(64 * 1024 + 1));
    assert!(discussion(&value, &pr).is_err());
}

#[test]
fn writes_pin_authentication_host_and_preserve_literal_bodies() {
    let pr = loaded();
    let thread = pr.threads.as_ref().unwrap().threads[0].clone();
    let body = "中文\n@secret $(echo no) 'quoted'";
    let action = write::Action::Reply {
        thread: thread.clone(),
        body: body.into(),
    };
    assert!(write::validate(&pr, &pr, &action).is_ok());
    let request = write_request(&pr, &action).unwrap();
    assert_eq!(
        request.url,
        "https://code.example.test/api/v4/projects/team%2Fsub%2Frepo/merge_requests/7/discussions/abc123/notes"
    );
    assert_eq!(request.host, "code.example.test");
    assert_eq!(request.method, api::Method::Post);
    assert_eq!(request.body, json!({ "body": body }));
    assert!(write_result(&json!({"id":100,"body":body}).to_string(), &action).is_ok());
    for invalid in ["{}", "[]", "{\"id\":100,\"body\":\"different\"}"] {
        assert!(write_result(invalid, &action).is_err());
    }
    let resolve = write::Action::Resolve(thread);
    let request = write_request(&pr, &resolve).unwrap();
    assert_eq!(request.method, api::Method::Put);
    assert_eq!(request.body, json!({ "resolved": true }));
    assert!(
        write_result(
            r#"{"id":"abc123","notes":[{"resolvable":true,"resolved":true}]}"#,
            &resolve
        )
        .is_ok()
    );
    assert!(write_result(r#"{"id":"other","notes":[]}"#, &resolve).is_err());
    let mut changed = pr.clone();
    changed.api_host = Some("other.test".into());
    assert!(write::validate(&pr, &changed, &action).is_err());
    changed = pr.clone();
    changed.head = "d".repeat(40);
    assert!(write::validate(&pr, &changed, &action).is_err());
    changed = pr.clone();
    changed.threads.as_mut().unwrap().threads[0].resolvable = false;
    assert!(
        write::validate(
            &changed,
            &changed,
            &write::Action::Resolve(changed.threads.as_ref().unwrap().threads[0].clone())
        )
        .is_err()
    );
}

#[test]
fn unknown_and_non_failure_pipeline_states_never_pass_or_create_failure_tasks() {
    for value in ["pending", "running", "canceled", "skipped", "future_status"] {
        let check = Check {
            name: "job".into(),
            state: check_state(value),
            url: None,
        };
        assert_ne!(check.state, "SUCCESS");
        assert!(!check.needs_attention());
    }
}

#[test]
fn multiline_discussion_uses_end_side_and_pipeline_progress_does_not_invalidate_snapshot() {
    let pr = loaded();
    let mut value = discussion_value();
    value["notes"][0]["position"]["line_range"]["end"] =
        json!({"type":"old","old_line":27,"new_line":28});
    let thread = discussion(&value, &pr).unwrap().unwrap();
    assert_eq!(thread.diff_side, DiffSide::Left);
    assert_eq!(thread.path, "src/旧.rs");
    assert_eq!(thread.original_line, Some(27));
    let mut reads = 0;
    let result = snapshot(&target(), "feature/中文", &"a".repeat(40), &mut |path| {
        let mut value = fetch(path)?;
        if path == "projects/12/merge_requests/7" {
            reads += 1;
            if reads == 2 {
                value["head_pipeline"]["status"] = json!("running");
            }
        }
        Ok(value)
    });
    assert!(result.is_ok());
}
