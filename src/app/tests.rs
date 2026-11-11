use super::{
    apply_event, finish_tool_cards, highlight_ranges, latest_naming_excerpt, same_failure,
    search_snippet, single_line_title,
};
use crate::{
    domain::{ChatItem, ItemKind, PermissionLevel, Provider, SessionRecord, Usage},
    protocol::Event,
};

#[test]
fn error_retry_clears_errors_and_preserves_partial_output_across_restore() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    let user = ChatItem::from(crate::domain::ChatItemData {
        reasoning_summary: None,
        elapsed_ms: None,
        key: "turn-0-user".into(),
        created_at: 0,
        kind: ItemKind::User,
        text: "检查项目并说明结果".into(),
        input: String::new(),
        detail: String::new(),
        status: Default::default(),
        approval: None,
        model: None,
    });
    s.messages.push(user);
    apply_event(
        &mut s,
        "turn-0",
        Event::Delta {
            key: "reply".into(),
            text: "已经检查了配置。".into(),
        },
    );
    apply_event(
        &mut s,
        "turn-0",
        Event::Tool {
            key: "tool".into(),
            title: "读取文件".into(),
            detail: "配置内容".into(),
            status: "已完成".into(),
        },
    );
    apply_event(
        &mut s,
        "turn-0",
        Event::ErrorBanner("API Error: 503 unavailable".into()),
    );
    let original = serde_json::to_value(&s.messages[..3]).unwrap();
    assert_eq!(
        super::runtime::error_retry_prompt(&s, 3),
        Some("检查项目并说明结果")
    );
    super::runtime::clear_retry_errors(&mut s);
    assert_eq!(serde_json::to_value(&s.messages).unwrap(), original);

    // Retry output uses a new turn prefix, even when the CLI reuses its item keys.
    let prefix = super::runtime::next_turn_prefix(&s);
    apply_event(
        &mut s,
        &prefix,
        Event::Delta {
            key: "reply".into(),
            text: "接着检查代码。".into(),
        },
    );
    apply_event(
        &mut s,
        &prefix,
        Event::ErrorBanner("API Error: 503 unavailable".into()),
    );
    assert_eq!(serde_json::to_value(&s.messages[..3]).unwrap(), original);
    assert_eq!(
        super::runtime::error_retry_prompt(&s, 4),
        Some("检查项目并说明结果")
    );
    assert_eq!(super::runtime::error_retry_prompt(&s, 3), None);

    let mut restored: SessionRecord =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    super::runtime::clear_retry_errors(&mut restored);
    let prefix = super::runtime::next_turn_prefix(&restored);
    apply_event(
        &mut restored,
        &prefix,
        Event::Delta {
            key: "reply".into(),
            text: "检查完成。".into(),
        },
    );
    assert_eq!(
        serde_json::to_value(&restored.messages[..3]).unwrap(),
        original
    );
    assert_eq!(restored.messages[3].text, "接着检查代码。");
    assert_eq!(restored.messages[4].text, "检查完成。");
    assert_eq!(
        restored
            .messages
            .iter()
            .filter(|m| m.kind == ItemKind::User)
            .count(),
        1
    );
}

#[test]
fn error_retry_only_clears_errors_after_the_last_visible_reply() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    s.messages = serde_json::from_value(serde_json::json!([
        {"key": "old-user", "kind": "User", "text": "old question"},
        {"key": "old-error", "kind": "Error", "text": "old error"},
        {"key": "reply", "kind": "Assistant", "text": "last visible reply"},
        {"key": "new-user", "kind": "User", "text": "new question"},
        {"key": "error-1", "kind": "Error", "text": "first failure"},
        {"key": "notice", "kind": "Notice", "text": "retrying"},
        {"key": "empty", "kind": "Assistant", "text": "  "},
        {"key": "tool", "kind": "Tool", "text": "read file"},
        {"key": "error-2", "kind": "Error", "text": "second failure"}
    ]))
    .unwrap();
    let original = serde_json::to_value(&s.messages).unwrap();
    let removed = super::runtime::clear_retry_errors(&mut s);
    assert_eq!(
        s.messages
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        [
            "old-user",
            "old-error",
            "reply",
            "new-user",
            "notice",
            "empty",
            "tool"
        ]
    );
    assert_eq!(
        removed.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        [4, 8]
    );
    for (index, item) in removed {
        s.messages.insert(index, item);
    }
    assert_eq!(serde_json::to_value(&s.messages).unwrap(), original);
}

#[test]
fn error_retry_clears_repeated_failures_before_any_reply() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    s.messages = serde_json::from_value(serde_json::json!([
        {"key": "turn-0-user", "kind": "User", "text": "question"},
        {"key": "turn-0-notice", "kind": "Notice", "text": "starting"}
    ]))
    .unwrap();
    let original = serde_json::to_value(&s.messages).unwrap();
    for _ in 0..3 {
        let prefix = super::runtime::next_turn_prefix(&s);
        for error in ["API Error: 503 unavailable", "API Error: 429 rate limit"] {
            apply_event(&mut s, &prefix, Event::ErrorBanner(error.into()));
        }
        assert_eq!(s.messages.len(), 4);
        assert_eq!(super::runtime::error_retry_prompt(&s, 3), Some("question"));
        assert_eq!(super::runtime::clear_retry_errors(&mut s).len(), 2);
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), original);
    }
}

#[test]
fn error_retry_only_targets_a_failure_in_the_latest_exchange() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    apply_event(
        &mut s,
        "turn-0",
        Event::ErrorBanner("API Error: 503".into()),
    );
    assert_eq!(super::runtime::error_retry_prompt(&s, 0), None);
    assert_eq!(super::runtime::error_retry_prompt(&s, usize::MAX), None);
    for (kind, text) in [
        (ItemKind::User, "新提问"),
        (ItemKind::Error, "API Error: 503"),
    ] {
        let mut item = s.messages[0].clone();
        item.kind = kind;
        item.text = text.into();
        s.messages.push(item);
    }
    assert_eq!(super::runtime::error_retry_prompt(&s, 0), None);
    assert_eq!(super::runtime::error_retry_prompt(&s, 1), None);
    assert_eq!(super::runtime::error_retry_prompt(&s, 2), Some("新提问"));
}

