//! Gemini CLI 0.71 ACP. The runtime owns processes, cancellation and approval tickets.
use super::*;
use crate::{
    domain::Usage,
    protocol::{AcpSettings, ApprovalKind, Event, presentation::Presentation},
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::{cell::Cell, collections::HashMap};
mod config;
#[cfg(test)]
mod tests;

pub fn gemini(request: AdapterRequest) -> Box<dyn CliAdapter> {
    Box::new(GeminiAdapter::new(request))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Initialize,
    Session,
    Mode,
    Model,
    Prompt,
    Done,
}

struct GeminiAdapter {
    request: AdapterRequest,
    stage: Stage,
    pending: u64,
    session: Option<String>,
    settings: AcpSettings,
    tools: HashMap<String, Value>,
    reply: u64,
    thoughts: crate::protocol::ThoughtStream,
    presentation: Presentation,
    context_window: u64,
    context_tokens: Option<u64>,
    cancelled: Cell<bool>,
    // Dropped only after ChatRuntime has stopped the complete process tree.
    config: Option<config::RunConfig>,
}

impl GeminiAdapter {
    fn new(mut request: AdapterRequest) -> Self {
        request.permission = request.tool_policy.permission(request.permission);
        Self {
            request,
            stage: Stage::Initialize,
            pending: 1,
            session: None,
            settings: Default::default(),
            tools: HashMap::new(),
            reply: 0,
            thoughts: Default::default(),
            presentation: Default::default(),
            context_window: 0,
            context_tokens: None,
            cancelled: Cell::new(false),
            config: None,
        }
    }

    fn restricted(&self) -> bool {
        self.request.text_only
            || self.request.permission == PermissionLevel::ReadOnly
            || self.request.tool_policy == ToolPolicy::Automation
    }

    fn mode(&self) -> &'static str {
        if !self.restricted() && self.request.permission == PermissionLevel::FullAccess {
            "yolo"
        } else {
            "default"
        }
    }

    fn fail(&mut self, key: &str) -> Step {
        self.stage = Stage::Done;
        let mut step = Step::new();
        self.thoughts.finish(&mut step.events);
        step.done = true;
        step.error = Some(conduit::i18n::text(key));
        step
    }

    fn send(&mut self, stage: Stage, method: &str, params: Value, step: &mut Step) {
        self.stage = stage;
        self.pending += 1;
        step.writes
            .push(json!({"jsonrpc":"2.0","id":self.pending,"method":method,"params":params}));
    }

    fn catalog(&mut self, result: &Value, step: &mut Step) {
        if let Some(models) = result["models"]["availableModels"].as_array() {
            self.settings.options = json!([{"id":"model","currentValue":result["models"]["currentModelId"].as_str(),
                "options":models.iter().take(512).filter_map(|m| Some(json!({"value":m["modelId"].as_str()?,"name":m["name"].as_str()?}))).collect::<Vec<_>>()}]);
            step.events
                .push(Event::AcpConfig(self.settings.options.clone()));
        }
    }

    fn prompt(&mut self, step: &mut Step) {
        self.send(
            Stage::Prompt,
            "session/prompt",
            json!({"sessionId":self.session,
            "prompt":[{"type":"text","text":self.request.prompt}]}),
            step,
        );
    }

    fn permission(&self, value: &Value, step: &mut Step) {
        let params = &value["params"];
        let approval = Approval {
            id: format!("gemini:{}", value["id"]),
            wire_id: value["id"].clone(),
            title: params["toolCall"]["title"]
                .as_str()
                .unwrap_or("Gemini tool")
                .into(),
            detail: serde_json::to_string_pretty(&tool_view(&params["toolCall"]))
                .unwrap_or_default(),
            input: json!({"sessionId":params["sessionId"],"options":params["options"],"toolCall":tool_view(&params["toolCall"])}),
            allow_once: option(params, "allow_once").is_some(),
            kind: if params["toolCall"]
                .pointer("/_meta/gemini-cli/mcp")
                .is_some()
            {
                ApprovalKind::McpTool
            } else {
                ApprovalKind::GeminiTool
            },
        };
        if self.stage != Stage::Prompt
            || self.cancelled.get()
            || params["sessionId"].as_str() != self.session.as_deref()
            || !valid_id(&value["id"])
            || params["toolCall"]["toolCallId"].as_str().is_none()
        {
            step.writes.push(self.approval_response(&approval, false));
        } else {
            step.events.push(Event::Approval(approval));
        }
    }

    fn update(&mut self, params: &Value, step: &mut Step) {
        // Loading replays native history; local transcripts already contain these events.
        if self.stage != Stage::Prompt
            || self.cancelled.get()
            || params["sessionId"].as_str() != self.session.as_deref()
        {
            return;
        }
        let update = &params["update"];
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                self.thoughts.finish(&mut step.events);
                if let Some(text) = self.presentation.content(&update["content"], step) {
                    step.events.push(Event::Delta {
                        key: format!("gemini-reply-{}", self.reply),
                        text,
                    });
                }
            }
            Some("agent_thought_chunk") if !self.request.text_only => {
                if let Some(text) = update["content"]["text"].as_str()
                    && self.thoughts.push("gemini", text, &mut step.events)
                {
                    self.reply += 1;
                }
            }
            Some("tool_call" | "tool_call_update") if !self.request.text_only => {
                let Some(id) = update["toolCallId"].as_str() else {
                    return;
                };
                if !self.tools.contains_key(id) {
                    self.thoughts.finish(&mut step.events);
                    self.reply += 1;
                }
                let tool = self.tools.entry(id.into()).or_insert_with(|| json!({}));
                for (key, value) in tool_view(update).as_object().into_iter().flatten() {
                    tool[key] = value.clone();
                }
                step.events.push(Event::Tool {
                    key:id.into(), title:tool["title"].as_str().unwrap_or("Gemini tool").into(),
                    detail:serde_json::to_string_pretty(&json!({"acpKind":tool["kind"],"content":tool["content"],"output":tool["rawOutput"],"locations":tool["locations"]})).unwrap_or_default(),
                    status:crate::message_status::ToolStatus::from_wire(tool["status"].as_str().unwrap_or("running")),
                });
                if let Some(input) = tool.get("rawInput") {
                    step.events.push(Event::ToolInput {
                        key: id.into(),
                        input: serde_json::to_string_pretty(input).unwrap_or_default(),
                    });
                }
            }
            Some("usage_update") => {
                self.context_window = update["size"].as_u64().unwrap_or(0);
                self.context_tokens = update["used"].as_u64();
            }
            Some("plan") => step.events.push(Event::Plan {
                steps: update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| {
                        Some((p["content"].as_str()?.into(), p["status"].as_str()?.into()))
                    })
                    .collect(),
            }),
            Some("user_message_chunk" | "current_mode_update" | "available_commands_update") => {}
            Some(kind) => self.presentation.ignored(&format!("Gemini update/{kind}")),
            None => self.presentation.ignored("Gemini update/malformed"),
        }
    }

    fn transition(&mut self, result: &Value, step: &mut Step) -> Result<()> {
        match self.stage {
            Stage::Initialize => {
                ensure!(
                    result["protocolVersion"] == 1 && result["agentInfo"]["name"] == "gemini-cli",
                    conduit::i18n::text("gemini.invalid_agent")
                );
                let mut params = json!({"cwd":crate::child::normalized_dir(&self.request.cwd).to_string_lossy(),"mcpServers":[]});
                let method = if let Some(id) = &self.request.native_id {
                    ensure!(
                        result["agentCapabilities"]["loadSession"] == true,
                        conduit::i18n::text("gemini.resume_unavailable")
                    );
                    params["sessionId"] = json!(id);
                    "session/load"
                } else {
                    "session/new"
                };
                // Existing CLI credentials are reused; authenticate can mutate persistent login settings.
                self.send(Stage::Session, method, params, step);
            }
            Stage::Session => {
                let id = result["sessionId"]
                    .as_str()
                    .or(self.request.native_id.as_deref())
                    .filter(|s| !s.is_empty() && s.len() <= 512)
                    .ok_or_else(|| {
                        anyhow::anyhow!(conduit::i18n::text("gemini.invalid_session"))
                    })?;
                ensure!(
                    self.request
                        .native_id
                        .as_deref()
                        .is_none_or(|old| old == id),
                    conduit::i18n::text("gemini.resume_mismatch")
                );
                self.session = Some(id.into());
                step.events.push(Event::NativeId(id.into()));
                let mode = self.mode();
                ensure!(
                    result["modes"]["availableModes"]
                        .as_array()
                        .is_some_and(|m| m.iter().any(|m| m["id"] == mode)),
                    conduit::i18n::text("gemini.mode_unavailable")
                );
                self.send(
                    Stage::Mode,
                    "session/set_mode",
                    json!({"sessionId":self.session,"modeId":mode}),
                    step,
                );
            }
            Stage::Mode => {
                if let Some(model) = self
                    .request
                    .model
                    .clone()
                    .or_else(|| self.request.route.default_model.clone())
                {
                    ensure!(
                        self.settings.models().iter().any(|m| m.id == model),
                        conduit::i18n::text("gemini.model_unavailable")
                    );
                    self.send(
                        Stage::Model,
                        "session/set_model",
                        json!({"sessionId":self.session,"modelId":model}),
                        step,
                    );
                } else {
                    self.prompt(step);
                }
            }
            Stage::Model => self.prompt(step),
            Stage::Prompt => {
                self.thoughts.finish(&mut step.events);
                if let Some(usage) = result.get("usage") {
                    step.events.push(Event::Usage(Usage {
                        input_tokens_known: usage["inputTokens"].as_u64().is_some(),
                        // Gemini omits cachedReadTokens when the measured count is zero.
                        cached_input_tokens_known: usage["inputTokens"].as_u64().is_some(),
                        output_tokens_known: usage["outputTokens"].as_u64().is_some(),
                        input_tokens: usage["inputTokens"].as_u64().unwrap_or(0),
                        cached_input_tokens: usage["cachedReadTokens"].as_u64().unwrap_or(0),
                        output_tokens: usage["outputTokens"].as_u64().unwrap_or(0),
                        total_tokens: usage["totalTokens"].as_u64().unwrap_or(0),
                        turn_output_tokens: usage["outputTokens"].as_u64(),
                        context_window: self.context_window,
                        context_tokens: self.context_tokens,
                        ..Default::default()
                    }));
                }
                ensure!(
                    result["stopReason"] == "end_turn"
                        || (self.cancelled.get() && result["stopReason"] == "cancelled"),
                    conduit::i18n::text("gemini.incomplete_turn")
                );
                self.stage = Stage::Done;
                step.done = true;
            }
            Stage::Done => {}
        }
        Ok(())
    }
}

