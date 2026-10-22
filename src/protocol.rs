//! Provider-specific wire formats translated into presentation events.
use crate::domain::Provider;
use serde_json::{Value, json};

/// Which wire request an approval answers. The response shape differs per
/// request type, so the decision must not be inferred from the ticket text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalKind {
    Command,
    FileChange,
    Permissions,
    ClaudeTool,
}
#[derive(Clone, Debug)]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub wire_id: Value,
    pub input: Value,
    pub allow_once: bool,
    pub kind: ApprovalKind,
}
#[derive(Clone, Debug)]
pub enum Event {
    NativeId(String),
    Delta {
        key: String,
        text: String,
    },
    Text {
        key: String,
        text: String,
    },
    Tool {
        key: String,
        title: String,
        detail: String,
        status: String,
    },
    ToolInput {
        key: String,
        input: String,
    },
    Approval(Approval),
    ApprovalCancelled(String),
    ApprovalResolved {
        id: String,
        allow: bool,
    },
    Notice(String),
    Finished {
        error: Option<String>,
        cancelled: bool,
    },
}
pub struct Step {
    pub writes: Vec<Value>,
    pub events: Vec<Event>,
    pub done: bool,
    pub error: Option<String>,
}
impl Step {
    fn new() -> Self {
        Self {
            writes: vec![],
            events: vec![],
            done: false,
            error: None,
        }
    }
}

pub struct Protocol {
    provider: Provider,
    cwd: String,
    prompt: String,
    native_id: Option<String>,
    model: Option<String>,
    thread_id: Option<String>,
    turn_id: Option<String>,
    claude_message: String,
    tool_inputs: std::collections::HashMap<String, Value>,
    #[cfg(test)]
    pub approval_test: bool,
    #[cfg(test)]
    pub test_model: Option<String>,
    #[cfg(test)]
    pub test_permission_mode: Option<String>,
}
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}
fn id_text(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}