#[test]
fn opencode_diff_content_is_classified_and_bounded_for_the_existing_edit_view() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    apply_event(&mut s, "turn-0", Event::Tool {
        key: "edit-1".into(), title: "update configuration".into(), status: "已完成".into(),
        detail: serde_json::json!({"acpKind":"edit","content":[{"type":"diff","path":"settings.rs","oldText":"old setting","newText":"new setting"}]}).to_string(),
    });
    let item = &s.messages[0];
    assert!(super::is_tool_edit(item));
    let diffs = super::edit_tool_diffs(item).unwrap();
    assert_eq!(diffs[0].path, "settings.rs");
    assert_eq!((diffs[0].added, diffs[0].removed), (1, 1));
    let mut long = item.clone();
    long.detail = serde_json::json!({"acpKind":"edit","content":[{"type":"diff","path":"large.rs","oldText":"","newText":"line\n".repeat(1000)}]}).to_string();
    let diffs = super::edit_tool_diffs(&long).unwrap();
    // Two file headers plus one hunk header wrap the bounded body.
    assert_eq!(
        diffs[0].patch.lines().count(),
        super::MAX_EDIT_DIFF_LINES + 2
    );
    assert!(diffs[0].truncated);
}
#[test]
fn assistant_reply_combines_text_across_tools_within_user_boundaries() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (key, kind, text) in [
        ("old", ItemKind::Assistant, "导入的早期回复"),
        ("user", ItemKind::User, "提交修改"),
        ("before", ItemKind::Assistant, "我会提交并推送。"),
        ("tool", ItemKind::Tool, "exec_command"),
        ("reasoning", ItemKind::Reasoning, "检查结果"),
        ("empty", ItemKind::Assistant, " \n"),
        ("final", ItemKind::Assistant, "已提交并推送。"),
        ("notice", ItemKind::Notice, "提示"),
        ("next-user", ItemKind::User, "继续"),
        ("next", ItemKind::Assistant, "下一轮回复"),
    ] {
        s.messages.push(ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: key.into(),
            created_at: 0,
            kind,
            text: text.into(),
            input: String::new(),
            detail: String::new(),
            status: Default::default(),
            approval: None,
            model: None,
        }));
    }
    for index in [2, 6] {
        assert_eq!(super::chat::assistant_reply_range(&s, index), 2..8);
        assert_eq!(
            super::chat::assistant_reply_text(&s, index),
            "我会提交并推送。\n\n已提交并推送。"
        );
    }
    assert_eq!(super::chat::assistant_reply_text(&s, 0), "导入的早期回复");
    assert_eq!(super::chat::assistant_reply_text(&s, 9), "下一轮回复");
}

#[test]
fn indexed_events_keep_order_inputs_and_stream_snapshots_across_batches() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    let mut index = super::MessageIndex::default();
    for ordinal in 0..500 {
        super::apply_event_indexed(
            &mut s,
            "turn-1",
            Event::ToolInput {
                key: ordinal.to_string(),
                input: format!("input-{ordinal}"),
            },
            &mut index,
        );
    }
    for ordinal in 0..500 {
        super::apply_event_indexed(
            &mut s,
            "turn-1",
            Event::Tool {
                key: ordinal.to_string(),
                title: "Bash".into(),
                detail: "result".into(),
                status: "完成".into(),
            },
            &mut index,
        );
    }
    assert_eq!(s.messages.len(), 500);
    for (ordinal, item) in s.messages.iter().enumerate() {
        assert_eq!(item.key, format!("turn-1-{ordinal}"));
        assert_eq!(item.input, format!("input-{ordinal}"));
        assert_eq!(item.detail, "result");
    }
    // Approval handling appends messages outside the event index.
    s.record_tool_approval(
        "turn-1",
        "ticket",
        "请求使用 Write",
        r#"{"tool_use_id":"write"}"#,
        "等待中",
    );
    super::apply_event_indexed(
        &mut s,
        "turn-1",
        Event::ToolInput {
            key: "write".into(),
            input: "payload".into(),
        },
        &mut index,
    );
    assert_eq!(s.messages.len(), 501);
    assert!(s.messages[500].approval.is_some());
    for text in ["first", " second"] {
        super::apply_event_indexed(
            &mut s,
            "turn-1",
            Event::Delta {
                key: "reply".into(),
                text: text.into(),
            },
            &mut index,
        );
    }
    super::apply_event_indexed(
        &mut s,
        "turn-1",
        Event::Text {
            key: "reply".into(),
            text: "complete snapshot".into(),
        },
        &mut index,
    );
    assert_eq!(s.messages.last().unwrap().text, "complete snapshot");
}
#[test]
fn message_outline_keeps_real_indices_and_keys_amid_tools_and_replies() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (key, kind, text) in [
        ("user-1", ItemKind::User, "第一轮\n  修复\tbug"),
        ("tool-1", ItemKind::Tool, "cargo check"),
        ("assistant-1", ItemKind::Assistant, "完成"),
        ("user-2", ItemKind::User, "第一轮\n  修复\tbug"),
        ("notice", ItemKind::Notice, "notice"),
        ("user-3", ItemKind::User, ""),
    ] {
        s.messages.push(ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: key.into(),
            created_at: 0,
            kind,
            text: text.into(),
            input: String::new(),
            detail: String::new(),
            status: Default::default(),
            approval: None,
            model: None,
        }));
    }
    let entries = super::message_outline_entries(&s, 0);
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.index, entry.key.as_str()))
            .collect::<Vec<_>>(),
        vec![(0, "user-1"), (3, "user-2"), (5, "user-3")]
    );
    assert_eq!(entries[0].label, "第一轮 修复 bug");
    assert_eq!(entries[2].label, "（空消息）");
    s.messages[0].text = "中🙂".repeat(90);
    let entries = super::message_outline_entries(&s, 0);
    assert_eq!(entries[0].label.chars().count(), 101);
    assert!(entries[0].label.ends_with('…'));
}