impl CliAdapter for GeminiAdapter {
    fn launch(&mut self) -> Result<LaunchSpec> {
        ensure!(
            matches!(self.request.intent, crate::commands::RequestIntent::Message),
            conduit::i18n::text("commands.unsupported")
        );
        ensure!(
            !self.request.prompt.trim_start().starts_with(['/', '$']),
            conduit::i18n::text("gemini.commands_native")
        );
        ensure!(
            self.request.route.config.is_empty(),
            conduit::i18n::text("gemini.native_route")
        );
        ensure!(
            self.request.effort == ReasoningEffort::Default
                && self.request.route.acp_variant.is_none(),
            conduit::i18n::text("gemini.effort_unavailable")
        );
        if let Some(id) = &self.request.native_id {
            ensure!(
                !id.is_empty()
                    && id.len() <= 512
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
                conduit::i18n::text("gemini.invalid_session")
            );
        }
        let config = config::RunConfig::prepare(&self.request)?;
        let mut args = vec!["--acp".into(), "--approval-mode".into(), self.mode().into()];
        args.extend(config.args.clone());
        let environment = config.environment.clone();
        self.config = Some(config);
        Ok(LaunchSpec {
            args,
            environment,
            completion: Completion::ProtocolEvent,
            input: InitialInput::Json(
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":1,"clientInfo":{"name":"conduit","version":env!("CARGO_PKG_VERSION")},
                "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}}),
            ),
        })
    }

