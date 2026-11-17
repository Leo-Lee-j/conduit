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
                        step.events.push(Event::ErrorBanner(conduit::i18n::text(
                            "protocol.compaction_failed",
                        )));
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
                        self.presentation.notice(
                            "claude:hook_error",
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
                    .unsupported(&format!("Claude/system/{subtype}"), step),
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
                    step.events.push(Event::ErrorBanner(conduit::i18n::text(
                        "protocol.auth_failed",
                    )));
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
            _ => self
                .presentation
                .unsupported(&format!("Claude/{kind}"), step),
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
            "warning" | "configWarning" | "deprecationNotice" => {
                let message = params["message"]
                    .as_str()
                    .or_else(|| params["summary"].as_str())
                    .unwrap_or(method);
                self.presentation.notice(
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
                    self.presentation.notice(
                        &format!("codex:mcp:{}", text(params, "name")),
                        conduit::i18n::format(
                            "protocol.mcp_failed",
                            &[("name", bounded(text(params, "name")))],
                        ),
                        step,
                    );
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
                    self.presentation.notice(
                        &format!("codex:hook:{}", bounded(text(run, "id"))),
                        conduit::i18n::format("protocol.hook_failed", &[("name", name)]),
                        step,
                    );
                } else if run["status"] == "running" {
                    self.presentation
                        .activity("protocol.activity_tool", &[("name", name)], step);
                }
            }
            _ => self
                .presentation
                .unsupported(&format!("Codex/{method}"), step),
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
            matches!(&auth.events[..], [Event::ErrorBanner(message)] if !message.contains("SECRET_AUTH_CODE"))
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
            matches!(&warning.events[..], [Event::Notice(message)] if message == "unsupported config")
        );
        let mcp = p.accept(json!({"method":"mcpServer/startupStatus/updated","params":{"name":"tools","status":"failed","error":"SECRET"}}));
        assert!(
            matches!(&mcp.events[..], [Event::Notice(message)] if message.contains("tools") && !message.contains("SECRET")),
            "MCP events: {:?}",
            mcp.events
        );
        let hook = p.accept(json!({"method":"hook/completed","params":{"threadId":"root","turnId":"turn","run":{"id":"hook","eventName":"preToolUse","status":"failed","entries":[{"text":"SECRET"}]}}}));
        assert!(
            matches!(&hook.events[..], [Event::Notice(message)] if message.contains("preToolUse") && !message.contains("SECRET"))
        );
        let unknown = json!({"method":"future/notification","params":{"data":"SECRET_BASE64"}});
        assert!(
            matches!(&p.accept(unknown.clone()).events[..], [Event::Notice(message)] if !message.contains("SECRET_BASE64"))
        );
        assert!(p.accept(unknown).events.is_empty());
        let mut count = 0;
        for i in 0..100 {
            count += p
                .accept(json!({"method":format!("future/{i}"),"params":{}}))
                .events
                .len();
        }
        assert!(count < 32);
    }
}