#[test]
fn message_outline_cache_only_indexes_appends_and_resets_with_session() {
    let _locale = crate::test_support::chinese();
    let mut session = session();
    let mut cache = super::MessageOutlineCache::default();
    let item = |key: &str, kind| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: key.into(),
            created_at: 0,
            kind,
            text: "用户消息".into(),
            input: String::new(),
            detail: String::new(),
            status: Default::default(),
            approval: None,
            model: None,
        })
    };
    session.messages.push(item("user-1", ItemKind::User));
    cache.refresh(Some(&session));
    let label = cache.entries[0].label.as_ptr();
    cache.refresh(Some(&session));
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.entries[0].label.as_ptr(), label);
    session.messages.push(item("reply", ItemKind::Assistant));
    session.messages.push(item("user-2", ItemKind::User));
    cache.refresh(Some(&session));
    assert_eq!(cache.entries.len(), 2);
    assert_eq!(cache.entries[1].index, 2);
    session.messages.truncate(1);
    cache.refresh(Some(&session));
    assert_eq!(cache.entries.len(), 1);
    session.id += 1;
    cache.refresh(Some(&session));
    assert_eq!(cache.session, Some(session.id));
    cache.refresh(None);
    assert!(cache.entries.is_empty());
}

#[test]
fn tool_text_preserves_literal_logs_code_and_removes_terminal_escapes() {
    let _locale = crate::test_support::chinese();
    let text = "\u{1b}[31m错误\u{1b}[0m\r\n\tcode();\rretry\n\u{1b}]8;;https://example.test\u{1b}\\label\u{1b}]8;;\u{7}\n<b>literal</b> **text**";
    assert_eq!(
        super::format_tool_text(text),
        "错误\n    code();\nretry\nlabel\n<b>literal</b> **text**"
    );
    let json = "{\"command\":\"echo 中文\",\"args\":[1,2]}";
    let formatted = super::format_tool_text(json);
    assert!(formatted.contains('\n'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&formatted).unwrap(),
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
    // A partial streaming JSON result remains useful text, never discarded.
    assert_eq!(
        super::format_tool_text("{\"output\":\"partial"),
        "{\"output\":\"partial"
    );
}
#[test]
fn tool_approval_matches_provider_id_and_survives_execution_updates() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for key in ["first", "second"] {
        apply_event(
            &mut s,
            "turn-1",
            Event::Tool {
                key: key.into(),
                title: "same command".into(),
                detail: String::new(),
                status: "运行中".into(),
            },
        );
    }
    let detail = r#"{"itemId":"first"}"#;
    s.record_tool_approval("turn-1", "ticket", "请求执行命令", detail, "等待中");
    s.record_tool_approval("turn-1", "ticket", "请求执行命令", detail, "已允许");
    apply_event(
        &mut s,
        "turn-1",
        Event::Tool {
            key: "first".into(),
            title: "same command".into(),
            detail: "done".into(),
            status: "completed".into(),
        },
    );
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].approval.as_ref().unwrap().status, "已允许");
    assert_eq!(s.messages[0].status, "completed");
    assert!(s.messages[1].approval.is_none());
    assert!(s.messages.iter().all(|item| item.kind == ItemKind::Tool));
}
#[test]
fn permission_before_tool_start_merges_and_pending_cancellation_is_scoped() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for prefix in ["turn-0", "turn-1"] {
        s.record_tool_approval(
            prefix,
            "ticket",
            "请求使用 WebSearch",
            r#"{"tool_use_id":"search"}"#,
            "等待中",
        );
        apply_event(
            &mut s,
            prefix,
            Event::Tool {
                key: "search".into(),
                title: "WebSearch".into(),
                detail: String::new(),
                status: "运行中".into(),
            },
        );
    }
    finish_tool_cards(&mut s, "turn-1", "已取消");
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].approval.as_ref().unwrap().status, "等待中");
    assert_eq!(s.messages[1].approval.as_ref().unwrap().status, "已取消");
    s.record_tool_approval(
        "turn-1",
        "ticket",
        "请求使用 WebSearch",
        r#"{"tool_use_id":"search"}"#,
        "已拒绝",
    );
    assert_eq!(s.messages[1].approval.as_ref().unwrap().status, "已拒绝");
}
#[test]
fn claude_approval_without_id_matches_input_and_never_another_turn() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (prefix, key, query) in [
        ("old", "a", "one"),
        ("now", "b", "one"),
        ("now", "c", "two"),
    ] {
        apply_event(
            &mut s,
            prefix,
            Event::Tool {
                key: key.into(),
                title: "WebSearch".into(),
                detail: String::new(),
                status: "运行中".into(),
            },
        );
        apply_event(
            &mut s,
            prefix,
            Event::ToolInput {
                key: key.into(),
                input: format!(r#"{{"query":"{query}"}}"#),
            },
        );
    }
    s.record_tool_approval(
        "now",
        "request",
        "请求使用 WebSearch",
        r#"{"input":{"query":"one"}}"#,
        "已允许",
    );
    assert_eq!(s.messages.len(), 3);
    assert!(s.messages[0].approval.is_none());
    assert_eq!(s.messages[1].approval.as_ref().unwrap().status, "已允许");
    assert!(s.messages[2].approval.is_none());
}
#[test]
fn commit_generation_snapshot_replaces_stream_without_duplicate_text() {
    let _locale = crate::test_support::chinese();
    let mut parts = vec![];
    super::update_commit_text(&mut parts, "msg".into(), "feat: ".into(), false);
    super::update_commit_text(&mut parts, "msg".into(), "说明".into(), false);
    super::update_commit_text(&mut parts, "msg".into(), "feat: 说明".into(), true);
    super::update_commit_text(&mut parts, "body".into(), "原因".into(), true);
    assert_eq!(
        parts,
        vec![
            ("msg".into(), "feat: 说明".into()),
            ("body".into(), "原因".into())
        ]
    );
}
pub(crate) fn session() -> SessionRecord {
    SessionRecord {
        branching: Default::default(),
        transcript: Default::default(),
        id: 1,
        provider: Provider::Claude,
        created_at: 0,
        exit_code: None,
        title: String::new(),
        archived: false,
        pinned: false,
        native_id: None,
        model: None,
        messages: vec![],
        interrupted: false,
        channel: Default::default(),
        reasoning_effort: Default::default(),
        opencode: Default::default(),
        permission: PermissionLevel::default(),
        usage: Usage::default(),
    }
}

