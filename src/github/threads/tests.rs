use super::*;
use serde_json::json;

pub(crate) const PR: &str = "https://github.com/example/repo/pull/7";
pub(crate) fn comment(id: usize) -> Value {
    json!({"id":format!("C{id}"),"author":{"login":"reviewer"},"body":format!("Check input {id}"),
        "url":format!("{PR}#discussion_r{id}"),"updatedAt":"2026-11-10T10:00:00Z","diffHunk":"@@ -1 +1 @@\n-old\n+new",
        "commit":{"oid":"a".repeat(40)},"originalCommit":{"oid":"b".repeat(40)},"state":"SUBMITTED"})
}

pub(crate) fn node(id: usize) -> Value {
    json!({"id":format!("T{id}"),"path":"a.rs","diffSide":"RIGHT","startDiffSide":null,
        "line":12,"startLine":null,"originalLine":11,"originalStartLine":null,
        "subjectType":"LINE","isResolved":false,"isOutdated":false,
        "first":{"totalCount":1,"nodes":[comment(id)]},"recent":{"totalCount":1,"nodes":[comment(id)]}})
}

fn response(nodes: Vec<Value>, next: Option<&str>) -> String {
    json!({"data":{"repository":{"pullRequest":{"url":PR,"headRefOid":"a".repeat(40),
        "reviewThreads":{"nodes":nodes,"pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}}}}}}).to_string()
}

pub(crate) fn snapshot() -> Snapshot {
    let mut result = collect(PR, &"a".repeat(40), |_| Ok(response(vec![node(1)], None))).unwrap();
    result.fetched_at = 100;
    result.requested_at = 100_000_000_000;
    result
}

#[test]
fn review_threads_preserve_original_sides_null_locations_and_file_comments() {
    let mut old = node(1);
    old["diffSide"] = json!("LEFT");
    old["startDiffSide"] = json!("RIGHT");
    old["line"] = Value::Null;
    old["originalStartLine"] = json!(9);
    old["isResolved"] = json!(true);
    old["isOutdated"] = json!(true);
    let mut file = node(2);
    file["subjectType"] = json!("FILE");
    file["line"] = Value::Null;
    file["originalLine"] = Value::Null;
    let result = collect(PR, &"a".repeat(40), |_| {
        Ok(response(vec![old.clone(), file.clone()], None))
    })
    .unwrap();
    let thread = &result.threads[0];
    assert!(thread.is_resolved && thread.is_outdated);
    assert_eq!(thread.line, None);
    assert_eq!(thread.original_start_line, Some(9));
    assert_eq!(thread.diff_side, DiffSide::Left);
    assert_eq!(thread.start_diff_side, Some(DiffSide::Right));
    assert_eq!(result.threads[1].subject_type, Subject::File);
    let feedback = thread.feedback(PR, &result.head).unwrap();
    assert!(feedback.contains("    Check input 1"));
    assert!(feedback.contains(&"b".repeat(40)));
    assert!(feedback.contains("    +new"));
}

#[test]
fn review_threads_keep_first_and_recent_comments_and_mark_omissions() {
    let mut node = node(1);
    node["first"]["totalCount"] = json!(30);
    node["recent"] = json!({"totalCount":30,"nodes":(11..=30).map(comment).collect::<Vec<_>>()});
    let result = collect(PR, &"a".repeat(40), |_| {
        Ok(response(vec![node.clone()], None))
    })
    .unwrap();
    assert!(result.limited);
    assert_eq!(result.threads[0].comments.len(), 21);
    assert_eq!(result.threads[0].comments[0].id, "C1");
    assert_eq!(result.threads[0].comments[20].id, "C30");
    node["recent"]["totalCount"] = json!(31);
    assert!(
        collect(PR, &"a".repeat(40), |_| Ok(response(
            vec![node.clone()],
            None
        )))
        .is_err()
    );
}

#[test]
fn review_threads_do_not_accept_partial_errors_wrong_pr_or_changed_heads() {
    let valid: Value = serde_json::from_str(&response(vec![node(1)], None)).unwrap();
    for (path, value) in [
        ("headRefOid", json!("b".repeat(40))),
        ("url", json!("https://github.com/other/repo/pull/7")),
    ] {
        let mut wrong = valid.clone();
        wrong["data"]["repository"]["pullRequest"][path] = value;
        assert!(page(&wrong.to_string(), PR, &"a".repeat(40)).is_err());
    }
    let mut partial = valid.clone();
    partial["errors"] = json!([{"message":"permission denied"}]);
    assert!(page(&partial.to_string(), PR, &"a".repeat(40)).is_err());
    for json in ["{}", "null", r#"{"data":{"repository":null}}"#] {
        assert!(page(json, PR, &"a".repeat(40)).is_err());
    }
    let mut calls = 0;
    assert!(
        collect(PR, &"a".repeat(40), |_| {
            calls += 1;
            if calls == 1 {
                Ok(response(vec![node(1)], Some("next")))
            } else {
                anyhow::bail!("network failed")
            }
        })
        .is_err()
    );
}

#[test]
fn review_threads_pagination_is_bounded_and_rejects_repeated_cursors_or_ids() {
    let mut calls = 0;
    let result = collect(PR, &"a".repeat(40), |cursor| {
        assert_eq!(
            cursor.map(str::to_owned),
            (calls > 0).then(|| format!("page-{calls}"))
        );
        let start = calls * 25;
        calls += 1;
        Ok(response(
            (start..start + 25).map(node).collect(),
            Some(&format!("page-{calls}")),
        ))
    })
    .unwrap();
    assert_eq!(calls, 8);
    assert_eq!(result.threads.len(), 200);
    assert!(result.limited);
    assert!(
        collect(PR, &"a".repeat(40), |_| Ok(response(
            vec![node(1)],
            Some("repeat")
        )))
        .is_err()
    );
    let mut i = 0;
    assert!(
        collect(PR, &"a".repeat(40), |_| {
            i += 1;
            Ok(response(vec![node(i)], Some("repeat")))
        })
        .is_err()
    );
    assert!(collect(PR, &"a".repeat(40), |_| Ok(response(vec![], Some("next")))).is_err());
    assert!(
        collect(PR, &"a".repeat(40), |_| Ok(response(
            (0..26).map(node).collect(),
            None
        )))
        .is_err()
    );
}

#[test]
fn review_threads_validate_repository_links_content_and_missing_evidence() {
    for url in [
        "file:///repo",
        "https://user@github.com/example/repo/pull/7",
        "https://github.com/example/repo/pull/0",
        "https://github.com/example/repo/pull/7?x=y",
        "https://github.com/example/repo/pull/7/files",
        "https://github.com/example/repo/pull/2147483648",
    ] {
        assert!(identity(url).is_err(), "{url}");
    }
    let enterprise = identity("https://git.example.com/org/repo/pull/7").unwrap();
    assert_eq!(enterprise.host, "git.example.com");
    for (field, value) in [
        ("url", json!("https://elsewhere.example/payload")),
        ("url", json!("file:///secret")),
        ("body", json!("x".repeat(64 * 1024 + 1))),
        ("diffHunk", json!("x".repeat(64 * 1024 + 1))),
        ("updatedAt", json!("invalid")),
        ("commit", json!({"oid":"bad"})),
        ("state", json!("UNKNOWN")),
    ] {
        let mut node = node(1);
        node["first"]["nodes"][0][field] = value.clone();
        node["recent"]["nodes"][0][field] = value;
        assert!(thread(&node, PR).is_err(), "{field}");
    }
    let mut unavailable = node(1);
    for part in ["first", "recent"] {
        unavailable[part]["nodes"][0]["author"] = Value::Null;
        unavailable[part]["nodes"][0]["originalCommit"] = Value::Null;
        unavailable[part]["nodes"][0]["state"] = json!("PENDING");
    }
    assert!(thread(&unavailable, PR).is_ok());
    assert!(collect(PR, &"a".repeat(40), |_| Ok(" ".repeat(MAX_BYTES + 1))).is_err());
    let mut oversized = snapshot().threads.remove(0);
    oversized.comments[0].body = "x".repeat(crate::review::MAX_FEEDBACK_BYTES);
    assert!(oversized.feedback(PR, &"a".repeat(40)).is_err());
}
