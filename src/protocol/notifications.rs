//! Optional notifications never become assistant text or imply turn success.
use super::{Event, Protocol, Step, presentation::bounded, text};
use serde_json::Value;

impl Protocol {
    pub(super) fn claude_notification(&mut self, value: &Value, step: &mut Step) {
        let kind = text(value, "type");
        match kind {
            "system" => match text(value, "subtype") {
                "init" => {}
                "status" => {
                    let key = match text(value, "status") {
                        "compacting" => "protocol.activity_compacting",
                        _ => "protocol.activity_requesting",
                    };
                    self.presentation.activity(key, &[], step);
                    if value["compact_result"] == "failed" {
                        step.events.push(Event::Failure(
                            conduit::i18n::text("protocol.compaction_failed").into(),
                        ));
                    }
                }
                "compact_boundary" => self.presentation.notice(
                    "claude:compacted",
                    conduit::i18n::text("commands.compacted"),
                    step,
                ),
                "api_retry" => self.presentation.activity(
                    "protocol.activity_retry",
                    &[
                        ("attempt", value["attempt"].to_string()),
                        ("max", value["max_retries"].to_string()),
                        ("delay", value["retry_delay_ms"].to_string()),
                    ],
                    step,
                ),
                "hook_started" | "hook_progress" => self.presentation.activity(
                    "protocol.activity_tool",
                    &[("name", bounded(text(value, "hook_name")))],
                    step,
                ),
                "hook_response" => {
                    if value["outcome"] == "error" {
                        self.presentation.session_notice(
                            &format!("claude:hook_error:{}", bounded(text(value, "hook_name"))),
                            conduit::i18n::format(
                                "protocol.hook_failed",
                                &[("name", bounded(text(value, "hook_name")))],
                            ),
                            step,
                        );
                    }
                }
                "informational" => {
                    let content = bounded(text(value, "content"));
                    if value["prevent_continuation"] == true {
                        step.done = true;
                        step.error = Some(content);
                    } else {
                        self.presentation
                            .notice(&format!("claude:info:{content}"), content, step);
                    }
                }
                subtype => self
                    .presentation
                    .ignored(&format!("Claude/system/{subtype}")),
            },
            "tool_progress" if value["parent_tool_use_id"].is_null() => self.presentation.activity(
                "protocol.activity_tool",
                &[("name", bounded(text(value, "tool_name")))],
                step,
            ),
            "auth_status" => {
                // Auth output may contain credentials or a device login code.
                // Only expose state; never copy output/error payloads.
                if value.get("error").is_some_and(|v| !v.is_null() && v != "") {
                    step.events.push(Event::Failure(
                        conduit::i18n::text("protocol.auth_failed").into(),
                    ));
                } else if value["isAuthenticating"] == true {
                    self.presentation
                        .activity("protocol.activity_authenticating", &[], step);
                } else {
                    self.presentation
                        .activity("protocol.activity_requesting", &[], step);
                }
            }
            "rate_limit_event" => {
                let info = &value["rate_limit_info"];
                if matches!(text(info, "status"), "allowed_warning" | "rejected") {
                    self.presentation.notice(
                        "claude:rate_limit",
                        conduit::i18n::text("protocol.rate_limited"),
                        step,
                    );
                }
            }
            // Keep-alives and summaries duplicate data already represented by tools.
            "keep_alive" | "tool_use_summary" | "tool_progress" => {}
            "" => {}
            _ => self.presentation.ignored(&format!("Claude/{kind}")),
        }
    }