#[test]
fn composer_defaults_to_medium_and_preserves_explicit_effort_until_model_changes() {
    let _locale = crate::test_support::chinese();
    for engine in [Provider::Claude, Provider::Codex] {
        let mut selection = super::ComposerSelection::new(engine);
        assert_eq!(selection.effort, super::ReasoningEffort::Medium);
        selection.set_model(Some("model-a".into()));
        selection.effort = super::ReasoningEffort::High;
        selection.set_model(Some("model-a".into()));
        assert_eq!(selection.effort, super::ReasoningEffort::High);
        selection.set_model(Some("model-b".into()));
        assert_eq!(selection.effort, super::ReasoningEffort::Medium);
        selection.effort = super::ReasoningEffort::Default;
        let mut session = session();
        session.provider = engine;
        selection.apply(&mut session);
        assert_eq!(
            super::ComposerSelection::from_session(&session).effort,
            super::ReasoningEffort::Default,
            "explicit CLI defaults survive restoration"
        );
    }
}
#[test]
fn claude_thinking_stream_and_snapshot_restore_without_duplicate_reply_or_opaque_data() {
    let _locale = crate::test_support::chinese();
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Claude, "C:/work".into(), "hello".into(), None);
    let mut s = session();
    let events = [
        serde_json::json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m"}}}),
        serde_json::json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"Checking"}}}),
        serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":" the code"}}}),
        serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-signature"}}}),
        serde_json::json!({"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text"}}}),
        serde_json::json!({"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Done"}}}),
        serde_json::json!({"type":"assistant","message":{"id":"m","content":[
            {"type":"thinking","thinking":"Checking the code","signature":"opaque-signature"},
            {"type":"redacted_thinking","data":"opaque-data"},
            {"type":"text","text":"Done"}
        ]}}),
    ];
    for value in events {
        for event in protocol.accept(value).events {
            apply_event(&mut s, "turn", event);
        }
    }
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].kind, ItemKind::Reasoning);
    assert_eq!(s.messages[0].text, "Checking the code");
    assert_eq!(s.messages[1].kind, ItemKind::Assistant);
    assert_eq!(s.messages[1].text, "Done");
    let saved = serde_json::to_string(&s.messages).unwrap();
    assert!(!saved.contains("opaque-"));
    let restored: Vec<ChatItem> = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored[0].text, "Checking the code");
    assert_eq!(restored[0].key, s.messages[0].key);
}

#[test]
fn contiguous_tool_groups_keep_reasoning_independently_visible() {
    let _locale = crate::test_support::chinese();
    let activity = |key: &str, kind: ItemKind, title: &str, status: &str| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: key.into(),
            created_at: 0,
            kind,
            text: title.into(),
            input: String::new(),
            detail: String::new(),
            status: status.into(),
            approval: None,
            model: None,
        })
    };
    let mut s = session();
    s.messages = vec![
        activity("a", ItemKind::Tool, "exec_command", "completed"),
        activity("b", ItemKind::Tool, "WebSearch", "completed"),
        activity("c", ItemKind::Reasoning, "thinking", ""),
        activity("d", ItemKind::Tool, "other", "completed"),
    ];
    // Reasoning is a transcript item in its own right. It must remain visible
    // after the following assistant reply instead of being swallowed by the
    // completed tool group.
    assert_eq!(super::contiguous_tool_group(&s, 1), Some((0, 2)));
    assert_eq!(super::contiguous_tool_group(&s, 2), None);
    assert_eq!(super::contiguous_tool_group(&s, 3), Some((3, 4)));

    s.messages[1].status = "运行中".into();
    assert_eq!(super::contiguous_tool_group(&s, 0), Some((0, 2)));

    s.messages[1].status = "completed".into();
    s.messages
        .push(activity("reply", ItemKind::Assistant, "done", ""));
    let mut rows = super::chat::rows::Rows::default();
    rows.refresh(Some(&s), &Default::default());
    assert_eq!(
        rows.rows.iter().map(|row| row.message).collect::<Vec<_>>(),
        vec![0, 2, 3, 4]
    );
    assert!(
        rows.group(2).is_none(),
        "thinking remains visible after tool groups collapse"
    );
    assert_eq!(rows.group(0).unwrap().count, 2);
    assert!(!rows.group(0).unwrap().expanded);
}
#[test]
fn empty_stream_blocks_do_not_split_tools_or_count_as_replies() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    apply_event(
        &mut s,
        "turn",
        Event::Text {
            key: "before".into(),
            text: " \n".into(),
        },
    );
    for (key, title) in [("a", "Read"), ("b", "exec_command")] {
        apply_event(
            &mut s,
            "turn",
            Event::Tool {
                key: key.into(),
                title: title.into(),
                detail: String::new(),
                status: "completed".into(),
            },
        );
        apply_event(
            &mut s,
            "turn",
            Event::Text {
                key: format!("after-{key}"),
                text: String::new(),
            },
        );
    }
    assert_eq!(super::contiguous_tool_group(&s, 1), Some((1, 4)));
    assert_eq!(super::contiguous_tool_group(&s, 3), Some((1, 4)));
    assert!(!super::tool_group_has_reply(&s, 4));
    assert!(super::is_empty_chat_placeholder(&s.messages[0]));
    assert!(super::is_empty_chat_placeholder(&s.messages[2]));

    // A real streamed reply keeps its original key/index and becomes a real
    // group boundary as soon as it contains text.
    apply_event(
        &mut s,
        "turn",
        Event::Delta {
            key: "after-a".into(),
            text: "读取完成".into(),
        },
    );
    assert_eq!(s.messages.len(), 5);
    assert_eq!(s.messages[2].text, "读取完成");
    assert!(!super::is_empty_chat_placeholder(&s.messages[2]));
    assert_eq!(super::contiguous_tool_group(&s, 1), Some((1, 2)));
    assert_eq!(super::contiguous_tool_group(&s, 3), Some((3, 4)));

    s.messages[2].text.clear();
    s.messages[2].kind = ItemKind::User;
    assert_eq!(super::contiguous_tool_group(&s, 1), Some((1, 2)));
    s.messages[2].kind = ItemKind::Tool;
    s.messages[2].text = "Edit".into();
    assert_eq!(super::contiguous_tool_group(&s, 1), Some((1, 2)));
    s.messages[2].text = "Read".into();
    s.messages[2].approval = Some(crate::domain::ToolApproval {
        request: "approval".into(),
        status: "等待中".into(),
    });
    assert_eq!(super::contiguous_tool_group(&s, 1), Some((1, 2)));
}

