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

#[test]
fn codex_heading_only_completion_keeps_streamed_summary_details() {
    let mut session = crate::app::tests::session();
    session.messages.clear();
    session.provider = Provider::Codex;
    let mut index = MessageIndex::default();
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Codex, "C:/work".into(), "inspect".into(), None);
    for value in [
        json!({"method":"item/reasoning/summaryTextDelta", "params":{"itemId":"r1","summaryIndex":0,"delta":"**Inspect files**\n\nRead the implementation as it arrives."}}),
        json!({"method":"item/completed", "params":{"item":{"id":"r1","type":"reasoning","summary":["**Inspect files**"],"content":[]}}}),
    ] {
        for event in protocol.accept(value).events {
            apply_event_indexed(&mut session, "turn", event, &mut index);
        }
    }
    assert_eq!(
        session.messages[0].text,
        "**Inspect files**\n\nRead the implementation as it arrives."
    );
}

#[test]
fn codex_multi_section_completion_keeps_each_streamed_summary_body() {
    let mut session = crate::app::tests::session();
    session.messages.clear();
    session.provider = Provider::Codex;
    let mut index = MessageIndex::default();
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Codex, "C:/work".into(), "inspect".into(), None);
    for value in [
        json!({"method":"item/reasoning/summaryTextDelta", "params":{"itemId":"r1","summaryIndex":0,"delta":"**Checking GMV routing scope**\n\nRead the routing implementation."}}),
        json!({"method":"item/reasoning/summaryTextDelta", "params":{"itemId":"r1","summaryIndex":1,"delta":"**Inspecting GMV sync paths**\n\nCheck the mapper and its tests."}}),
        // Completion shape observed in the user's native rollout.
        json!({"method":"item/completed", "params":{"item":{"id":"r1","type":"reasoning","summary":["**Checking GMV routing scope**","**Inspecting GMV sync paths**"],"content":[]}}}),
    ] {
        for event in protocol.accept(value).events {
            apply_event_indexed(&mut session, "turn", event, &mut index);
        }
    }
    assert_eq!(
        session.messages[0].text,
        "**Checking GMV routing scope**\n\nRead the routing implementation.\n\n**Inspecting GMV sync paths**\n\nCheck the mapper and its tests."
    );
}

#[test]
fn codex_summary_preview_follows_latest_update_and_disclosure_keeps_all_sections() {
    for text in [
        "**Checking GMV routing scope**\n\n**Inspecting GMV sync paths**",
        "**Checking GMV routing scope**\n**Inspecting GMV sync paths**",
    ] {
        let item = serde_json::from_value(json!({
            "key":"summary","kind":"Reasoning","text":text,"reasoning_summary":true
        }))
        .unwrap();
        assert_eq!(super::reasoning_preview(text), "Inspecting GMV sync paths");
        assert_eq!(
            super::message_markdown(&item, Provider::Codex),
            "**Checking GMV routing scope**\n\n**Inspecting GMV sync paths**",
            "opening a summary must include its first section too"
        );
    }
    for (text, expected) in [
        ("**First**\n\n**Inspecting GMV", "Inspecting GMV"),
        ("**First**\n\n**", "First"),
        ("**First**\n\nReading the mapper.\n", "Reading the mapper."),
        ("**First**\n<!-- progress -->", "First"),
    ] {
        assert_eq!(super::reasoning_preview(text), expected);
    }
}

