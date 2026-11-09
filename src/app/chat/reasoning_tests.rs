//! Protect the provider -> queue -> transcript -> visible reasoning path.
use crate::app::{
    Event, MessageIndex, Provider, apply_event_indexed, finish_tool_cards, tool_running,
};
use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, px};
use serde_json::json;

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.simulate_next_frame(cx);
        let _ = window.draw(cx);
    });
}

#[gpui_kit::test]
fn reasoning_stream_stays_visible_after_reply_and_restoration(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "reasoning-visibility");
    cx.simulate_resize(gpui_kit::size(px(1200.), px(900.)));
    let sender = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().messages.clear();
            crate::app::queue::tests::running(app, 1)
        })
    });
    // A successful handshake and an empty block are not evidence of thinking text.
    sender
        .send(Event::NativeId("reasoning-thread".into()))
        .unwrap();
    sender
        .send(Event::ReasoningSummary {
            key: "empty".into(),
            text: "  ".into(),
            summary_index: 0,
            snapshot: false,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session_control.runs[&1].activity_label(),
            conduit::i18n::text_static("stream_events.waiting_for_model_response")
        );
        assert!(app.session(1).unwrap().messages.is_empty());
    });
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Codex, "C:/work".into(), "inspect".into(), None);
    let mut feed = |value| {
        for event in protocol.accept(value).events {
            sender.send(event).unwrap();
        }
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        draw(cx);
    };
    feed(json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":0,"delta":"**Inspect files**"}}));
    feed(json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":1,"delta":"Check the implementation."}}));
    assert!(
        cx.debug_bounds("reasoning-body-1-0").is_some(),
        "streamed body must be visible without a click"
    );
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session_control.runs[&1].activity_label(),
            conduit::i18n::text_static("stream_events.thinking")
        );
        let item = &app.session(1).unwrap().messages[0];
        assert_eq!(item.text, "**Inspect files**\nCheck the implementation.");
        assert_eq!(item.status, crate::message_status::ToolStatus::Running);
        assert!(app.chat_rows.group(0).is_none());
    });
    // Empty completion closes the activity without erasing the accumulated text.
    for event in protocol
        .accept(json!({"method":"item/completed",
        "params":{"item":{"id":"r1","type":"reasoning","summary":[]}}}))
        .events
    {
        sender.send(event).unwrap();
    }
    sender
        .send(Event::Tool {
            key: "tool".into(),
            title: "Read".into(),
            detail: String::new(),
            status: "completed".into(),
        })
        .unwrap();
    sender
        .send(Event::Text {
            key: "reply".into(),
            text: "The answer.".into(),
        })
        .unwrap();
    sender
        .send(Event::Finished {
            error: None,
            cancelled: false,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    draw(cx);
    assert!(
        cx.debug_bounds("reasoning-body-1-0").is_some(),
        "answer completion must not hide reasoning inside tool groups"
    );
    cx.update(|_, cx| {
        let app = app.read(cx);
        let session = app.session(1).unwrap();
        assert_eq!(
            session.messages[0].text,
            "**Inspect files**\nCheck the implementation."
        );
        assert_eq!(
            session.messages[0].status,
            crate::message_status::ToolStatus::Completed
        );
        assert!(app.chat_rows.group(0).is_none());
        assert_eq!(app.chat_rows.group(1).unwrap().count, 1);
        assert!(!app.chat_rows.group(1).unwrap().expanded);
    });
    // Exercise the actual click handler and timeline cache invalidation.
    let point = cx.debug_bounds("activity-1-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-0").is_none());
    assert!(cx.debug_bounds("activity-1-0").is_some());
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.switch(1, Some(2), window, cx);
            app.switch(0, Some(1), window, cx);
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds("reasoning-body-1-0").is_none(),
        "manual collapse survives session switches"
    );
    let point = cx.debug_bounds("activity-1-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-0").is_some());
    // Reloaded transcript snapshots use the same standalone rows and default visibility.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            let saved = serde_json::to_string(&app.session(1).unwrap().messages).unwrap();
            app.session_mut(1).unwrap().messages = serde_json::from_str(&saved).unwrap();
            app.reasoning_expansion.clear();
            app.sync_chat_rows();
            app.scroll.remeasure();
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-0").is_some());
}