#[test]
fn tool_group_waits_through_completed_tools_until_nonempty_reply_in_same_turn() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (key, status) in [
        ("a", "运行中"),
        ("a", "completed"),
        ("b", "运行中"),
        ("b", "completed"),
    ] {
        apply_event(
            &mut s,
            "turn",
            Event::Tool {
                key: key.into(),
                title: "Read".into(),
                detail: String::new(),
                status: status.into(),
            },
        );
        assert!(!super::tool_group_has_reply(&s, s.messages.len()));
    }
    let end = s.messages.len();
    apply_event(
        &mut s,
        "turn",
        Event::Text {
            key: "reply".into(),
            text: String::new(),
        },
    );
    assert!(!super::tool_group_has_reply(&s, end));
    apply_event(
        &mut s,
        "turn",
        Event::Delta {
            key: "reply".into(),
            text: "检查完成".into(),
        },
    );
    assert!(super::tool_group_has_reply(&s, end));

    // A stopped/failed turn without a reply remains inspectable even when
    // the next turn eventually produces an answer.
    s.messages[end].kind = ItemKind::User;
    apply_event(
        &mut s,
        "next",
        Event::Text {
            key: "reply".into(),
            text: "下一轮回复".into(),
        },
    );
    assert!(!super::tool_group_has_reply(&s, end));
}