    fn accept(&mut self, value: Value) -> Step {
        let mut step = Step::new();
        if self.stage == Stage::Done {
            return step;
        }
        match value["method"].as_str() {
            Some("session/update") => {
                self.update(&value["params"], &mut step);
                return step;
            }
            Some("session/request_permission") if value.get("id").is_some() => {
                self.permission(&value, &mut step);
                return step;
            }
            Some(method) => {
                if value.get("id").is_some() {
                    self.presentation
                        .unsupported(&format!("Gemini request/{method}"), &mut step);
                    step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Unsupported client capability"}}));
                }
                return step;
            }
            None => {}
        }
        if value["id"] != self.pending {
            return step;
        }
        if self.cancelled.get() && self.stage != Stage::Prompt {
            self.stage = Stage::Done;
            step.done = true;
            return step;
        }
        if value.get("error").is_some_and(|v| !v.is_null()) {
            return self.fail(
                if matches!(value["error"]["code"].as_i64(), Some(-32000 | -32001)) {
                    "gemini.authentication"
                } else if self.stage == Stage::Session && self.request.native_id.is_some() {
                    "gemini.resume_failed"
                } else {
                    "gemini.request_failed"
                },
            );
        }
        let Some(result) = value.get("result").filter(|v| v.is_object()) else {
            return self.fail("gemini.invalid_response");
        };
        self.catalog(result, &mut step);
        if let Err(error) = self.transition(result, &mut step) {
            self.stage = Stage::Done;
            self.thoughts.finish(&mut step.events);
            step.done = true;
            step.error = Some(error.to_string());
            step.writes.clear();
        }
        step
    }

