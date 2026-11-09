//! Qoder ACP transport. Process ownership and approval tickets remain in ChatRuntime.
use super::*;
use crate::{
    domain::Usage,
    protocol::{AcpSettings, ApprovalKind, Event},
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::collections::HashMap;
mod questions;
#[cfg(test)]
mod tests;

pub fn qoder(request: AdapterRequest) -> Box<dyn CliAdapter> {
    Box::new(QoderAdapter::new(request))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Initialize,
    Session,
    Mode,
    Model,
    Effort,
    Prompt,
    Done,
}

struct QoderAdapter {
    request: AdapterRequest,
    stage: Stage,
    pending_id: u64,
    session_id: Option<String>,
    settings: AcpSettings,
    tools: HashMap<String, Value>,
    reply_index: u64,
    context_window: u64,
    context_tokens: Option<u64>,
    cancellation_requested: std::cell::Cell<bool>,
}

impl QoderAdapter {
    fn new(request: AdapterRequest) -> Self {
        Self {
            request,
            stage: Stage::Initialize,
            pending_id: 1,
            session_id: None,
            settings: Default::default(),
            tools: HashMap::new(),
            reply_index: 0,
            context_window: 0,
            context_tokens: None,
            cancellation_requested: std::cell::Cell::new(false),
        }
    }

    fn fail(&mut self, key: &str) -> Step {
        self.stage = Stage::Done;
        Step {
            done: true,
            error: Some(conduit::i18n::text(key)),
            ..Step::new()
        }
    }

    fn send(&mut self, stage: Stage, method: &str, params: Value, step: &mut Step) {
        self.stage = stage;
        self.pending_id += 1;
        step.writes
            .push(json!({"jsonrpc":"2.0","id":self.pending_id,"method":method,"params":params}));
    }

    fn catalog(&mut self, value: &Value, step: &mut Step) {
        if let Some(options) = value["configOptions"].as_array() {
            // Persist only presentation fields, never arbitrary provider configuration.
            let options: Vec<_> = options
                .iter()
                .filter(|o| {
                    matches!(
                        o["id"].as_str(),
                        Some("model" | "mode" | "reasoning_effort")
                    )
                })
                .map(clean_option)
                .collect();
            self.settings.options = json!(options);
            step.events
                .push(Event::AcpConfig(self.settings.options.clone()));
        } else if let Some(models) = value["models"]["availableModels"].as_array() {
            self.settings.options = json!([{"id":"model", "currentValue":value["models"]["currentModelId"],
                "options": models.iter().filter_map(|m| Some(json!({"value":m["modelId"].as_str()?,"name":m["name"].as_str()?}))).collect::<Vec<_>>()}]);
            step.events
                .push(Event::AcpConfig(self.settings.options.clone()));
        }
    }

    fn configure_model(&mut self, step: &mut Step) -> Result<()> {
        if let Some(model) = self
            .request
            .model
            .clone()
            .or_else(|| self.request.route.default_model.clone())
        {
            ensure!(
                self.settings.models().iter().any(|m| m.id == model),
                conduit::i18n::text("qoder.model_unavailable")
            );
            self.send(
                Stage::Model,
                "session/set_model",
                json!({"sessionId":self.session_id,"modelId":model}),
                step,
            );
        } else {
            self.configure_effort(step)?;
        }
        Ok(())
    }

    fn configure_effort(&mut self, step: &mut Step) -> Result<()> {
        let selected = self
            .request
            .route
            .acp_variant
            .clone()
            .or_else(|| self.request.effort.wire().map(str::to_owned));
        let effort = selected.or_else(|| self.settings.effective_variant().map(str::to_owned));
        if let Some(effort) = effort {
            ensure!(
                self.settings.variants().iter().any(|(id, _)| id == &effort),
                conduit::i18n::text("qoder.effort_unavailable")
            );
            self.send(
                Stage::Effort,
                "session/set_config_option",
                json!({"sessionId":self.session_id,"configId":"reasoning_effort","value":effort}),
                step,
            );
        } else {
            self.prompt(step);
        }
        Ok(())
    }

    fn prompt(&mut self, step: &mut Step) {
        // Two content blocks keep user text out of Qoder's single-block slash-command
        // dispatcher (including /allow and permission-mode changes).
        self.send(
            Stage::Prompt,
            "session/prompt",
            json!({"sessionId":self.session_id,
            "prompt":[{"type":"text","text":self.request.prompt},{"type":"text","text":"\n"}]}),
            step,
        );
    }

    fn permission(&self, value: &Value, step: &mut Step) {
        let params = &value["params"];
        let approval = Approval {
            id: format!("qoder:{}", value["id"]),
            wire_id: value["id"].clone(),
            title: params["toolCall"]["title"]
                .as_str()
                .unwrap_or("Qoder tool")
                .into(),
            detail: serde_json::to_string_pretty(&params["toolCall"]).unwrap_or_default(),
            input: params.clone(),
            allow_once: option(params, "allow_once").is_some(),
            kind: ApprovalKind::QoderTool,
        };
        if self.stage != Stage::Prompt
            || params["sessionId"].as_str() != self.session_id.as_deref()
            || !valid_id(&value["id"])
            || params["toolCall"]["toolCallId"].as_str().is_none()
        {
            step.writes.push(self.approval_response(&approval, false));
        } else {
            step.events.push(Event::Approval(approval));
        }
    }

    fn update(&mut self, params: &Value, step: &mut Step) {
        if params["sessionId"].as_str() != self.session_id.as_deref() {
            return;
        }
        let update = &params["update"];
        if update["sessionUpdate"] == "config_option_update" {
            self.catalog(update, step);
            return;
        }
        // session/load may replay the entire native transcript; it is already saved locally.
        if self.stage != Stage::Prompt {
            return;
        }
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    step.events.push(Event::Delta {
                        key: format!("qoder-reply-{}", self.reply_index),
                        text: text.into(),
                    });
                }
            }
            Some("tool_call" | "tool_call_update") => {
                let Some(id) = update["toolCallId"].as_str() else {
                    return;
                };
                if !self.tools.contains_key(id) {
                    self.reply_index += 1;
                }
                let merged = self.tools.entry(id.into()).or_insert_with(|| json!({}));
                for (k, v) in update.as_object().into_iter().flatten() {
                    merged[k] = v.clone();
                }
                let status = match merged["status"].as_str() {
                    Some("completed") => conduit::i18n::text_static("qoder.tool_completed"),
                    Some("failed") => conduit::i18n::text_static("tools.failed"),
                    _ => conduit::i18n::text_static("qoder.tool_running"),
                };
                step.events.push(Event::Tool { key:id.into(), title:merged["title"].as_str().unwrap_or("Qoder tool").into(),
                    detail:serde_json::to_string_pretty(&json!({"acpKind":merged["kind"],"content":merged["content"],"output":merged["rawOutput"],"locations":merged["locations"]})).unwrap_or_default(), status:status.into() });
                if let Some(input) = merged.get("rawInput") {
                    step.events.push(Event::ToolInput {
                        key: id.into(),
                        input: serde_json::to_string_pretty(input).unwrap_or_default(),
                    });
                }
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
            Some("usage_update") => {
                self.context_window = update["size"].as_u64().unwrap_or(0);
                self.context_tokens = update["used"].as_u64();
            }
            // Do not render agent_thought_chunk as a public reasoning summary.
            _ => {}
        }
    }
}

