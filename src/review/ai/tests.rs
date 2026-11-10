use super::*;
use serde_json::{Value, json};

const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -10,3 +10,4 @@\n keep\n-old\n+new\n+second\n end\n@@ -30 +31 @@\n-tail\n+last\n";

fn request() -> Review {
    let mut review = Review::capture(
        Target {
            workspace: PathBuf::from("C:/repo"),
            path: "a.rs".into(),
            source: "unstaged".into(),
        },
        PATCH,
    )
    .unwrap();
    review.session = 42;
    review.session_created_at = 123;
    review
}

fn finding() -> Value {
    json!({"path":"a.rs","side":"new","start_line":11,"end_line":12,
        "priority":"P1","title":"Check empty input","body":"Empty input can panic. Verify an empty-input case."})
}

fn report(review: &Review, findings: Vec<Value>) -> Value {
    json!({"version":1,"review_id":review.key,"fingerprint":review.fingerprint,"findings":findings})
}

fn rejection(review: &Review, value: Value) -> Rejection {
    parse(review, &value.to_string())
        .err()
        .expect("must reject report")
}

#[test]
fn exact_snapshots_preserve_sides_ranges_and_accept_only_entire_json() {
    let review = request();
    let mut old = finding();
    old["side"] = json!("old");
    old["end_line"] = json!(11);
    let text = report(&review, vec![finding(), old]).to_string();
    for text in [
        text.clone(),
        format!("```json\r\n{text}\r\n```"),
        format!("```conduit-review\n{text}\n```"),
    ] {
        let found = parse(&review, &text).unwrap();
        assert_eq!(found[0].anchor.snippet, "new\nsecond");
        assert_eq!(found[1].anchor.snippet, "old");
        assert_eq!(found[1].anchor.side, Side::Original);
        assert_eq!(found[0].anchor.fingerprint, fingerprint(PATCH));
    }
    for text in [
        format!("Here is the report: {text}"),
        format!("```json\n{text}\n```\nDone"),
        format!("```js\n{text}\n```"),
        "{}\n{}".into(),
    ] {
        assert_eq!(parse(&review, &text).err(), Some(Rejection::Format));
    }
}

#[test]
fn mismatched_identity_schema_and_captured_content_are_rejected() {
    let review = request();
    for (field, value) in [
        ("version", json!(2)),
        ("review_id", json!("other")),
        ("fingerprint", json!("wrong")),
    ] {
        let mut output = report(&review, vec![finding()]);
        output[field] = value;
        assert_eq!(rejection(&review, output), Rejection::Snapshot);
    }
    let output = report(&review, vec![finding()]);
    let mut changed = review.clone();
    changed.patch.push_str("changed");
    assert_eq!(rejection(&changed, output), Rejection::Snapshot);
    for (field, value) in [
        ("side", json!("right")),
        ("priority", json!("P4")),
        ("start_line", json!(-1)),
        ("extra", json!(true)),
    ] {
        let mut invalid = finding();
        invalid[field] = value;
        assert_eq!(
            rejection(&review, report(&review, vec![invalid])),
            Rejection::Format
        );
    }
    let mut unknown = report(&review, vec![]);
    unknown["passed"] = json!(true);
    assert_eq!(rejection(&review, unknown), Rejection::Format);
}

#[test]
fn absent_lines_sparse_hunks_and_invalid_finding_content_fail_atomically() {
    let review = request();
    for (start, end) in [(0, 1), (12, 11), (1, 1), (11, 31), (100, 100), (1, 201)] {
        let mut bad = finding();
        bad["start_line"] = json!(start);
        bad["end_line"] = json!(end);
        assert_eq!(
            rejection(&review, report(&review, vec![finding(), bad])),
            Rejection::Range(2)
        );
    }
    for (field, value) in [
        ("path", "../a.rs".into()),
        ("path", "other.rs".into()),
        ("title", "  ".into()),
        ("title", "bad\ntitle".into()),
        ("title", "x".repeat(257)),
        ("body", " ".into()),
        ("body", "x".repeat(MAX_COMMENT_BYTES + 1)),
    ] {
        let mut bad = finding();
        bad[field] = json!(value);
        let text = report(&review, vec![finding(), bad]).to_string();
        let mut reviews = Reviews {
            ai: vec![review.clone()],
            ..Default::default()
        };
        reviews.accept_ai(&review.key, Some("reply".into()), Ok(&text));
        assert!(reviews.comments.is_empty());
        assert!(matches!(
            reviews.ai[0].state,
            ReportState::Rejected(Rejection::Finding(2))
        ));
    }
}