    fn startup_pending(&self) -> bool {
        !matches!(self.stage, Stage::Prompt | Stage::Done)
    }
    fn cancel_message(&self) -> Option<Value> {
        self.cancelled.set(true);
        self.session
            .as_ref()
            .map(|id| json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":id}}))
    }
    fn automatic_approval(&self, approval: &Approval) -> Option<bool> {
        if self.restricted() || self.cancelled.get() {
            Some(false)
        } else if self.request.permission == PermissionLevel::FullAccess {
            Some(option(&approval.input, "allow_once").is_some())
        } else {
            None
        }
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        let allow = allow
            && !self.restricted()
            && !self.cancelled.get()
            && self.stage == Stage::Prompt
            && approval.input["sessionId"].as_str() == self.session.as_deref();
        // Never turn a single approval into persistent permission or invent an option ID.
        let outcome = option(
            &approval.input,
            if allow { "allow_once" } else { "reject_once" },
        )
        .map(|id| json!({"outcome":"selected","optionId":id}))
        .unwrap_or_else(|| json!({"outcome":"cancelled"}));
        json!({"jsonrpc":"2.0","id":approval.wire_id,"result":{"outcome":outcome}})
    }
    fn user_input_response(
        &self,
        _: &Approval,
        _: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        None
    }
    fn enable_full_access(&mut self) {
        // Restricted runs were launched with a restrictive native policy; never elevate them in place.
        if !self.restricted() {
            self.request.permission = PermissionLevel::FullAccess;
        }
    }
}

fn option<'a>(params: &'a Value, kind: &str) -> Option<&'a str> {
    params["options"]
        .as_array()?
        .iter()
        .find(|o| o["kind"] == kind)?["optionId"]
        .as_str()
        .filter(|id| !id.is_empty())
}
fn valid_id(id: &Value) -> bool {
    id.as_str().is_some_and(|s| !s.is_empty()) || id.is_i64() || id.is_u64()
}

// Do not copy provider metadata, embedded binary media or arbitrary RPC fields into history.
fn tool_view(value: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in ["toolCallId", "title", "kind", "status", "rawInput"] {
        if let Some(v) = value.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(text) = value["rawOutput"].as_str() {
        out.insert("rawOutput".into(), json!(text));
    }
    if let Some(locations) = value["locations"].as_array() {
        out.insert(
            "locations".into(),
            json!(
                locations
                    .iter()
                    .filter_map(|l| Some(
                        json!({"path":l["path"].as_str()?,"line":l["line"].as_u64()})
                    ))
                    .collect::<Vec<_>>()
            ),
        );
    }
    if let Some(content) = value["content"].as_array() {
        out.insert("content".into(), json!(content.iter().filter_map(|c| match c["type"].as_str() {
            Some("diff") => Some(json!({"type":"diff","path":c["path"].as_str(),"oldText":c["oldText"].as_str(),"newText":c["newText"].as_str()})),
            Some("content") if c["content"]["type"] == "text" => Some(json!({"type":"content","content":{"type":"text","text":c["content"]["text"].as_str()}})),
            _ => None,
        }).collect::<Vec<_>>()));
    }
    Value::Object(out)
}
