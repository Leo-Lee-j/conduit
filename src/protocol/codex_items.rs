//! Only explicitly supported item fields belong in the transcript. In
//! particular, a future item may contain opaque JSON or encoded media.
use super::*;
use crate::message_status::ToolStatus;

fn fields(value: &Value, names: &[&str]) -> Value {
    let mut result = serde_json::Map::new();
    for &name in names {
        if let Some(value) = value.get(name).filter(|value| !value.is_null()) {
            result.insert(name.into(), value.clone());
        }
    }
    Value::Object(result)
}

impl Protocol {
    pub(super) fn codex_item(&mut self, item: &Value, completed: bool, step: &mut Step) {
        let kind = text(item, "type");
        let key = text(item, "id").to_owned();
        match kind {
            "userMessage" | "hookPrompt" | "subAgentActivity" | "enteredReviewMode" => return,
            "contextCompaction" => {
                if completed {
                    self.presentation.notice(
                        "codex:compacted",
                        conduit::i18n::text("commands.compacted"),
                        step,
                    );
                } else {
                    self.presentation
                        .activity("protocol.activity_compacting", &[], step);
                }
                return;
            }
            "plan" => {
                if completed {
                    step.events.push(Event::Text {
                        key,
                        text: text(item, "text").into(),
                    });
                }
                return;
            }
            _ => {}
        }
        let mut title = kind.to_owned();
        let input = match kind {
            "commandExecution" => {
                title = text(item, "command").into();
                Some(fields(item, &["command", "cwd"]))
            }
            "fileChange" => {
                title = "修改文件".into();
                Some(item["changes"].clone())
            }
            "mcpToolCall" => {
                title = text(item, "tool").into();
                Some(fields(item, &["server", "tool", "arguments"]))
            }
            "dynamicToolCall" => {
                title = text(item, "tool").into();
                Some(fields(item, &["tool", "arguments"]))
            }
            "collabAgentToolCall" => {
                title = text(item, "tool").into();
                Some(fields(item, &["tool", "prompt", "receiverThreadIds"]))
            }
            "webSearch" => {
                let mut input = fields(item, &["query"]);
                input["action"] = fields(
                    &item["action"],
                    &["type", "query", "queries", "url", "pattern"],
                );
                Some(input)
            }
            "imageView" => Some(fields(item, &["path"])),
            "imageGeneration" => Some(fields(item, &["revisedPrompt", "savedPath"])),
            "sleep" => Some(fields(item, &["durationMs"])),
            _ => {
                self.presentation.ignored(&format!("Codex/item/{kind}"));
                title = if kind.is_empty() {
                    "unknown-item"
                } else {
                    kind
                }
                .chars()
                .take(96)
                .collect();
                None
            }
        };
        let known = input.is_some();
        if let Some(input) = input {
            // Completion-only items still need their safe input to be inspectable.
            if !completed || self.tool_inputs.get(&key) != Some(&input) {
                self.tool_inputs.insert(key.clone(), input.clone());
                step.events.push(Event::ToolInput {
                    key: key.clone(),
                    input: pretty(&input),
                });
            }
        }
        let detail = match kind {
            "commandExecution" if completed => pretty(&json!({
                "output": item["aggregatedOutput"], "exitCode": item["exitCode"]
            })),
            "fileChange" => pretty(&item["changes"]),
            "mcpToolCall" if completed => {
                let content = item["result"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|content| self.presentation.content(content, step))
                    .collect::<Vec<_>>()
                    .join("\n");
                if content.is_empty() && !item["result"]["structuredContent"].is_null() {
                    self.presentation
                        .unsupported("mcpToolCall/structuredContent", step);
                }
                // structuredContent and _meta are opaque and may embed media.
                // MCP's text/resource content is the readable representation.
                let error = item["error"]
                    .as_str()
                    .unwrap_or_else(|| text(&item["error"], "message"));
                pretty(&json!({"result":content,"error":error}))
            }
            "dynamicToolCall" if completed => item["contentItems"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|content| match text(content, "type") {
                    "inputText" => Some(text(content, "text").to_owned()),
                    _ => {
                        self.presentation.unsupported("dynamicToolCall/media", step);
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
            "imageGeneration" if completed => {
                if item["savedPath"].is_null() && !item["result"].is_null() {
                    self.presentation.unsupported("imageGeneration/media", step);
                }
                pretty(&fields(item, &["savedPath", "revisedPrompt"]))
            }
            _ => String::new(),
        };
        let status = if !completed {
            ToolStatus::Running
        } else if (kind == "dynamicToolCall" && item["success"] == false)
            || (kind == "commandExecution"
                && item["exitCode"].as_i64().is_some_and(|code| code != 0))
        {
            ToolStatus::Failed
        } else if !known {
            ToolStatus::Unspecified
        } else if text(item, "status").is_empty() {
            ToolStatus::Completed
        } else {
            ToolStatus::from_wire(text(item, "status"))
        };
        step.events.push(Event::Tool {
            key,
            title,
            detail,
            status,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn protocol() -> Protocol {
        Protocol::new(Provider::Codex, "C:/work".into(), "hello".into(), None)
    }

    fn item(protocol: &mut Protocol, item: Value, completed: bool) -> Step {
        protocol.accept(json!({"method":if completed { "item/completed" } else { "item/started" },"params":{"item":item}}))
    }

    #[test]
    fn unknown_items_keep_only_the_type_and_never_copy_opaque_fields() {
        let mut protocol = protocol();
        for completed in [false, true] {
            let step = item(
                &mut protocol,
                json!({"id":"future","type":"futureItem",
                "status":"SECRET_STATUS","payload":{"data":"SECRET_BASE64"}}),
                completed,
            );
            assert!(
                matches!(&step.events[..], [Event::Tool { title, detail, status, .. }]
                if title == "futureItem" && detail.is_empty() && *status == if completed { ToolStatus::Unspecified } else { ToolStatus::Running })
            );
            assert!(protocol.tool_inputs.is_empty());
            assert!(!format!("{:?}", step.events).contains("SECRET"));
        }
    }

    #[test]
    fn web_search_keeps_readable_queries_without_serializing_results() {
        let mut protocol = protocol();
        let step = item(
            &mut protocol,
            json!({"id":"web","type":"webSearch","query":"rust documentation",
            "action":{"type":"search","queries":["rust documentation"],"opaque":"SECRET"},
            "results":[{"image":"SECRET_BASE64"}]}),
            true,
        );
        assert!(step.events.iter().any(|event| matches!(event, Event::ToolInput { input, .. } if input.contains("rust documentation"))));
        assert!(!format!("{:?}", step.events).contains("SECRET"));
    }

    #[test]
    fn image_and_mcp_media_never_enter_tool_inputs_or_details() {
        let mut protocol = protocol();
        let image = json!({"id":"image","type":"imageGeneration","status":"completed",
            "savedPath":"C:/work/image.png","revisedPrompt":"a tree","result":"SECRET_BASE64".repeat(4096)});
        for completed in [false, true] {
            let step = item(&mut protocol, image.clone(), completed);
            assert!(!format!("{:?}", step.events).contains("SECRET_BASE64"));
            assert!(
                step.events
                    .iter()
                    .map(crate::event_queue::event_bytes)
                    .sum::<usize>()
                    < 4096
            );
        }
        let step = item(
            &mut protocol,
            json!({"id":"mcp","type":"mcpToolCall","status":"completed",
            "server":"tools","tool":"read","arguments":{"path":"notes"},"result":{
                "content":[{"type":"text","text":"readable result"},{"type":"image","data":"SECRET_BASE64"}],
                "structuredContent":{"image":"SECRET_BASE64"},"_meta":{"token":"SECRET_TOKEN"}}}),
            true,
        );
        assert!(step.events.iter().any(|event| matches!(event, Event::Tool { detail, .. } if detail.contains("readable result"))));
        assert!(!format!("{:?}", step.events).contains("SECRET"));
        assert!(!format!("{:?}", protocol.tool_inputs).contains("SECRET"));
    }

    #[test]
    fn codex_tool_outcomes_are_normalized_before_presentation() {
        for (native, exit, expected) in [
            ("declined", None, ToolStatus::Denied),
            ("completed", Some(0), ToolStatus::Completed),
            ("completed", Some(1), ToolStatus::Failed),
            ("failed", None, ToolStatus::Failed),
        ] {
            let step = item(
                &mut protocol(),
                json!({"id":"command","type":"commandExecution",
                "command":"cargo test","status":native,"exitCode":exit}),
                true,
            );
            assert!(
                step.events.iter().any(
                    |event| matches!(event, Event::Tool { status, .. } if *status == expected)
                )
            );
        }
    }
}