impl Protocol {
    pub fn new(provider: Provider, cwd: String, prompt: String, native_id: Option<String>) -> Self {
        Self {
            provider,
            cwd,
            prompt,
            native_id,
            model: None,
            thread_id: None,
            turn_id: None,
            claude_message: String::new(),
            tool_inputs: Default::default(),
            #[cfg(test)]
            approval_test: false,
            #[cfg(test)]
            test_model: None,
            #[cfg(test)]
            test_permission_mode: None,
        }
    }
    pub fn with_model(mut self, model: Option<String>) -> Self {
        self.model = model;
        self
    }
    fn claude_permission_mode(&self) -> &str {
        #[cfg(test)]
        if let Some(mode) = &self.test_permission_mode {
            return mode;
        }
        "default"
    }
    pub fn args(&self) -> Vec<String> {
        match self.provider {
            Provider::Codex => vec!["app-server".into(), "--stdio".into()],
            Provider::Claude => {
                let mut args: Vec<String> = [
                    "--print",
                    "--verbose",
                    "--output-format",
                    "stream-json",
                    "--input-format",
                    "stream-json",
                    "--include-partial-messages",
                    "--permission-prompt-tool",
                    "stdio",
                    "--permission-mode",
                ]
                .map(String::from)
                .into();
                args.push(self.claude_permission_mode().into());
                if let Some(model) = &self.model {
                    args.push("--model".into());
                    args.push(model.clone());
                }
                #[cfg(test)]
                if let Some(model) = &self.test_model {
                    args.push("--model".into());
                    args.push(model.clone());
                }
                if let Some(id) = &self.native_id {
                    args.push(format!("--resume={id}"));
                }
                args
            }
        }
    }
    pub fn initialize(&self) -> Value {
        match self.provider {
            Provider::Codex => {
                json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"conduit","title":"Conduit","version":"0.1.0"},"capabilities":{"experimentalApi":false}}})
            }
            Provider::Claude => {
                json!({"type":"control_request","request_id":"conduit-init","request":{"subtype":"initialize","hooks":{}}})
            }
        }
    }
    pub fn accept(&mut self, value: Value) -> Step {
        match self.provider {
            Provider::Codex => self.codex(value),
            Provider::Claude => self.claude(value),
        }
    }
    fn codex(&mut self, v: Value) -> Step {
        let mut step = Step::new();
        if let Some(error) = v.get("error")
            && v.get("id").is_some()
        {
            step.done = true;
            step.error = Some(pretty(error));
            return step;
        }
        match v.get("id").and_then(Value::as_u64) {
            Some(1) if v.get("result").is_some() => {
                step.writes.push(json!({"method":"initialized"}));
                let mut params = json!({"cwd":self.cwd,"approvalPolicy":"on-request","approvalsReviewer":"user","sandbox":"workspace-write"});
                #[cfg(test)]
                if self.approval_test {
                    params["approvalPolicy"] = json!("untrusted");
                    params["sandbox"] = json!("read-only");
                }
                if let Some(model) = &self.model {
                    params["model"] = json!(model);
                }
                let method = if let Some(id) = &self.native_id {
                    params["threadId"] = json!(id);
                    "thread/resume"
                } else {
                    "thread/start"
                };
                step.writes
                    .push(json!({"id":2,"method":method,"params":params}));
            }
            Some(2) if v.get("result").is_some() => {
                let id = v
                    .pointer("/result/thread/id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if id.is_empty() {
                    step.done = true;
                    step.error = Some("Codex 未返回 thread ID，未发送任务".into());
                    return step;
                }
                if self
                    .native_id
                    .as_deref()
                    .is_some_and(|expected| expected != id)
                {
                    step.done = true;
                    step.error =
                        Some("Codex 返回了不同的 thread ID，未发送任务。请新建会话。".into());
                    return step;
                }
                self.thread_id = Some(id.into());
                step.events.push(Event::NativeId(id.into()));
                let mut params = json!({"threadId":id,"input":[{"type":"text","text":self.prompt,"text_elements":[]}]});
                if let Some(model) = &self.model {
                    params["model"] = json!(model);
                }
                step.writes
                    .push(json!({"id":3,"method":"turn/start","params":params}));
            }
            Some(3) if v.get("result").is_some() => {
                self.turn_id = v
                    .pointer("/result/turn/id")
                    .and_then(Value::as_str)
                    .map(String::from);
            }
            _ => {}
        }
        let method = text(&v, "method");
        let p = &v["params"];
        // Ignore notifications belonging to a different thread or turn.
        if let Some(id) = p.get("threadId").and_then(Value::as_str)
            && self
                .thread_id
                .as_deref()
                .is_some_and(|expected| expected != id)
        {
            return step;
        }
        let notification_turn = p
            .get("turnId")
            .and_then(Value::as_str)
            .or_else(|| p.pointer("/turn/id").and_then(Value::as_str));
        if let (Some(expected), Some(actual)) = (self.turn_id.as_deref(), notification_turn)
            && expected != actual
        {
            return step;
        }
        match method {
            "item/agentMessage/delta" => step.events.push(Event::Delta {
                key: text(p, "itemId").into(),
                text: text(p, "delta").into(),
            }),
            "item/started" | "item/completed" => {
                let item = &p["item"];
                let key = text(item, "id").to_string();
                let kind = text(item, "type");
                if kind == "agentMessage" {
                    if method == "item/completed" {
                        step.events.push(Event::Text {
                            key,
                            text: text(item, "text").into(),
                        });
                    }
                } else if kind != "userMessage" && kind != "reasoning" {
                    let input = match kind {
                        "commandExecution" => json!({"command":item["command"],"cwd":item["cwd"]}),
                        "fileChange" => item["changes"].clone(),
                        "mcpToolCall" => {
                            json!({"server":item["server"],"tool":item["tool"],"arguments":item["arguments"]})
                        }
                        _ => item.clone(),
                    };
                    if method == "item/started" {
                        self.tool_inputs.insert(key.clone(), input.clone());
                        step.events.push(Event::ToolInput {
                            key: key.clone(),
                            input: pretty(&input),
                        });
                    }
                    let title = match kind {
                        "commandExecution" => text(item, "command"),
                        "fileChange" => "修改文件",
                        "mcpToolCall" => text(item, "tool"),
                        _ => kind,
                    }
                    .to_string();
                    let detail = match kind {
                        "commandExecution" if method == "item/completed" => pretty(
                            &json!({"output":item["aggregatedOutput"],"exitCode":item["exitCode"]}),
                        ),
                        "commandExecution" => String::new(),
                        "fileChange" => pretty(&item["changes"]),
                        "mcpToolCall" if method == "item/completed" => {
                            pretty(&json!({"result":item["result"],"error":item["error"]}))
                        }
                        "mcpToolCall" => String::new(),
                        _ => pretty(item),
                    };
                    step.events.push(Event::Tool {
                        key,
                        title,
                        detail,
                        status: if method == "item/completed" {
                            text(item, "status").to_string()
                        } else {
                            "运行中".into()
                        },
                    });
                }
            }
            "item/commandExecution/outputDelta" => step.events.push(Event::Tool {
                key: text(p, "itemId").into(),
                title: "命令输出".into(),
                detail: text(p, "delta").into(),
                status: "输出中".into(),
            }),
            "turn/completed" => {
                step.done = true;
                if text(&p["turn"], "status") != "completed" {
                    step.error = Some(
                        p["turn"]
                            .get("error")
                            .filter(|v| !v.is_null())
                            .map(pretty)
                            .unwrap_or_else(|| text(&p["turn"], "status").into()),
                    );
                }
            }
            "error" => {
                step.events.push(Event::Notice(pretty(p)));
            }
            "serverRequest/resolved" => {
                if let Some(id) = p
                    .get("requestId")
                    .filter(|v| v.is_string() || v.is_i64() || v.is_u64())
                {
                    step.events.push(Event::ApprovalCancelled(id_text(id)));
                }
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                if !v
                    .get("id")
                    .is_some_and(|v| v.is_string() || v.is_i64() || v.is_u64())
                {
                    step.done = true;
                    step.error = Some("Codex 审批请求缺少有效 ID，已停止".into());
                    return step;
                }
                let mut details = p.clone();
                if let Some(input) = self.tool_inputs.get(text(p, "itemId")) {
                    details["toolInput"] = input.clone();
                }
                step.events.push(Event::Approval(Approval {
                    id: id_text(&v["id"]),
                    wire_id: v["id"].clone(),
                    input: p.clone(),
                    title: if p
                        .get("networkApprovalContext")
                        .is_some_and(|v| !v.is_null())
                    {
                        "请求网络访问"
                    } else if method.contains("commandExecution") {
                        "请求执行命令"
                    } else {
                        "请求修改文件"
                    }
                    .into(),
                    detail: pretty(&details),
                    allow_once: p
                        .get("availableDecisions")
                        .and_then(Value::as_array)
                        .is_none_or(|decisions| {
                            decisions.iter().any(|v| v.as_str() == Some("accept"))
                        }),
                    kind: if method.contains("commandExecution") {
                        ApprovalKind::Command
                    } else {
                        ApprovalKind::FileChange
                    },
                }));
            }
            "item/permissions/requestApproval" => {
                if !v
                    .get("id")
                    .is_some_and(|v| v.is_string() || v.is_i64() || v.is_u64())
                {
                    step.done = true;
                    step.error = Some("Codex 权限扩展请求缺少有效 ID，已停止".into());
                    return step;
                }
                // Codex asks to extend the sandbox (additional network or
                // filesystem access) beyond the current permission profile.
                // Approving echoes the requested profile back for this turn;
                // denying grants an empty profile (the schema has no deny enum
                // and an unknown scope value would fail deserialization).
                let mut details = p.clone();
                if let Some(input) = self.tool_inputs.get(text(p, "itemId")) {
                    details["toolInput"] = input.clone();
                }
                step.events.push(Event::Approval(Approval {
                    id: id_text(&v["id"]),
                    wire_id: v["id"].clone(),
                    input: p.clone(),
                    title: "请求扩展权限".into(),
                    detail: pretty(&details),
                    allow_once: true,
                    kind: ApprovalKind::Permissions,
                }));
            }
            _ if v.get("id").is_some() && !method.is_empty() => {
                // Unknown interactive protocols are rejected, never silently approved.
                step.writes.push(json!({"id":v["id"],"error":{"code":-32601,"message":"Conduit 尚不支持该交互，请取消任务并使用官方 CLI"}}));
                step.events
                    .push(Event::Notice(format!("尚不支持的 CLI 交互：{method}")));
            }
            _ => {}
        }
        step
    }
    fn claude(&mut self, v: Value) -> Step {
        let mut step = Step::new();
        if let Some(id) = v
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            if self
                .native_id
                .as_deref()
                .is_some_and(|expected| expected != id)
            {
                step.done = true;
                step.error =
                    Some("Claude 返回了不同的 session ID，已停止，避免丢失上下文。".into());
                return step;
            }
            if self.thread_id.as_deref() != Some(id) {
                self.thread_id = Some(id.into());
                step.events.push(Event::NativeId(id.into()));
            }
        }
        match text(&v, "type") {
            "control_response" if text(&v["response"], "request_id") == "conduit-init" => {
                if text(&v["response"], "subtype") == "error" {
                    step.done = true;
                    step.error = Some(pretty(&v["response"]));
                } else {
                    step.writes.push(
                        json!({"type":"user","message":{"role":"user","content":self.prompt}}),
                    );
                }
            }
            "stream_event" => {
                let event = &v["event"];
                if text(event, "type") == "message_start" {
                    self.claude_message = text(&event["message"], "id").into();
                }
                let key = format!(
                    "{}-{}",
                    self.claude_message,
                    event["index"].as_u64().unwrap_or(0)
                );
                match text(event, "type") {
                    "content_block_delta" if text(&event["delta"], "type") == "text_delta" => {
                        step.events.push(Event::Delta {
                            key,
                            text: text(&event["delta"], "text").into(),
                        })
                    }
                    "content_block_start"
                        if text(&event["content_block"], "type") == "tool_use" =>
                    {
                        let block = &event["content_block"];
                        step.events.push(Event::Tool {
                            key: text(block, "id").into(),
                            title: text(block, "name").into(),
                            detail: String::new(),
                            status: "运行中".into(),
                        });
                    }
                    _ => {}
                }
            }
            "assistant" => {
                let message = &v["message"];
                let id = text(message, "id");
                if let Some(blocks) = message["content"].as_array() {
                    for (index, block) in blocks.iter().enumerate() {
                        match text(block, "type") {
                            "text" => step.events.push(Event::Text {
                                key: format!("{id}-{index}"),
                                text: text(block, "text").into(),
                            }),
                            "tool_use" => step.events.push(Event::Tool {
                                key: text(block, "id").into(),
                                title: text(block, "name").into(),
                                detail: String::new(),
                                status: "运行中".into(),
                            }),
                            _ => {}
                        }
                        if text(block, "type") == "tool_use" {
                            step.events.push(Event::ToolInput {
                                key: text(block, "id").into(),
                                input: pretty(&block["input"]),
                            });
                        }
                    }
                }
            }
            "user" => {
                if let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) {
                    for block in blocks.iter().filter(|b| text(b, "type") == "tool_result") {
                        step.events.push(Event::Tool {
                            key: text(block, "tool_use_id").into(),
                            title: String::new(),
                            detail: if let Some(t) = block["content"].as_str() {
                                t.into()
                            } else {
                                pretty(&block["content"])
                            },
                            status: if block["is_error"].as_bool().unwrap_or(false) {
                                "失败"
                            } else {
                                "完成"
                            }
                            .into(),
                        });
                    }
                }
            }
            "control_request" => {
                let request = &v["request"];
                let id = text(&v, "request_id");
                if text(request, "subtype") == "can_use_tool" {
                    if id.is_empty() {
                        step.done = true;
                        step.error = Some("Claude 审批请求缺少有效 ID，已停止".into());
                        return step;
                    }
                    step.events.push(Event::Approval(Approval {
                        id: id.into(),
                        wire_id: json!(id),
                        title: format!("请求使用 {}", text(request, "tool_name")),
                        detail: pretty(request),
                        input: request["input"].clone(),
                        allow_once: true,
                        kind: ApprovalKind::ClaudeTool,
                    }));
                } else {
                    step.writes.push(json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":"Unsupported control request"}}));
                    step.events.push(Event::Notice(format!(
                        "不支持的 Claude 控制请求：{}",
                        text(request, "subtype")
                    )));
                }
            }
            "control_cancel_request" => step
                .events
                .push(Event::ApprovalCancelled(text(&v, "request_id").into())),
            "result" => {
                step.done = true;
                if v["is_error"].as_bool().unwrap_or(false)
                    || text(&v, "subtype").starts_with("error")
                {
                    step.error = Some(
                        v.get("errors")
                            .map(pretty)
                            .unwrap_or_else(|| text(&v, "result").into()),
                    );
                }
            }
            _ => {}
        }
        step
    }
    pub fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        match self.provider {
            Provider::Codex => match approval.kind {
                ApprovalKind::Command | ApprovalKind::FileChange => {
                    json!({"id":approval.wire_id,"result":{"decision":if allow { "accept" } else { "decline" }}})
                }
                ApprovalKind::Permissions => {
                    // Grant nothing on denial: the response schema has no deny
                    // enum, an empty profile is the documented safe denial.
                    let permissions = if allow {
                        approval
                            .input
                            .get("permissions")
                            .cloned()
                            .unwrap_or_else(|| json!({}))
                    } else {
                        json!({})
                    };
                    json!({"id":approval.wire_id,"result":{"permissions":permissions,"scope":"turn"}})
                }
                ApprovalKind::ClaudeTool => unreachable!("Claude approvals are handled below"),
            },
            Provider::Claude => {
                json!({"type":"control_response","response":{"subtype":"success","request_id":approval.wire_id,
                "response": if allow { json!({"behavior":"allow","updatedInput":approval.input}) } else { json!({"behavior":"deny","message":"用户拒绝了本次操作"}) }}})
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_override_reaches_both_wire_formats() {
        let claude = Protocol::new(Provider::Claude, "C:\\work".into(), "hello".into(), None)
            .with_model(Some("claude-custom".into()));
        let args = claude.args();
        let position = args.iter().position(|arg| arg == "--model").unwrap();
        assert_eq!(args[position + 1], "claude-custom");
        assert!(
            !Protocol::new(Provider::Claude, "C:\\work".into(), "hello".into(), None)
                .args()
                .contains(&"--model".to_string())
        );

        let mut codex = Protocol::new(Provider::Codex, "C:\\work".into(), "hello".into(), None)
            .with_model(Some("gpt-custom".into()));
        let step = codex.accept(json!({"id":1,"result":{}}));
        assert_eq!(step.writes[1]["method"], "thread/start");
        assert_eq!(step.writes[1]["params"]["model"], "gpt-custom");
        let step = codex.accept(json!({"id":2,"result":{"thread":{"id":"thread-1"}}}));
        assert_eq!(step.writes[0]["params"]["model"], "gpt-custom");

        let mut resumed = Protocol::new(
            Provider::Codex,
            "C:\\work".into(),
            "hello".into(),
            Some("thread-1".into()),
        )
        .with_model(Some("gpt-custom".into()));
        let step = resumed.accept(json!({"id":1,"result":{}}));
        assert_eq!(step.writes[1]["method"], "thread/resume");
        assert_eq!(step.writes[1]["params"]["model"], "gpt-custom");
    }
    #[test]
    fn codex_ignores_other_threads_and_previous_turn_completion() {
        let mut p = Protocol::new(Provider::Codex, "C:\\work".into(), "hello".into(), None);
        p.accept(json!({"id":2,"result":{"thread":{"id":"thread-1"}}}));
        p.accept(json!({"id":3,"result":{"turn":{"id":"turn-2"}}}));
        let previous = p.accept(json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}}));
        assert!(!previous.done);
        let other = p.accept(json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-other","turnId":"turn-2","itemId":"m","delta":"ignored"}}));
        assert!(other.events.is_empty());
        let current = p.accept(json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-2","status":"completed"}}}));
        assert!(current.done && current.error.is_none());
    }
    #[test]
    fn codex_exact_resume_and_no_silent_new_thread() {
        let mut protocol = Protocol::new(
            Provider::Codex,
            "C:\\work".into(),
            "hello".into(),
            Some("known-thread".into()),
        );
        let step = protocol.accept(json!({"id":1,"result":{}}));
        assert_eq!(step.writes[1]["method"], "thread/resume");
        assert_eq!(step.writes[1]["params"]["threadId"], "known-thread");
        assert_eq!(step.writes[1]["params"]["sandbox"], "workspace-write");
        let step = protocol.accept(json!({"id":2,"result":{"thread":{"id":"other-thread"}}}));
        assert!(step.done && step.error.is_some());
        assert!(step.writes.is_empty());
    }
    #[test]
    fn codex_approval_preserves_request_id_and_only_grants_once() {
        let mut protocol = Protocol::new(Provider::Codex, "C:\\work".into(), "hello".into(), None);
        let step = protocol.accept(json!({"id":"server-42","method":"item/commandExecution/requestApproval","params":{"command":"echo test","itemId":"item1"}}));
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing approval");
        };
        assert_eq!(
            protocol.approval_response(approval, true),
            json!({"id":"server-42","result":{"decision":"accept"}})
        );
        assert_eq!(
            protocol.approval_response(approval, false)["result"]["decision"],
            "decline"
        );
    }
    #[test]
    fn codex_file_change_approval_and_resolved_request_keep_exact_identity() {
        let mut protocol = Protocol::new(Provider::Codex, "C:\\work".into(), "hello".into(), None);
        let step = protocol.accept(json!({"id":41,"method":"item/fileChange/requestApproval","params":{"itemId":"file-1","changes":[{"path":"src/main.rs","kind":"update"}],"availableDecisions":["accept","decline"]}}));
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing file change approval");
        };
        assert_eq!(approval.title, "请求修改文件");
        assert!(approval.allow_once);
        assert_eq!(approval.wire_id, json!(41));
        assert_eq!(
            protocol.approval_response(approval, true),
            json!({"id":41,"result":{"decision":"accept"}})
        );

        let step = protocol.accept(json!({"id":42,"method":"item/fileChange/requestApproval","params":{"itemId":"file-2","availableDecisions":["decline"]}}));
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing restricted approval");
        };
        assert!(!approval.allow_once);

        let step =
            protocol.accept(json!({"method":"serverRequest/resolved","params":{"requestId":41}}));
        assert!(matches!(&step.events[0], Event::ApprovalCancelled(id) if id == "41"));
    }
    #[test]
    fn claude_control_response_preserves_tool_input() {
        let mut protocol = Protocol::new(Provider::Claude, "C:\\work".into(), "hello".into(), None);
        let step = protocol.accept(json!({"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Edit","input":{"file_path":"test.rs","new_string":"a"}}}));
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing approval");
        };
        let response = protocol.approval_response(approval, true);
        assert_eq!(response["response"]["request_id"], "req-1");
        assert_eq!(
            response["response"]["response"]["updatedInput"],
            approval.input
        );
        assert!(
            response["response"]["response"]
                .get("updatedPermissions")
                .is_none()
        );
        assert_eq!(
            protocol.approval_response(approval, false)["response"]["response"]["behavior"],
            "deny"
        );
    }
    #[test]
    fn claude_partial_and_snapshot_share_message_key() {
        let mut protocol = Protocol::new(Provider::Claude, "C:\\work".into(), "hello".into(), None);
        protocol.accept(
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}),
        );
        let partial = protocol.accept(json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}}));
        let snapshot = protocol.accept(json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hello"}]}}));
        assert!(matches!(&partial.events[0], Event::Delta { key, .. } if key == "m1-0"));
        assert!(matches!(&snapshot.events[0], Event::Text { key, .. } if key == "m1-0"));
    }
    #[test]
    fn unknown_interactive_requests_are_never_auto_approved() {
        let mut protocol = Protocol::new(Provider::Codex, "C:\\work".into(), "hello".into(), None);
        let step = protocol.accept(json!({"id":88,"method":"unsupported/approval","params":{}}));
        assert_eq!(step.writes[0]["error"]["code"], -32601);
        let mut protocol = Protocol::new(
            Provider::Claude,
            "C:\\work".into(),
            "hello".into(),
            Some("expected".into()),
        );
        let step =
            protocol.accept(json!({"type":"system","subtype":"init","session_id":"different"}));
        assert!(step.done && step.error.is_some());
        assert!(step.events.is_empty());
    }
    #[test]
    fn codex_permissions_approval_grants_requested_profile_or_nothing() {
        let mut protocol = Protocol::new(Provider::Codex, "C:\\work".into(), "hello".into(), None);
        // Shape follows PermissionsRequestApprovalParams from the 0.163.0
        // app-server schema: additional filesystem/network access beyond the
        // current sandbox profile.
        let request = json!({
            "id": 77,
            "method": "item/permissions/requestApproval",
            "params": {
                "cwd": "C:\\work",
                "itemId": "item-1",
                "permissions": {
                    "fileSystem": {"write": ["C:\\work\\logs"]},
                    "network": {"enabled": true}
                },
                "reason": "write logs outside the sandbox",
                "startedAtMs": 1,
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        });
        let step = protocol.accept(request);
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing permissions approval");
        };
        assert_eq!(approval.wire_id, json!(77));
        assert_eq!(approval.kind, ApprovalKind::Permissions);
        assert_eq!(approval.title, "请求扩展权限");
        let allowed = protocol.approval_response(approval, true);
        assert_eq!(allowed["id"], 77);
        assert_eq!(allowed["result"]["scope"], "turn");
        assert_eq!(
            allowed["result"]["permissions"],
            approval.input["permissions"]
        );
        // Denying must grant an empty profile; the schema has no deny enum and
        // an invented scope value would fail deserialization on the CLI side.
        let denied = protocol.approval_response(approval, false);
        assert_eq!(denied["id"], 77);
        assert_eq!(denied["result"]["permissions"], json!({}));
        assert_eq!(denied["result"]["scope"], "turn");
        // A permissions request without an ID stops the run instead of being
        // silently approved.
        let missing =
            protocol.accept(json!({"method":"item/permissions/requestApproval","params":{}}));
        assert!(missing.done && missing.error.is_some());
        assert!(missing.writes.is_empty());
    }
}