#[gpui_kit::test]
fn reasoning_live_deltas_show_before_completion_with_collapse_default(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "reasoning-live-default");
    cx.simulate_resize(gpui_kit::size(px(1200.), px(900.)));
    let sender = cx.update(|_, cx| {
        app.update(cx, |app, _| {
            app.startup = None;
            app.writable = false;
            app.store.ui.collapse_reasoning = true;
            app.session_mut(1).unwrap().provider = Provider::Codex;
            app.session_mut(1).unwrap().messages.clear();
            crate::app::queue::tests::running(app, 1)
        })
    });
    let mut protocol =
        crate::protocol::Protocol::new(Provider::Codex, "C:/work".into(), "inspect".into(), None);
    let mut feed = |value, cx: &mut VisualTestContext| {
        for event in protocol.accept(value).events {
            sender.send(event).unwrap();
        }
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        draw(cx);
        draw(cx);
    };
    feed(
        json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":0,"delta":"**Checking GMV routing scope**"}}),
        cx,
    );
    assert!(cx.debug_bounds("reasoning-body-1-0").is_none());
    feed(
        json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":0,"delta":"\n\nRead the implementation."}}),
        cx,
    );
    assert!(
        cx.debug_bounds("reasoning-body-1-0").is_some(),
        "collapse-by-default must not hide a live stream"
    );
    let before = cx.debug_bounds("reasoning-body-1-0").unwrap().size.height;
    feed(
        json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":0,"delta":"\n\nCheck the mapper."}}),
        cx,
    );
    assert!(cx.debug_bounds("reasoning-body-1-0").unwrap().size.height > before);
    cx.update(|_, cx| {
        let app = app.read(cx);
        let item = &app.session(1).unwrap().messages[0];
        assert!(tool_running(item), "no completion was sent");
        assert_eq!(super::reasoning_preview(&item.text), "Check the mapper.");
    });
    let point = cx.debug_bounds("activity-1-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    draw(cx);
    feed(
        json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":1,"delta":"**Inspecting GMV"}}),
        cx,
    );
    assert!(cx.debug_bounds("reasoning-body-1-0").is_none());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            super::reasoning_preview(&app.session(1).unwrap().messages[0].text),
            "Inspecting GMV",
            "a manually collapsed stream still updates its current status"
        );
    });
    feed(
        json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":1,"delta":" sync paths**"}}),
        cx,
    );
    feed(
        json!({"method":"item/completed", "params":{"item":{
        "id":"r1","type":"reasoning","summary":["**Checking GMV routing scope**","**Inspecting GMV sync paths**"],"content":[]}}}),
        cx,
    );
    assert!(cx.debug_bounds("reasoning-body-1-0").is_none());
    let point = cx.debug_bounds("activity-1-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-0").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        let item = &app.session(1).unwrap().messages[0];
        assert!(!tool_running(item));
        assert_eq!(
            super::message_markdown(item, Provider::Codex),
            "**Checking GMV routing scope**\n\nRead the implementation.\n\nCheck the mapper.\n\n**Inspecting GMV sync paths**"
        );
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
            app.session_mut(1).unwrap().provider = Provider::Codex;
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
            replace: false,
            summary: false,
            elapsed_ms: None,
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
    assert!(
        protocol
            .accept(json!({"method":"item/started",
        "params":{"item":{"id":"r1","type":"reasoning","summary":[],"content":[]}}}))
            .events
            .is_empty()
    );
    let mut feed = |value| {
        for event in protocol.accept(value).events {
            sender.send(event).unwrap();
        }
        cx.update(|window, cx| app.update(cx, |app, cx| app.poll_streams(window, cx)));
        draw(cx);
        draw(cx);
        cx.debug_bounds("reasoning-quote-1-0").unwrap().size.height
    };
    let partial_heading_height = feed(json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":0,"delta":"**Inspect"}}));
    let heading_height = feed(json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":0,"delta":" files**"}}));
    assert_eq!(
        partial_heading_height, heading_height,
        "streaming a heading must keep one compact row"
    );
    let sections_height = feed(json!({"method":"item/reasoning/summaryTextDelta",
        "params":{"itemId":"r1","summaryIndex":1,"delta":"**Check tests**"}}));
    assert!(
        sections_height > heading_height,
        "separate summary parts must render on separate lines"
    );
    feed(json!({"method":"item/reasoning/textDelta",
        "params":{"itemId":"r1","contentIndex":0,"delta":"Read the implementation."}}));
    let detailed_height = feed(json!({"method":"item/reasoning/textDelta",
        "params":{"itemId":"r1","contentIndex":0,"delta":"\n\nCheck the implementation.\n\nVerify the result."}}));
    assert!(
        detailed_height > sections_height,
        "later paragraphs must expand the visible reasoning body"
    );
    assert!(
        cx.debug_bounds("reasoning-body-1-0").is_some(),
        "streamed body must be visible without a click"
    );
    assert!(cx.debug_bounds("reasoning-elapsed-1-0").is_some());
    cx.update(|_, cx| {
        let app = app.read(cx);
        assert_eq!(
            app.session_control.runs[&1].activity_label(),
            conduit::i18n::text_static("stream_events.thinking")
        );
        let item = &app.session(1).unwrap().messages[0];
        assert_eq!(
            item.text,
            "Read the implementation.\n\nCheck the implementation.\n\nVerify the result."
        );
        assert_eq!(item.status, crate::message_status::ToolStatus::Running);
        assert_eq!(item.reasoning_summary, Some(false));
        assert!(app.chat_rows.group(0).is_none());
    });
    // A late heading-only snapshot must not erase the streamed body.
    for event in protocol
        .accept(json!({"method":"item/completed",
        "params":{"item":{"id":"r1","type":"reasoning","summary":["**Inspect files**"]}}}))
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
            "Read the implementation.\n\nCheck the implementation.\n\nVerify the result."
        );
        assert_eq!(
            session.messages[0].status,
            crate::message_status::ToolStatus::Completed
        );
        assert!(session.messages[0].elapsed_ms.is_some());
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
    // The default-collapse preference still allows the full received body to
    // be opened, including after restoring a transcript.
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.store.ui.collapse_reasoning = true;
            app.reasoning_expansion.clear();
            app.remeasure_message(0);
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-0").is_none());
    let point = cx.debug_bounds("activity-1-0").unwrap().center();
    cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(cx.debug_bounds("reasoning-body-1-0").is_some());
}