    pub(super) fn codex_notification(&mut self, method: &str, params: &Value, step: &mut Step) {
        match method {
            ""
            | "thread/started"
            | "turn/diff/updated"
            | "item/reasoning/textDelta"
            | "item/reasoning/summaryPartAdded" => {}
            "item/mcpToolCall/progress" => self.presentation.activity(
                "protocol.activity_tool",
                &[("name", bounded(text(params, "message")))],
                step,
            ),
            "item/fileChange/outputDelta" | "item/fileChange/patchUpdated" => self
                .presentation
                .activity("protocol.activity_file_change", &[], step),
            "thread/compacted" => self.presentation.notice(
                "codex:compacted",
                conduit::i18n::text("commands.compacted"),
                step,
            ),
            // Addressed to the client author (Conduit's own request shapes), not
            // to the person chatting; they cannot act on it.
            "deprecationNotice" => self.presentation.ignored("Codex/deprecationNotice"),
            "warning" | "configWarning" | "guardianWarning" => {
                let message = params["message"]
                    .as_str()
                    .or_else(|| params["summary"].as_str())
                    .unwrap_or(method);
                self.presentation.session_notice(
                    &format!("codex:{method}:{}", bounded(message)),
                    bounded(message),
                    step,
                );
            }
            "thread/status/changed" => {
                let key = if params["status"]["type"] == "systemError" {
                    "protocol.native_system_error"
                } else {
                    "protocol.activity_requesting"
                };
                if params["status"]["type"] == "systemError" {
                    self.presentation
                        .notice("codex:system_error", conduit::i18n::text(key), step);
                } else if params["status"]["type"] == "active" {
                    self.presentation.activity(key, &[], step);
                }
            }
            "mcpServer/startupStatus/updated" => {
                if params["status"] == "failed" {
                    self.presentation
                        .ignored("Codex/mcpServer/startupStatus/updated/failed");
                } else if params["status"] == "starting" {
                    self.presentation.activity(
                        "protocol.activity_tool",
                        &[("name", bounded(text(params, "name")))],
                        step,
                    );
                }
            }
            "model/rerouted" => self.presentation.notice(
                "codex:rerouted",
                conduit::i18n::format(
                    "protocol.model_rerouted",
                    &[("model", bounded(text(params, "toModel")))],
                ),
                step,
            ),
            "hook/started" | "hook/completed" => {
                let run = &params["run"];
                let name = bounded(text(run, "eventName"));
                if matches!(text(run, "status"), "failed" | "blocked") {
                    // Keyed by event, not run: a broken hook fails on every
                    // tool call and must not add a row each time.
                    self.presentation.session_notice(
                        &format!("codex:hook:{name}"),
                        conduit::i18n::format(
                            if run["status"] == "blocked" {
                                "protocol.hook_blocked"
                            } else {
                                "protocol.hook_failed"
                            },
                            &[("name", name)],
                        ),
                        step,
                    );
                } else if run["status"] == "running" {
                    self.presentation
                        .activity("protocol.activity_tool", &[("name", name)], step);
                }
            }
            _ => self.presentation.ignored(&format!("Codex/{method}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Provider;
    use serde_json::json;

    #[test]
    fn claude_status_retry_tool_and_auth_progress_do_not_become_replies() {
        let mut p = Protocol::new(Provider::Claude, "C:/work".into(), "hello".into(), None);
        for notification in [
            json!({"type":"system","subtype":"status","status":"compacting"}),
            json!({"type":"system","subtype":"api_retry","attempt":1,"max_retries":3,"retry_delay_ms":200}),
            json!({"type":"tool_progress","tool_use_id":"tool","tool_name":"Bash","parent_tool_use_id":null,"elapsed_time_seconds":4}),
            json!({"type":"auth_status","isAuthenticating":true,"output":["SECRET_AUTH_CODE"]}),
        ] {
            let step = p.accept(notification);
            assert!(!step.done && step.error.is_none());
            assert!(
                matches!(&step.events[..], [Event::Activity(message)] if !message.is_empty() && !message.contains("SECRET_AUTH_CODE") && !message.contains('{'))
            );
        }
        let auth = p.accept(
            json!({"type":"auth_status","isAuthenticating":false,"error":"SECRET_AUTH_CODE"}),
        );
        assert!(
            matches!(&auth.events[..], [Event::Failure(message)] if !message.contains("SECRET_AUTH_CODE"))
        );
        assert!(p.accept(json!({"type":"keep_alive"})).events.is_empty());
        let compact = p.accept(json!({"type":"system","subtype":"compact_boundary","compact_metadata":{"pre_tokens":100}}));
        assert!(matches!(&compact.events[..], [Event::Notice(_)]));
        let limit =
            p.accept(json!({"type":"rate_limit_event","rate_limit_info":{"status":"rejected"}}));
        assert!(matches!(&limit.events[..], [Event::Notice(_)]));
        let stop = p.accept(json!({"type":"system","subtype":"informational","content":"cannot continue","prevent_continuation":true}));
        assert!(stop.done);
        assert_eq!(stop.error.as_deref(), Some("cannot continue"));
    }

    #[test]
    fn codex_optional_notifications_filter_foreign_turns_and_bound_unknown_types() {
        let mut p = Protocol::new(Provider::Codex, "C:/work".into(), "hello".into(), None);
        p.thread_id = Some("root".into());
        p.turn_id = Some("turn".into());
        for method in [
            "item/mcpToolCall/progress",
            "item/fileChange/outputDelta",
            "item/fileChange/patchUpdated",
        ] {
            let notification = json!({"method":method,"params":{"threadId":"root","turnId":"turn","itemId":"tool","message":"working","delta":"output"}});
            let mut foreign = notification.clone();
            foreign["params"]["threadId"] = json!("other");
            assert!(p.accept(foreign).events.is_empty());
            let mut old = notification.clone();
            old["params"]["turnId"] = json!("old");
            assert!(p.accept(old).events.is_empty());
            assert!(matches!(
                &p.accept(notification).events[..],
                [Event::Activity(_)]
            ));
        }
        let warning =
            p.accept(json!({"method":"configWarning","params":{"summary":"unsupported config"}}));
        assert!(
            matches!(&warning.events[..], [Event::SessionNotice { message, .. }] if message == "unsupported config")
        );
        let mcp = p.accept(json!({"method":"mcpServer/startupStatus/updated","params":{"name":"tools","status":"failed","error":"SECRET"}}));
        assert!(mcp.events.is_empty() && !mcp.done && mcp.error.is_none());
        let hook = json!({"method":"hook/completed","params":{"threadId":"root","turnId":"turn","run":{"id":"hook","eventName":"preToolUse","status":"failed","entries":[{"text":"SECRET"}]}}});
        assert!(
            matches!(&p.accept(hook.clone()).events[..], [Event::SessionNotice { message, .. }] if message.contains("preToolUse") && !message.contains("SECRET"))
        );
        // A hook that fails on every tool call is reported once, not per run.
        let mut again = hook;
        again["params"]["run"]["id"] = json!("hook-2");
        assert!(p.accept(again).events.is_empty());
        // Notifications need no reply: unknown or client-developer-facing ones
        // never reach the transcript, and they cannot crowd out real notices.
        for method in [
            "future/notification",
            "remoteControl/status/changed",
            "thread/goal/cleared",
            "thread/settings/updated",
            "turn/moderationMetadata",
            "account/rateLimits/updated",
            "item/commandExecution/terminalInteraction",
            "deprecationNotice",
        ] {
            let ignored = json!({"method":method,"params":{"threadId":"root","summary":"Full-history hydration is deprecated","data":"SECRET_BASE64"}});
            assert!(p.accept(ignored).events.is_empty(), "{method}");
        }
        for i in 0..100 {
            assert!(
                p.accept(json!({"method":format!("future/{i}"),"params":{}}))
                    .events
                    .is_empty()
            );
        }
        let guardian = p.accept(
            json!({"method":"guardianWarning","params":{"threadId":"root","message":"review required"}}),
        );
        assert!(
            matches!(&guardian.events[..], [Event::SessionNotice { message, .. }] if message == "review required")
        );
        // A request, unlike a notification, leaves the CLI waiting: stay visible.
        let request = p.accept(json!({"id":7,"method":"future/request","params":{}}));
        assert_eq!(request.writes[0]["error"]["code"], -32601);
        assert!(matches!(&request.events[..], [Event::Notice(_)]));
    }

    #[test]
    fn claude_unknown_notifications_stay_out_of_the_transcript() {
        let mut p = Protocol::new(Provider::Claude, "C:/work".into(), "hello".into(), None);
        for notification in [
            json!({"type":"system","subtype":"future_subtype","data":"SECRET"}),
            json!({"type":"future_type","data":"SECRET"}),
        ] {
            let step = p.accept(notification);
            assert!(step.events.is_empty() && !step.done && step.error.is_none());
        }
    }

    #[test]
    fn mcp_startup_failures_stay_out_of_the_transcript() {
        for locale in ["en", "zh-CN"] {
            let _locale = crate::test_support::scoped(locale);
            let mut protocol =
                Protocol::new(Provider::Codex, "C:/work".into(), "hello".into(), None);
            for reason in ["", "reauthenticationRequired", "futureReason"] {
                let notification = json!({"method":"mcpServer/startupStatus/updated","params":{
                    "name":"context7","status":"failed","failureReason":reason,"error":"SECRET"}});
                for _ in 0..2 {
                    let step = protocol.accept(notification.clone());
                    assert!(step.events.is_empty() && step.writes.is_empty());
                    assert!(!step.done && step.error.is_none());
                }
            }
            let starting = protocol
                .accept(json!({"method":"mcpServer/startupStatus/updated","params":{
                "name":"context7","status":"starting"}}));
            assert!(matches!(&starting.events[..], [Event::Activity(_)]));
        }
    }

    #[test]
    fn blocking_hooks_have_safe_localized_reasons() {
        for (locale, blocked) in [("en", "blocked"), ("zh-CN", "阻止了操作")] {
            let _locale = crate::test_support::scoped(locale);
            let mut protocol =
                Protocol::new(Provider::Codex, "C:/work".into(), "hello".into(), None);
            let step = protocol.accept(json!({"method":"hook/completed","params":{"run":{
                "eventName":"preToolUse","status":"blocked","statusMessage":"SECRET","entries":[{"text":"SECRET"}]}}}));
            assert!(
                matches!(&step.events[..], [Event::SessionNotice { message, .. }]
                if message.contains("preToolUse") && message.contains(blocked) && !message.contains("SECRET"))
            );
        }
    }

    #[test]
    fn unsupported_interaction_replies_follow_the_selected_language() {
        for (locale, expected) in [("en", "does not support"), ("zh-CN", "尚不支持")] {
            let _locale = crate::test_support::scoped(locale);
            let mut protocol =
                Protocol::new(Provider::Codex, "C:/work".into(), "hello".into(), None);
            let step = protocol.accept(json!({"id":7,"method":"future/request","params":{}}));
            assert!(
                step.writes[0]["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(expected)
            );
        }
    }
}