impl CliAdapter for QoderAdapter {
    fn launch(&mut self) -> Result<LaunchSpec> {
        ensure!(
            self.request.route.config.is_empty(),
            conduit::i18n::text("qoder.native_route")
        );
        let restricted =
            self.request.text_only || self.request.permission == PermissionLevel::ReadOnly;
        let mode =
            if !self.request.text_only && self.request.permission == PermissionLevel::FullAccess {
                "bypass_permissions"
            } else {
                "default"
            };
        let settings = json!({"disableAllHooks":true, "permissions":{"ask":if restricted { vec![] } else { vec!["*"] }}});
        let mut args = vec![
            "--acp".into(),
            "--cwd".into(),
            self.request.cwd.to_string_lossy().into_owned(),
            "--permission-mode".into(),
            mode.into(),
            "--settings".into(),
            settings.to_string(),
        ];
        if restricted {
            args.extend([
                "--strict-mcp-config".into(),
                "--mcp-config".into(),
                "{\"mcpServers\":{}}".into(),
                "--tools".into(),
                if self.request.text_only {
                    "".into()
                } else {
                    "Read,Grep,Glob".into()
                },
            ]);
            if self.request.text_only {
                args.extend(["--disallowed-tools".into(), "*".into()]);
            }
        }
        Ok(LaunchSpec {
            args,
            environment: self.request.route.env.clone(),
            completion: Completion::ProtocolEvent,
            input: InitialInput::Json(
                json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
                "protocolVersion":1,
                "clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false,
                    "elicitation":{"form":{}}}}}),
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
            Some("elicitation/create") if value.get("id").is_some() => {
                if self.stage != Stage::Prompt
                    || self.request.text_only
                    || value["params"]["sessionId"].as_str() != self.session_id.as_deref()
                {
                    step.writes.push(
                        json!({"jsonrpc":"2.0","id":value["id"],"result":{"action":"cancel"}}),
                    );
                } else {
                    match questions::approval(&value) {
                        Ok(approval) => step.events.push(Event::Approval(approval)),
                        Err(_) => step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32602,"message":"Unsupported elicitation form"}})),
                    }
                }
                return step;
            }
            Some(_) => {
                if value.get("id").is_some() {
                    step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Unsupported client capability"}}));
                }
                return step;
            }
            None => {}
        }
        if value["id"] != self.pending_id {
            return step;
        }
        if value.get("error").is_some_and(|v| !v.is_null()) {
            // Do not expose raw provider errors (which can contain credentials).
            return self.fail(if value["error"]["code"] == -32000 {
                "qoder.authentication"
            } else {
                "qoder.request_failed"
            });
        }
        let Some(result) = value.get("result").filter(|v| v.is_object()) else {
            return self.fail("qoder.invalid_response");
        };
        self.catalog(result, &mut step);
        let transition: Result<()> = (|| {
            match self.stage {
                Stage::Initialize => {
                    ensure!(
                        result["protocolVersion"] == 1
                            && result["agentInfo"]["name"]
                                .as_str()
                                .is_some_and(|s| s.starts_with("qoder")),
                        conduit::i18n::text("qoder.invalid_agent")
                    );
                    // Qoder restores existing login/environment credentials when opening
                    // the session. Do not call authenticate: qodercli-login can open a
                    // browser. An auth-required response directs the user to the CLI.
                    let mut params =
                        json!({"cwd":self.request.cwd.to_string_lossy(),"mcpServers":[]});
                    let method = if let Some(id) = &self.request.native_id {
                        params["sessionId"] = json!(id);
                        if result["agentCapabilities"]["sessionCapabilities"]
                            .get("resume")
                            .is_some()
                        {
                            "session/resume"
                        } else {
                            ensure!(
                                result["agentCapabilities"]["loadSession"] == true,
                                conduit::i18n::text("qoder.resume_unsupported")
                            );
                            "session/load"
                        }
                    } else {
                        "session/new"
                    };
                    self.send(Stage::Session, method, params, &mut step);
                }
                Stage::Session => {
                    let id = result["sessionId"]
                        .as_str()
                        .or(self.request.native_id.as_deref())
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| {
                            anyhow::anyhow!(conduit::i18n::text("qoder.invalid_session"))
                        })?;
                    ensure!(
                        self.request
                            .native_id
                            .as_deref()
                            .is_none_or(|expected| expected == id),
                        conduit::i18n::text("qoder.resume_mismatch")
                    );
                    self.session_id = Some(id.into());
                    step.events.push(Event::NativeId(id.into()));
                    let mode = if self.request.permission == PermissionLevel::FullAccess
                        && !self.request.text_only
                    {
                        "yolo"
                    } else {
                        "default"
                    };
                    if let Some(modes) = result["modes"]["availableModes"].as_array() {
                        ensure!(
                            modes.iter().any(|m| m["id"] == mode),
                            conduit::i18n::text("qoder.mode_unavailable")
                        );
                    }
                    self.send(
                        Stage::Mode,
                        "session/set_mode",
                        json!({"sessionId":self.session_id,"modeId":mode}),
                        &mut step,
                    );
                }
                Stage::Mode => self.configure_model(&mut step)?,
                Stage::Model => self.configure_effort(&mut step)?,
                Stage::Effort => self.prompt(&mut step),
                Stage::Prompt => {
                    ensure!(
                        matches!(
                            result["stopReason"].as_str(),
                            Some(
                                "end_turn"
                                    | "cancelled"
                                    | "max_tokens"
                                    | "max_turn_requests"
                                    | "refusal"
                            )
                        ),
                        conduit::i18n::text("qoder.invalid_response")
                    );
                    if let Some(usage) = result.get("usage") {
                        step.events.push(Event::Usage(Usage {
                            input_tokens: usage["inputTokens"].as_u64().unwrap_or(0),
                            output_tokens: usage["outputTokens"].as_u64().unwrap_or(0),
                            cached_input_tokens: usage["cachedReadTokens"].as_u64().unwrap_or(0),
                            total_tokens: usage["totalTokens"].as_u64().unwrap_or(0),
                            turn_output_tokens: usage["outputTokens"].as_u64(),
                            context_window: self.context_window,
                            context_tokens: self.context_tokens,
                            ..Default::default()
                        }));
                    }
                    if result["stopReason"] != "end_turn"
                        && !(result["stopReason"] == "cancelled"
                            && self.cancellation_requested.get())
                    {
                        step.error = Some(conduit::i18n::text("qoder.incomplete_turn"));
                    }
                    step.done = true;
                    self.stage = Stage::Done;
                }
                Stage::Done => {}
            }
            Ok(())
        })();
        if let Err(error) = transition {
            self.stage = Stage::Done;
            step.done = true;
            step.error = Some(error.to_string());
            step.writes.clear();
        }
        step
    }

    fn startup_pending(&self) -> bool {
        matches!(
            self.stage,
            Stage::Initialize | Stage::Session | Stage::Mode | Stage::Model | Stage::Effort
        )
    }
    fn cancel_message(&self) -> Option<Value> {
        self.session_id.as_ref().map(|id| {
            self.cancellation_requested.set(true);
            json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":id}})
        })
    }
    fn automatic_approval(&self, approval: &Approval) -> Option<bool> {
        if self.request.text_only {
            return Some(false);
        }
        if approval.is_user_input() {
            return None;
        }
        match self.request.permission {
            PermissionLevel::ReadOnly => Some(false),
            PermissionLevel::FullAccess => Some(
                option(&approval.input, "allow_once").is_some()
                    || option(&approval.input, "allow_always").is_some(),
            ),
            PermissionLevel::WorkspaceWrite => None,
        }
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        if approval.is_user_input() {
            return json!({"jsonrpc":"2.0","id":approval.wire_id,"result":{"action":"cancel"}});
        }
        let allow = allow
            && !self.request.text_only
            && self.request.permission != PermissionLevel::ReadOnly;
        let choice = option(
            &approval.input,
            if allow { "allow_once" } else { "reject_once" },
        )
        .or_else(|| {
            if allow && self.request.permission == PermissionLevel::FullAccess {
                option(&approval.input, "allow_always")
            } else {
                None
            }
        });
        let outcome = choice
            .map(|id| json!({"outcome":"selected","optionId":id}))
            .unwrap_or_else(|| json!({"outcome":"cancelled"}));
        json!({"jsonrpc":"2.0","id":approval.wire_id,"result":{"outcome":outcome}})
    }
    fn user_input_response(
        &self,
        approval: &Approval,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        if self.request.text_only {
            return None;
        }
        questions::response(approval, answers)
    }
    fn enable_full_access(&mut self) {
        if !self.request.text_only {
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
}
fn valid_id(id: &Value) -> bool {
    id.as_str().is_some_and(|s| !s.is_empty()) || id.is_i64() || id.is_u64()
}
fn clean_option(value: &Value) -> Value {
    let mut cleaned = serde_json::Map::new();
    for key in [
        "id",
        "name",
        "type",
        "category",
        "currentValue",
        "value",
        "description",
    ] {
        if let Some(v) = value.get(key).filter(|v| v.is_string()) {
            cleaned.insert(key.into(), v.clone());
        }
    }
    if let Some(options) = value["options"].as_array() {
        cleaned.insert(
            "options".into(),
            json!(options.iter().map(clean_option).collect::<Vec<_>>()),
        );
    }
    Value::Object(cleaned)
}