#[test]
fn reasoning_markdown_keeps_prose_and_code_unchanged() {
    for text in [
        "**One heading**",
        "**First heading**\n\n**Second heading**",
        "**Heading**\nA continuation of the same paragraph.",
        "**Heading**\n\nFirst paragraph.\n\nSecond paragraph.",
        "    **Indented code**\n    **More code**",
        "```\n**Code**\n**More code**\n```",
        "**Bold** text and **more bold**\n**Heading**",
    ] {
        assert!(
            matches!(super::reasoning_markdown(text), std::borrow::Cow::Borrowed(original) if original == text),
            "ordinary Markdown must remain unchanged: {text:?}"
        );
    }
}

#[gpui_kit::test]
fn restored_reasoning_preserves_separate_summary_paragraphs(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "reasoning-paragraphs");
    cx.simulate_resize(gpui_kit::size(px(1200.), px(900.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.session_mut(1).unwrap().provider = Provider::Codex;
            app.store.ui.collapse_reasoning = false;
            app.session_mut(1).unwrap().messages = serde_json::from_value(json!([
                {"key":"one", "kind":"Reasoning", "text":"**Inspect files**", "status":"completed", "elapsed_ms":46},
                {"key":"legacy", "kind":"Reasoning", "text":"**Inspect files**\n**Check tests**", "status":"completed"},
                {"key":"sections", "kind":"Reasoning", "text":"**Inspect files**\n\n**Check tests**", "status":"completed"},
                {"key":"details", "kind":"Reasoning", "text":"**Inspect files**\n\nRead the implementation.\n\nVerify the result.", "status":"completed"}
            ])).unwrap();
            app.sync_chat_rows();
            cx.notify();
        });
    });
    draw(cx);
    draw(cx);
    assert!(
        cx.debug_bounds("reasoning-body-1-0").is_none(),
        "a heading alone must not pretend to have an expandable body"
    );
    let one = cx.debug_bounds("reasoning-quote-1-0").unwrap().size.height;
    assert!(
        cx.debug_bounds("reasoning-elapsed-1-0").is_none(),
        "legacy summary delivery duration is not a reliable reasoning timer"
    );
    let legacy = cx.debug_bounds("reasoning-quote-1-1").unwrap().size.height;
    let sections = cx.debug_bounds("reasoning-quote-1-2").unwrap().size.height;
    let details = cx.debug_bounds("reasoning-quote-1-3").unwrap().size.height;
    assert!(
        legacy > one,
        "old heading-only parts must not collapse into one visual line"
    );
    assert_eq!(
        legacy, sections,
        "restored and new summary parts must render consistently"
    );
    assert!(
        details > sections,
        "the entire detailed summary must remain visible"
    );
    cx.update(|_, cx| {
        assert_eq!(
            app.read(cx).session(1).unwrap().messages[1].text,
            "**Inspect files**\n**Check tests**",
            "display normalization must not rewrite stored text"
        );
    });
}