#[test]
fn reasoning_completion_and_cancellation_preserve_received_text() {
    for status in [
        crate::message_status::ToolStatus::Cancelled,
        crate::message_status::ToolStatus::MissingResult,
    ] {
        let mut session = crate::app::tests::session();
        let mut index = MessageIndex::default();
        for prefix in ["old", "current"] {
            apply_event_indexed(
                &mut session,
                prefix,
                Event::ReasoningSummary {
                    key: "reason".into(),
                    text: "First part".into(),
                    summary_index: 0,
                    snapshot: false,
                },
                &mut index,
            );
        }
        apply_event_indexed(
            &mut session,
            "current",
            Event::ReasoningSummary {
                key: "reason".into(),
                text: "Final summary".into(),
                summary_index: 0,
                snapshot: true,
            },
            &mut index,
        );
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].text, "Final summary");
        assert_eq!(
            session.messages[1].status,
            crate::message_status::ToolStatus::Completed
        );
        apply_event_indexed(
            &mut session,
            "current",
            Event::ReasoningSummary {
                key: "partial".into(),
                text: "Partial summary".into(),
                summary_index: 0,
                snapshot: false,
            },
            &mut index,
        );
        finish_tool_cards(&mut session, "current", status.as_str());
        assert_eq!(
            session.messages[0].status,
            crate::message_status::ToolStatus::Running
        );
        assert!(!tool_running(&session.messages[2]));
        assert_eq!(session.messages[2].text, "Partial summary");
        // Completion-only empty items never create a phantom message.
        assert!(
            apply_event_indexed(
                &mut session,
                "current",
                Event::ReasoningSummary {
                    key: "empty".into(),
                    text: String::new(),
                    summary_index: 0,
                    snapshot: true,
                },
                &mut index
            )
            .is_none()
        );
        assert_eq!(session.messages.len(), 3);
    }
}

#[gpui_kit::test]
fn codex_client_rejection_does_not_claim_model_is_thinking(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "reasoning-rejection");
    let sender = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().messages.clear();
            crate::app::queue::tests::running(app, 1)
        })
    });
    sender
        .send(Event::NativeId("rejected-thread".into()))
        .unwrap();
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Codex, "C:/work".into(), "hello".into(), None);
    // Shape observed in the failing session; endpoint/account data is omitted.
    let error = json!({
        "message": "unexpected status 403 Forbidden: upstream_status: HTTP 403; cause: {\"error\":{\"message\":\"This account only allows Codex official clients\",\"type\":\"forbidden_error\"}}",
        "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode": 403}},
        "additionalDetails": null
    });
    for event in protocol
        .accept(json!({"method":"error", "params":{
            "error":error, "willRetry":false
        }}))
        .events
    {
        sender.send(event).unwrap();
    }
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session_control.runs[&1].activity_label(),
            conduit::i18n::text_static("stream_events.request_failed")
        );
        let messages = &app.session(1).unwrap().messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, crate::domain::ItemKind::Error);
        assert_eq!(
            crate::failure::failure_summary(&messages[0].text),
            conduit::i18n::text_static("failure.codex_official_clients_only")
        );
        assert_eq!(
            crate::failure::failure_guidance(&messages[0].detail),
            conduit::i18n::text_static("failure.codex_client_restriction_guidance")
        );
    });
    assert!(cx.debug_bounds("reasoning-body-1-0").is_none());
    // The final error must coalesce with the earlier banner, keeping its full cause.
    sender
        .send(Event::Finished {
            error: Some(error.to_string()),
            cancelled: false,
        })
        .unwrap();
    cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
    draw(cx);
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert!(!app.session_control.runs.contains_key(&1));
        let messages = &app.session(1).unwrap().messages;
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .detail
                .contains("only allows Codex official clients")
        );
    });
}