#[test]
fn tool_groups_skip_file_edits() {
    let _locale = crate::test_support::chinese();
    let tool = |key: &str, title: &str, input: &str| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: key.into(),
            created_at: 0,
            kind: ItemKind::Tool,
            text: title.into(),
            input: input.into(),
            detail: String::new(),
            status: "completed".into(),
            approval: None,
            model: None,
        })
    };
    let mut s = session();
    s.messages = vec![
        tool("a", "cargo build", r#"{"command":"cargo build"}"#),
        tool("b", "cargo test", r#"{"command":"cargo test"}"#),
        tool("c", "WebSearch", r#"{"query":"gpui"}"#),
        tool("d", "WebSearch", r#"{"query":"gpui list"}"#),
        tool(
            "e",
            "Edit",
            r#"{"file_path":"a.rs","old_string":"x","new_string":"y"}"#,
        ),
        tool(
            "f",
            "Edit",
            r#"{"file_path":"a.rs","old_string":"y","new_string":"z"}"#,
        ),
    ];
    assert_eq!(super::contiguous_tool_group(&s, 0), Some((0, 4)));
    assert_eq!(super::contiguous_tool_group(&s, 3), Some((0, 4)));
    assert_eq!(super::contiguous_tool_group(&s, 4), None);
}

#[test]
fn approved_tools_share_normal_activity_layout_without_hiding_unresolved_decisions() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (key, title, input) in [
        ("search", "WebSearch", r#"{"query":"gpui"}"#),
        ("fetch", "WebFetch", r#"{"url":"https://example.com"}"#),
    ] {
        apply_event(
            &mut s,
            "turn",
            Event::Tool {
                key: key.into(),
                title: title.into(),
                detail: String::new(),
                status: "completed".into(),
            },
        );
        apply_event(
            &mut s,
            "turn",
            Event::ToolInput {
                key: key.into(),
                input: input.into(),
            },
        );
        s.record_tool_approval(
            "turn",
            key,
            &format!("请求使用 {title}"),
            &format!(r#"{{"tool_use_id":"{key}"}}"#),
            "已允许",
        );
    }
    assert_eq!(super::contiguous_tool_group(&s, 0), Some((0, 2)));
    assert_eq!(super::tool_category(&s.messages[1]), "读取网页");
    assert_eq!(super::tool_summary(&s.messages[1]), "https://example.com");
    for status in ["等待中", "已拒绝", "已取消", "已中断"] {
        s.messages[0].approval.as_mut().unwrap().status = status.into();
        assert!(!super::is_compact_tool(&s.messages[0]), "{status}");
        assert_eq!(super::contiguous_tool_group(&s, 1), Some((1, 2)));
    }
    s.messages[0].approval.as_mut().unwrap().status = "已允许".into();
    s.messages[0].text = "Write".into();
    assert!(!super::is_compact_tool(&s.messages[0]));
}
#[test]
fn activity_rows_classify_failures_and_summaries() {
    let _locale = crate::test_support::chinese();
    let item = |kind: ItemKind, title: &str, input: &str, status: &str, detail: &str| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: "activity".into(),
            created_at: 0,
            kind,
            text: title.into(),
            input: input.into(),
            detail: detail.into(),
            status: status.into(),
            approval: None,
            model: None,
        })
    };
    let command = item(
        ItemKind::Tool,
        "pwsh",
        r#"{"command":"cargo test"}"#,
        "运行失败",
        r#"{"exitCode":1}"#,
    );
    assert_eq!(super::tool_category(&command), "终端命令");
    assert_eq!(super::tool_summary(&command), "cargo test");
    assert!(super::tool_failed(&command));

    let search = item(
        ItemKind::Tool,
        "WebSearch",
        r#"{"query":"gpui"}"#,
        "completed",
        "",
    );
    assert_eq!(super::tool_category(&search), "网络搜索");
    assert_eq!(super::tool_summary(&search), "gpui");
    assert!(!super::tool_failed(&search));

    let reasoning = item(ItemKind::Reasoning, "Let me look\nmore", "", "", "");
    assert_eq!(super::tool_category(&reasoning), "已思考");
    assert_eq!(super::tool_summary(&reasoning), "Let me look");
}
#[test]
fn edit_diffs_parse_old_new_pairs_and_codex_patches() {
    let _locale = crate::test_support::chinese();
    let item = |title: &str, input: &str| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: "edit".into(),
            created_at: 0,
            kind: ItemKind::Tool,
            text: title.into(),
            input: input.into(),
            detail: String::new(),
            status: "completed".into(),
            approval: None,
            model: None,
        })
    };
    let claude = super::edit_tool_diffs(&item(
        "Edit",
        r#"{"file_path":"src/main.rs","old_string":"let a = 1;","new_string":"let a = 2;\nlet b = 3;"}"#,
    ))
    .unwrap();
    assert_eq!(claude[0].path, "src/main.rs");
    assert_eq!((claude[0].added, claude[0].removed), (2, 1));
    let parsed = gpui_kit::component::diff::DiffFile::parse(&claude[0].patch).unwrap();
    assert_eq!(
        (parsed[0].additions(), parsed[0].deletions()),
        (2, 1),
        "the synthesized patch must round-trip through the diff parser"
    );

    let codex = super::edit_tool_diffs(&item(
        "修改文件",
        r#"[{"path":"notes.txt","kind":{"type":"update"},"diff":"@@ -1 +1 @@\n-old\n+new"}]"#,
    ))
    .unwrap();
    assert_eq!(codex[0].path, "notes.txt");
    assert_eq!(codex[0].note, "更新");
    assert_eq!((codex[0].added, codex[0].removed), (1, 1));
    assert!(
        codex[0].patch.contains("@@ -1,1 +1,1 @@"),
        "real hunk headers keep their source line numbers: {}",
        codex[0].patch
    );
}
#[test]
fn turn_file_stats_merge_repeated_edits_of_the_latest_turn() {
    let _locale = crate::test_support::chinese();
    let item = |kind: ItemKind, title: &str, input: &str| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: format!("{title}-{}", input.len()),
            created_at: 0,
            kind,
            text: title.into(),
            input: input.into(),
            detail: String::new(),
            status: "completed".into(),
            approval: None,
            model: None,
        })
    };
    let mut s = session();
    s.messages = vec![
        item(
            ItemKind::Tool,
            "Write",
            r#"{"file_path":"src/old.rs","content":"stale"}"#,
        ),
        item(ItemKind::User, "prompt", ""),
        item(
            ItemKind::Tool,
            "Edit",
            r#"{"file_path":"src/a.rs","old_string":"x","new_string":"y\nz"}"#,
        ),
        item(
            ItemKind::Tool,
            "Edit",
            r#"{"file_path":"src/a.rs","old_string":"x\ny","new_string":"z"}"#,
        ),
        item(
            ItemKind::Tool,
            "Write",
            r#"{"file_path":"src/b.rs","content":"one\ntwo"}"#,
        ),
        item(ItemKind::Tool, "pwsh", r#"{"command":"cargo test"}"#),
    ];
    let stats = super::turn_file_stats(&s);
    assert_eq!(stats.files.len(), 2);
    assert!(!stats.files.contains_key("src/old.rs"));
    assert_eq!((stats.added, stats.removed), (5, 3));
    let a = stats.files["src/a.rs"];
    assert_eq!((a.added, a.removed), (3, 3));
}
#[test]
fn plan_steps_follow_either_provider_shape_in_the_latest_turn() {
    let _locale = crate::test_support::chinese();
    let todo = super::plan_steps_from_input(
        r#"{"todos":[{"content":"Read code","status":"completed"},{"content":"Edit code","status":"in_progress"}]}"#,
    )
    .unwrap();
    assert_eq!(todo.len(), 2);
    assert_eq!(todo[0].status, super::PlanStatus::Completed);
    assert_eq!(todo[1].status, super::PlanStatus::InProgress);

    let item = |title: &str, input: &str| {
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: title.into(),
            created_at: 0,
            kind: ItemKind::Tool,
            text: title.into(),
            input: input.into(),
            detail: String::new(),
            status: "completed".into(),
            approval: None,
            model: None,
        })
    };
    let mut s = session();
    s.messages = vec![
        item(
            "update_plan",
            r#"{"plan":[{"step":"old step","status":"completed"}]}"#,
        ),
        ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: "prompt".into(),
            created_at: 0,
            kind: ItemKind::User,
            text: "new turn".into(),
            input: String::new(),
            detail: String::new(),
            status: Default::default(),
            approval: None,
            model: None,
        }),
        item(
            "update_plan",
            r#"{"plan":[{"step":"new step","status":"pending"}]}"#,
        ),
    ];
    let plan = super::plan_from_messages(&s);
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].text, "new step");
    assert_eq!(plan[0].status, super::PlanStatus::Pending);
}
#[test]
fn stats_paths_strip_the_workspace_prefix() {
    let _locale = crate::test_support::chinese();
    assert_eq!(
        super::workspace_relative_path("D:\\m\\conduit\\src\\app.rs", Some("D:\\m\\conduit")),
        "src/app.rs"
    );
    assert_eq!(
        super::workspace_relative_path("d:/m/conduit/README.md", Some("D:\\m\\conduit\\")),
        "README.md"
    );
    assert_eq!(
        super::workspace_relative_path("C:\\other\\file.rs", Some("D:\\m\\conduit")),
        "C:/other/file.rs"
    );
    assert_eq!(
        super::workspace_relative_path(
            "D:\\m\\conduit\\src\\app.rs",
            Some("\\\\?\\D:\\m\\conduit")
        ),
        "src/app.rs"
    );
}
#[test]
fn final_snapshot_replaces_stream_and_turns_keep_distinct_identity() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for part in ["Hello", " world"] {
        apply_event(
            &mut s,
            "turn-0",
            Event::Delta {
                key: "m1".into(),
                text: part.into(),
            },
        );
    }
    apply_event(
        &mut s,
        "turn-0",
        Event::Text {
            key: "m1".into(),
            text: "Hello world".into(),
        },
    );
    assert_eq!(s.messages.len(), 1);
    assert_eq!(s.messages[0].text, "Hello world");
    apply_event(
        &mut s,
        "turn-1",
        Event::Text {
            key: "m1".into(),
            text: "Second turn".into(),
        },
    );
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].text, "Hello world");
}
#[test]
fn command_output_updates_one_card_and_completion_replaces_partial_output() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (title, detail, status) in [
        ("cargo check", "", "运行中"),
        ("命令输出", "part", "输出中"),
        ("命令输出", "ial", "输出中"),
        ("cargo check", "partial\ndone", "completed"),
    ] {
        apply_event(
            &mut s,
            "turn-0",
            Event::Tool {
                key: "cmd1".into(),
                title: title.into(),
                detail: detail.into(),
                status: status.into(),
            },
        );
    }
    assert_eq!(s.messages.len(), 1);
    assert_eq!(s.messages[0].text, "cargo check");
    assert_eq!(s.messages[0].detail, "partial\ndone");
    assert_eq!(s.messages[0].status, "completed");
}
#[test]
fn cancellation_only_marks_unfinished_tools_from_its_turn() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (prefix, key, status) in [
        ("turn-0", "old", "运行中"),
        ("turn-1", "done", "completed"),
        ("turn-1", "active", "输出中"),
    ] {
        apply_event(
            &mut s,
            prefix,
            Event::Tool {
                key: key.into(),
                title: "tool".into(),
                detail: "partial result".into(),
                status: status.into(),
            },
        );
    }
    finish_tool_cards(&mut s, "turn-1", "已取消");
    assert!(s.messages.iter().all(|m| m.kind == ItemKind::Tool));
    assert_eq!(s.messages[0].status, "运行中");
    assert_eq!(s.messages[1].status, "completed");
    assert_eq!(s.messages[2].status, "已取消");
    assert_eq!(s.messages[2].detail, "partial result");
}
#[test]
fn adjacent_duplicate_completed_blocks_are_collapsed() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    for (key, text) in [("a", "duplicate"), ("b", "duplicate"), ("c", "next")] {
        apply_event(
            &mut s,
            "turn-0",
            Event::Text {
                key: key.into(),
                text: text.into(),
            },
        );
    }
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[0].text, "duplicate");
    assert_eq!(s.messages[1].text, "next");
}
#[test]
fn failure_echoed_as_assistant_text_is_detected() {
    let _locale = crate::test_support::chinese();
    assert!(same_failure("执行失败：boom", "boom"));
    assert!(same_failure("boom", "执行失败：boom"));
    assert!(same_failure(
        "Failed to authenticate. API Error: 403\nmore detail",
        "Failed to authenticate. API Error: 403\nmore detail"
    ));
    assert!(!same_failure("ok", "boom"));
    assert!(!same_failure("", "boom"));
}
#[test]
fn latest_naming_excerpt_prefers_recent_turns() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    assert_eq!(latest_naming_excerpt(&s), None);
    for (index, (kind, text)) in [
        (ItemKind::User, "旧问题"),
        (ItemKind::Assistant, "旧回答"),
        (ItemKind::Tool, "工具输出应忽略"),
        (ItemKind::User, "新问题"),
        (ItemKind::Assistant, "新回答"),
    ]
    .into_iter()
    .enumerate()
    {
        s.messages.push(ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key: format!("m{index}"),
            created_at: 0,
            kind,
            text: text.into(),
            input: String::new(),
            detail: String::new(),
            status: Default::default(),
            approval: None,
            model: None,
        }));
    }
    assert_eq!(
        latest_naming_excerpt(&s).as_deref(),
        Some("旧问题\n\n旧回答\n\n新问题\n\n新回答")
    );
    s.messages.push(ChatItem::from(crate::domain::ChatItemData {
        reasoning_summary: None,
        elapsed_ms: None,
        key: "long".into(),
        created_at: 0,
        kind: ItemKind::Assistant,
        text: "长".repeat(1_000),
        input: String::new(),
        detail: String::new(),
        status: Default::default(),
        approval: None,
        model: None,
    }));
    assert_eq!(latest_naming_excerpt(&s).unwrap().chars().count(), 800);
}
#[test]
fn single_line_title_collapses_line_breaks() {
    let _locale = crate::test_support::chinese();
    assert_eq!(
        single_line_title("# AGENTS.md\n<INSTRUCTIONS>"),
        "# AGENTS.md <INSTRUCTIONS>"
    );
    assert_eq!(single_line_title("第一行\r第二行"), "第一行 第二行");
}