#[gpui_kit::test]
fn tool_groups_can_collapse_before_reply_and_while_running(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx, _) = crate::app::queue::tests::fixture(cx, "tool-disclosure");
    cx.simulate_resize(gpui_kit::size(px(1200.), px(900.)));
    cx.update(|_, cx| {
        app.update(cx, |app, cx| {
            app.startup = None;
            app.writable = false;
            app.store.ui.collapse_reasoning = false;
            for id in [1, 2] {
                app.session_mut(id).unwrap().messages = serde_json::from_value(json!([
                    {"key":"user", "kind":"User", "text":"Check the implementation"},
                    {"key":"read", "kind":"Tool", "text":"Read", "status":"completed"},
                    {"key":"search", "kind":"Tool", "text":"Grep", "status":"completed"},
                    {"key":"thought", "kind":"Reasoning", "text":"Checking read index"}
                ]))
                .unwrap();
            }
            app.sync_chat_rows();
            cx.notify();
        });
    });
    draw(cx);
    for status in ["completed", "运行中"] {
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.session_mut(1).unwrap().messages[2].status = status.into();
                app.sync_chat_rows();
                cx.notify();
            });
        });
        draw(cx);
        assert!(cx.debug_bounds("activity-1-2").is_some());
        let point = cx.debug_bounds("tool-group-header-1-1").unwrap().center();
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        draw(cx);
        assert!(
            cx.debug_bounds("activity-1-2").is_none(),
            "{status} group must collapse"
        );
        assert!(cx.debug_bounds("reasoning-body-1-3").is_some());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.switch(1, Some(2), window, cx);
                app.sync_chat_rows();
                assert!(app.chat_rows.group(1).unwrap().expanded);
                app.switch(0, Some(1), window, cx);
                app.sync_chat_rows();
                assert!(!app.chat_rows.group(1).unwrap().expanded);
            });
        });
        draw(cx);
        let point = cx.debug_bounds("tool-group-header-1-1").unwrap().center();
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        draw(cx);
        assert!(cx.debug_bounds("activity-1-2").is_some());
    }
}