#[test]
fn imports_are_deduplicated_persistent_suggestions_and_do_not_reopen_resolved_notes() {
    let review = request();
    let text = report(&review, vec![finding(), finding()]).to_string();
    let mut reviews = Reviews {
        ai: vec![review.clone()],
        ..Default::default()
    };
    reviews.set_viewed(review.target.clone(), review.fingerprint.clone(), true);
    reviews
        .accept_ai(&review.key, Some("turn-final".into()), Ok(&text))
        .unwrap();
    assert_eq!(reviews.comments.len(), 1);
    let note = &reviews.comments[0];
    assert_eq!(note.status, Status::NeedsReview);
    let origin = note.ai.as_ref().unwrap();
    assert_eq!((origin.session, origin.session_created_at), (42, 123));
    assert_eq!(origin.review, review.key);
    assert_eq!(origin.message, "turn-final");
    assert_eq!(origin.priority, Priority::P1);
    assert!(note.display_text().contains("Check empty input"));
    assert!(reviews.is_viewed(&review.target, &review.fingerprint));
    assert!(!reviews.is_viewed(&review.target, &fingerprint("changed")));
    let feedback = reviews
        .feedback(&review.target, &[note.id], "changed")
        .unwrap();
    assert!(feedback.contains("Check empty input"));
    assert!(feedback.contains("    new\n    second"));
    reviews.comments[0].status = Status::Resolved;
    let mut restored: Reviews =
        serde_json::from_str(&serde_json::to_string(&reviews).unwrap()).unwrap();
    assert!(
        restored
            .accept_ai(&review.key, Some("other".into()), Ok(&text))
            .is_none()
    );
    assert_eq!(restored.comments.len(), 1);
    assert_eq!(restored.comments[0].status, Status::Resolved);
    assert_eq!(restored.ai[0].message.as_deref(), Some("turn-final"));
}

#[test]
fn empty_report_and_failed_reports_never_mark_viewed_and_failures_can_be_corrected() {
    let review = request();
    let mut reviews = Reviews {
        ai: vec![review.clone()],
        ..Default::default()
    };
    reviews.accept_ai(&review.key, None, Err(Rejection::Incomplete));
    assert!(matches!(
        reviews.ai[0].state,
        ReportState::Rejected(Rejection::Incomplete)
    ));
    reviews.accept_ai(
        &review.key,
        Some("reply".into()),
        Ok(&report(&review, vec![]).to_string()),
    );
    assert!(matches!(reviews.ai[0].state, ReportState::NoFindings));
    assert!(reviews.comments.is_empty());
    assert!(reviews.viewed.is_empty());
}

#[test]
fn count_byte_and_id_limits_reject_the_whole_batch() {
    let review = request();
    assert_eq!(
        parse(&review, &" ".repeat(MAX_REPLY_BYTES + 1)).err(),
        Some(Rejection::TooLarge)
    );
    assert_eq!(
        rejection(&review, report(&review, vec![finding(); 101])),
        Rejection::TooLarge
    );
    let mut reviews = Reviews {
        ai: vec![review.clone()],
        next_id: u64::MAX,
        ..Default::default()
    };
    let text = report(&review, vec![finding()]).to_string();
    reviews.accept_ai(&review.key, None, Ok(&text));
    assert!(reviews.comments.is_empty());
    assert!(matches!(
        reviews.ai[0].state,
        ReportState::Rejected(Rejection::Storage)
    ));
    reviews.next_id = 0;
    reviews
        .add(
            review.target.clone(),
            Anchor::capture(PATCH, Side::Modified, 11, 11).unwrap(),
            "manual",
        )
        .unwrap();
    reviews.comments = vec![reviews.comments[0].clone(); MAX_COMMENTS];
    reviews.accept_ai(&review.key, None, Ok(&text));
    assert_eq!(reviews.comments.len(), MAX_COMMENTS);
    assert!(reviews.comments.iter().all(|c| c.ai.is_none()));
    assert!(matches!(
        reviews.ai[0].state,
        ReportState::Rejected(Rejection::Storage)
    ));
}

#[test]
fn capture_capacity_and_old_saved_comments_remain_compatible() {
    let review = request();
    assert_ne!(review.key, request().key);
    assert!(Review::capture(review.target.clone(), "").is_err());
    assert!(Review::capture(review.target.clone(), &"x".repeat(MAX_FEEDBACK_BYTES + 1)).is_err());
    let mut reviews = Reviews::default();
    assert!(reviews.can_capture_ai(MAX_PATCH_BYTES).is_ok());
    assert!(reviews.can_capture_ai(MAX_PATCH_BYTES + 1).is_err());
    reviews.ai = vec![review.clone(); MAX_REPORTS];
    assert!(reviews.can_capture_ai(1).is_err());
    reviews.ai.truncate(1);
    assert!(reviews.can_capture_ai(MAX_PATCH_BYTES).is_err());
    let legacy = json!({"comments":[{"id":1,"target":review.target,
        "anchor":Anchor::capture(PATCH, Side::Modified, 11, 11).unwrap(),"text":"manual","status":"Open"}]});
    let restored: Reviews = serde_json::from_value(legacy).unwrap();
    assert!(restored.ai.is_empty());
    assert!(restored.comments[0].ai.is_none());
    assert_eq!(restored.comments[0].display_text(), "manual");
}