#[test]
fn error_summary_extracts_json_and_bounds_multiline_diagnostics() {
    let _locale = crate::test_support::chinese();
    assert_eq!(
        super::failure_summary(r#"执行失败：{"message":"连接已断开"}"#),
        "连接已断开"
    );
    assert_eq!(
        super::failure_summary("\n请求超时\n原始诊断信息"),
        "请求超时"
    );
    assert_eq!(super::failure_summary(""), "本次请求未完成，请查看详情。");
    assert_eq!(
        super::failure_summary(&"错".repeat(200)).chars().count(),
        161
    );
}
#[test]
fn session_search_matches_title_and_message_text() {
    let _locale = crate::test_support::chinese();
    let mut s = session();
    s.title = "Rust 重构计划".into();
    s.messages.push(ChatItem::from(crate::domain::ChatItemData {
        reasoning_summary: None,
        elapsed_ms: None,
        key: "m1".into(),
        created_at: 0,
        kind: ItemKind::Assistant,
        text: "let me check the ALPHA dependency".into(),
        input: String::new(),
        detail: String::new(),
        status: Default::default(),
        approval: None,
        model: None,
    }));
    let workspaces = [crate::domain::Workspace {
        path: std::env::temp_dir(),
        sessions: vec![s],
    }];
    for query in ["", "rust", "ALPHA", "重构"] {
        let results = super::search_worker::collect(&workspaces, query);
        assert!(results.matches.contains(&1), "{query}");
        assert_eq!(results.hits.len(), 1);
    }
    for query in ["beta", "unmatched"] {
        let results = super::search_worker::collect(&workspaces, query);
        assert!(results.matches.is_empty(), "{query}");
        assert!(results.hits.is_empty());
    }
}
#[test]
fn highlight_ranges_handle_unicode_and_case() {
    let _locale = crate::test_support::chinese();
    assert_eq!(
        highlight_ranges("Hello hello HELLO", "hello"),
        vec![(0, 5), (6, 11), (12, 17)]
    );
    // "中文" is 6 bytes, so Markdown starts at byte 6 and ends at 14.
    assert_eq!(
        highlight_ranges("中文Markdown测试", "markdown"),
        vec![(6, 14)]
    );
    assert_eq!(highlight_ranges("多字节中文测试", "中文"), vec![(9, 15)]);
    assert!(highlight_ranges("", "x").is_empty());
    assert!(highlight_ranges("abc", "").is_empty());
    assert!(highlight_ranges("İstanbul", "i").iter().all(|(a, b)| a < b));
}
#[test]
fn search_snippet_keeps_match_visible_for_multibyte_text() {
    let _locale = crate::test_support::chinese();
    let text = "这是很多中文内容，然后出现 Markdown 关键词，后面继续一些内容。";
    let snippet = search_snippet(text, "markdown");
    assert!(snippet.to_lowercase().contains("markdown"));
    assert!(snippet.chars().count() <= 162);
}

#[test]
fn historical_reply_timing_prefers_recorded_duration_and_bounds_estimates_to_a_turn() {
    let _locale = crate::test_support::chinese();
    use super::chat::reply_elapsed_label;
    let mut session = session();
    for (index, (kind, created_at)) in [
        (ItemKind::User, 100),
        (ItemKind::Assistant, 105),
        (ItemKind::Tool, 110),
        (ItemKind::Assistant, 115),
        (ItemKind::User, 1000),
        (ItemKind::Assistant, 1007),
    ]
    .into_iter()
    .enumerate()
    {
        session
            .messages
            .push(ChatItem::from(crate::domain::ChatItemData {
                reasoning_summary: None,
                elapsed_ms: None,
                key: format!("timed-{index}"),
                created_at,
                kind,
                text: "历史内容".into(),
                input: String::new(),
                detail: String::new(),
                status: Default::default(),
                approval: None,
                model: None,
            }));
    }
    assert_eq!(
        reply_elapsed_label(&session, 3, false).as_deref(),
        Some("耗时约 15 s")
    );
    assert_eq!(
        reply_elapsed_label(&session, 5, false).as_deref(),
        Some("耗时约 7 s")
    );
    assert_eq!(
        reply_elapsed_label(&session, 3, true).as_deref(),
        Some("耗时约 15 s")
    );
    assert_eq!(reply_elapsed_label(&session, 5, true), None);
    session.messages[3].elapsed_ms = Some(16250);
    assert_eq!(
        reply_elapsed_label(&session, 3, false).as_deref(),
        Some("耗时 16.2 s")
    );
    for (elapsed_ms, expected) in [
        (60_000, "耗时 60.0 s"),
        (60_001, "耗时 1分00秒"),
        (65_000, "耗时 1分05秒"),
        (120_000, "耗时 2分00秒"),
    ] {
        session.messages[3].elapsed_ms = Some(elapsed_ms);
        assert_eq!(
            reply_elapsed_label(&session, 3, false).as_deref(),
            Some(expected)
        );
    }
    session.messages[5].created_at = 1065;
    assert_eq!(
        reply_elapsed_label(&session, 5, false).as_deref(),
        Some("耗时约 1分05秒")
    );
    session.messages[5].created_at = 1007;
    session.messages[4].created_at = 0;
    assert_eq!(reply_elapsed_label(&session, 5, false), None);
    session.messages[4].created_at = 1008;
    assert_eq!(reply_elapsed_label(&session, 5, false), None);
}

#[test]
fn session_activity_survives_restore_and_legacy_messages() {
    let _locale = crate::test_support::chinese();
    let mut restored = session();
    restored.created_at = 10;
    assert_eq!(restored.last_activity_at(), 10);
    restored.messages = vec![
        serde_json::from_value(serde_json::json!({
            "key":"resumed", "kind":"User", "text":"continue", "created_at":100
        }))
        .unwrap(),
        serde_json::from_value(serde_json::json!({
            "key":"legacy-notice", "kind":"Notice", "text":"notice"
        }))
        .unwrap(),
    ];
    let saved = serde_json::to_string(&restored).unwrap();
    let restored: SessionRecord = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.last_activity_at(), 100);
}