#[test]
fn reasoning_times_each_phase_and_preserves_duration_after_completion() {
    use std::time::{Duration, Instant};
    let mut session = crate::app::tests::session();
    session.messages.clear();
    let mut index = MessageIndex::default();
    let start = Instant::now();
    let delta = |key: &str| Event::ReasoningSummary {
        key: key.into(),
        text: "Checking the implementation".into(),
        summary_index: 0,
        snapshot: false,
        replace: false,
        summary: false,
        elapsed_ms: None,
    };
    let feed =
        |session: &mut crate::domain::SessionRecord, index: &mut MessageIndex, seconds, event| {
            index.observe_reasoning(
                session,
                "turn",
                &event,
                start + Duration::from_secs(seconds),
            );
            apply_event_indexed(session, "turn", event, index);
        };
    feed(&mut session, &mut index, 0, delta("first"));
    feed(&mut session, &mut index, 2, delta("first"));
    assert_eq!(
        index.reasoning_elapsed("turn-first", start + Duration::from_secs(5)),
        Some(Duration::from_secs(5))
    );
    feed(
        &mut session,
        &mut index,
        7,
        Event::Tool {
            key: "tool".into(),
            title: "Read".into(),
            detail: String::new(),
            status: "运行中".into(),
        },
    );
    assert_eq!(session.messages[0].elapsed_ms, Some(7000));
    assert!(
        index
            .reasoning_elapsed("turn-first", start + Duration::from_secs(30))
            .is_none()
    );
    // A late completion snapshot must not include the intervening tool time.
    feed(
        &mut session,
        &mut index,
        30,
        Event::ReasoningSummary {
            key: "first".into(),
            text: String::new(),
            summary_index: 0,
            snapshot: true,
            replace: false,
            summary: false,
            elapsed_ms: None,
        },
    );
    assert_eq!(session.messages[0].elapsed_ms, Some(7000));
    feed(&mut session, &mut index, 40, delta("second"));
    feed(
        &mut session,
        &mut index,
        43,
        Event::Text {
            key: "reply".into(),
            text: "Answer".into(),
        },
    );
    assert_eq!(session.messages[2].elapsed_ms, Some(3000));
    feed(&mut session, &mut index, 50, delta("third"));
    feed(&mut session, &mut index, 52, delta("fourth"));
    assert_eq!(session.messages[4].elapsed_ms, Some(2000));
    index.stop_reasoning(start + Duration::from_secs(55));
    feed(
        &mut session,
        &mut index,
        90,
        Event::Finished {
            error: None,
            cancelled: true,
        },
    );
    assert_eq!(
        session.messages[5].elapsed_ms,
        Some(3000),
        "cleanup is not thinking time"
    );
    feed(
        &mut session,
        &mut index,
        100,
        Event::ReasoningSummary {
            key: "snapshot-only".into(),
            text: "Published summary".into(),
            summary_index: 0,
            snapshot: true,
            replace: false,
            summary: false,
            elapsed_ms: None,
        },
    );
    assert_eq!(session.messages[6].elapsed_ms, None);
    let saved = serde_json::to_string(&session.messages).unwrap();
    let restored: Vec<crate::domain::ChatItem> = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored[0].elapsed_ms, Some(7000));
    assert_eq!(restored[5].elapsed_ms, Some(3000));
}

#[test]
fn codex_reasoning_summary_uses_native_timing_or_keeps_it_unknown() {
    use std::time::{Duration, Instant};
    let mut session = crate::app::tests::session();
    session.provider = Provider::Codex;
    session.messages.clear();
    let now = Instant::now();
    let mut index = MessageIndex::default();
    for (key, native) in [("known", Some(3380)), ("unknown", None)] {
        for snapshot in [false, true] {
            let event = Event::ReasoningSummary {
                key: key.into(),
                text: "**Capping output length**".into(),
                summary_index: 0,
                snapshot,
                replace: false,
                summary: true,
                elapsed_ms: native,
            };
            index.observe_reasoning(&mut session, "turn", &event, now);
            apply_event_indexed(&mut session, "turn", event, &mut index);
        }
    }
    assert_eq!(session.messages[0].elapsed_ms, Some(3380));
    assert_eq!(session.messages[1].elapsed_ms, None);
    assert!(
        index
            .reasoning_elapsed("turn-unknown", now + Duration::from_secs(30))
            .is_none()
    );
    let saved = serde_json::to_string(&session.messages).unwrap();
    let restored: Vec<crate::domain::ChatItem> = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored[0].reasoning_summary, Some(true));
    assert_eq!(
        super::reasoning_label_key(&restored[0], true, false),
        "chat.reasoning_summary"
    );
    assert_eq!(restored[0].elapsed_ms, Some(3380));
    assert_eq!(restored[1].elapsed_ms, None);
    let mut item = restored[0].clone();
    item.reasoning_summary = Some(false);
    assert_eq!(
        super::reasoning_label_key(&item, true, false),
        "chat.reasoning"
    );
    assert_eq!(
        super::reasoning_label_key(&item, true, true),
        "chat.thinking"
    );
    item.reasoning_summary = None;
    assert_eq!(
        super::reasoning_label_key(&item, true, false),
        "chat.reasoning_summary"
    );
    assert_eq!(
        super::reasoning_label_key(&item, false, false),
        "chat.reasoning"
    );
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
                    replace: false,
                    summary: false,
                    elapsed_ms: None,
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
                replace: false,
                summary: false,
                elapsed_ms: None,
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
                replace: false,
                summary: false,
                elapsed_ms: None,
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
                    replace: false,
                    summary: false,
                    elapsed_ms: None,
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