#[test]
fn localized_tool_labels_preserve_edit_detection_and_approval_semantics() {
    let _locale = crate::test_support::scoped("en");
    for language in ["en", "zh-CN"] {
        conduit::i18n::set_locale(&language.to_string().into());
        let mut item: ChatItem = serde_json::from_value(serde_json::json!({
            "key":"edit", "kind":"Tool", "text":"修改文件", "status":"运行中",
            "input": serde_json::json!([{"path":"source.rs", "kind":"added", "diff":"@@ -0,0 +1 @@\n+hello"}]).to_string()
        })).unwrap();
        assert!(super::is_tool_edit(&item));
        assert!(super::tool_running(&item));
        let diffs = super::edit_tool_diffs(&item).unwrap();
        assert!(diffs[0].patch.starts_with("--- /dev/null\n"));
        assert_eq!(item.status.as_str(), "运行中");
        item.status = crate::message_status::ToolStatus::Failed;
        assert!(super::tool_failed(&item));
        assert_eq!(
            conduit::i18n::source("已允许"),
            if language == "en" {
                "Allowed"
            } else {
                "已允许"
            }
        );
        assert!(
            super::failure_guidance("provider not found").contains(if language == "en" {
                "Settings"
            } else {
                "设置"
            })
        );
    }
}
