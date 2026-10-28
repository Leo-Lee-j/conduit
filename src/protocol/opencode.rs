//! OpenCode 1.19 ACP adapter. Transport and process ownership stay in ChatRuntime.
use super::*;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenCodeSettings {
    /// Engine-specific variant. Do not translate xhigh to max implicitly.
    #[serde(default)]
    pub variant: Option<String>,
    /// Last effective session catalog, never credentials or raw provider config.
    #[serde(default)]
    pub options: Value,
}

impl OpenCodeSettings {
    pub fn models(&self) -> Vec<crate::models::ModelOption> {
        let mut models = vec![];
        if let Some(options) = self.option("model")["options"].as_array() {
            for option in options {
                if let Some(group) = option.get("options").and_then(Value::as_array) {
                    for item in group {
                        push_model(&mut models, item);
                    }
                } else {
                    push_model(&mut models, option);
                }
            }
        }
        models
    }

    pub fn option(&self, id: &str) -> &Value {
        self.options
            .as_array()
            .and_then(|items| items.iter().find(|o| o["id"] == id))
            .unwrap_or(&Value::Null)
    }

    pub fn variants(&self) -> Vec<(String, String)> {
        self.option("effort")["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| {
                Some((
                    o["value"].as_str()?.to_owned(),
                    o["name"].as_str()?.to_owned(),
                ))
            })
            .collect()
    }
}

fn push_model(models: &mut Vec<crate::models::ModelOption>, option: &Value) {
    if let Some(id) = option["value"].as_str() {
        crate::models::push_model(
            models,
            id,
            option["name"].as_str().unwrap_or(id),
            "OpenCode",
        );
    }
}

#[derive(Default)]
pub(super) struct State {
    pub session_id: Option<String>,
    agent: String,
    stage: Stage,
    next_id: u64,
    pending_id: u64,
    config: VecDeque<(String, String)>,
    options: OpenCodeSettings,
    tools: HashMap<String, Value>,
    context_window: u64,
    helper_directory: Option<std::path::PathBuf>,
}

impl State {
    pub fn startup_pending(&self) -> bool {
        matches!(
            self.stage,
            Stage::Initialize | Stage::Session | Stage::Configure
        )
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // Protocol is dropped after the runtime's process cleanup barrier.
        if let Some(path) = &self.helper_directory {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[derive(Default, PartialEq)]
enum Stage {
    #[default]
    Initialize,
    Session,
    Configure,
    Prompt,
    Done,
}

impl Protocol {
    pub(super) fn prepare_opencode(&mut self) -> Result<()> {
        use windows_sys::Win32::Security::Cryptography::{
            BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
        };
        let mut random = [0_u8; 32];
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                random.as_mut_ptr(),
                random.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            bail!("无法生成 OpenCode 本地服务凭证");
        }
        let secret: String = random.iter().map(|b| format!("{b:02x}")).collect();
        self.acp.agent = format!("conduit-{}", &secret[..16]);
        let inherited = self
            .route
            .env
            .get("OPENCODE_CONFIG_CONTENT")
            .cloned()
            .or_else(|| std::env::var("OPENCODE_CONFIG_CONTENT").ok())
            .unwrap_or_else(|| "{}".into());
        let mut config: Value =
            serde_json::from_str(&inherited).context("OPENCODE_CONFIG_CONTENT 不是有效 JSON")?;
        let object = config
            .as_object_mut()
            .context("OPENCODE_CONFIG_CONTENT 必须是对象")?;
        let agents = object
            .entry("agent")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .context("OpenCode agent 配置必须是对象")?;
        // This new agent has no user agent-specific rules. Its rules are appended
        // after inherited global rules, including inherited concrete allow rules.
        let permission = if self.text_only {
            json!({"*":"deny","external_directory":"deny"})
        } else if self.permission == PermissionLevel::ReadOnly {
            json!({"*":"deny","read":"allow","glob":"allow","grep":"allow","list":"allow","external_directory":"ask"})
        } else {
            let action = if self.permission == PermissionLevel::FullAccess {
                "allow"
            } else {
                "ask"
            };
            json!({"*":action,"read":"allow","glob":"allow","grep":"allow","list":"allow","external_directory":action,"question":"deny","plan_enter":"deny","plan_exit":"deny","task":"deny"})
        };
        agents.insert(self.acp.agent.clone(), json!({"mode":"primary","description":"Conduit managed permissions","permission":permission}));
        self.route
            .env
            .insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
        self.route
            .env
            .insert("OPENCODE_SERVER_PASSWORD".into(), secret.clone());
        self.route
            .env
            .insert("OPENCODE_SERVER_USERNAME".into(), "conduit".into());
        self.route
            .env
            .insert("OPENCODE_DISABLE_AUTOUPDATE".into(), "1".into());
        if self.text_only {
            let directory =
                std::env::temp_dir().join(format!("conduit-opencode-helper-{}", &secret[..24]));
            std::fs::create_dir(&directory).context("无法创建 OpenCode 辅助任务目录")?;
            self.route.env.insert(
                "OPENCODE_DB".into(),
                directory
                    .join("helper.sqlite")
                    .to_string_lossy()
                    .into_owned(),
            );
            self.route
                .env
                .insert("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "1".into());
            self.acp.helper_directory = Some(directory);
        }
        Ok(())
    }

    pub(super) fn opencode(&mut self, value: Value) -> Step {
        let mut step = Step::new();
        if value["method"] == "session/request_permission" && value.get("id").is_some() {
            self.acp_permission(value, &mut step);
            return step;
        }
        if let Some(method) = value["method"].as_str() {
            if method == "session/update" {
                self.acp_update(&value["params"], &mut step);
            } else if value.get("id").is_some() {
                step.writes.push(json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Conduit does not implement this client capability"}}));
            }
            return step;
        }
        let expected = if self.acp.stage == Stage::Initialize {
            1
        } else {
            self.acp.pending_id
        };
        if value["id"].as_u64() != Some(expected) {
            return step;
        }
        if let Some(error) = value.get("error") {
            self.acp.stage = Stage::Done;
            step.done = true;
            step.error = Some(format!(
                "OpenCode：{}",
                error["message"].as_str().unwrap_or("ACP 请求失败")
            ));
            return step;
        }
        let Some(result) = value.get("result") else {
            return step;
        };
        match self.acp.stage {
            Stage::Initialize => {
                if result["protocolVersion"] != 1 || result["agentInfo"]["name"] != "OpenCode" {
                    step.done = true;
                    step.error = Some("OpenCode ACP 协议或程序身份不兼容".into());
                    return step;
                }
                self.acp.stage = Stage::Session;
                let params = json!({"cwd":self.cwd,"mcpServers":[]});
                if let Some(id) = &self.native_id {
                    let mut params = params;
                    params["sessionId"] = json!(id);
                    let resume = result
                        .pointer("/agentCapabilities/sessionCapabilities/resume")
                        .is_some();
                    if !resume && result["agentCapabilities"]["loadSession"] != true {
                        step.done = true;
                        step.error = Some("OpenCode 不支持原生会话恢复；请更新 CLI".into());
                        return step;
                    }
                    self.acp.session_id = Some(id.clone());
                    self.acp_request(
                        if resume {
                            "session/resume"
                        } else {
                            "session/load"
                        },
                        params,
                        &mut step,
                    );
                } else {
                    self.acp_request("session/new", params, &mut step);
                }
            }
            Stage::Session => {
                if self.native_id.is_none() {
                    self.acp.session_id = result["sessionId"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                }
                let Some(id) = &self.acp.session_id else {
                    step.done = true;
                    step.error = Some("OpenCode 未返回原生会话 ID".into());
                    return step;
                };
                step.events.push(Event::NativeId(id.clone()));
                self.acp_catalog(result, &mut step);
                self.acp
                    .config
                    .push_back(("mode".into(), self.acp.agent.clone()));
                if let Some(model) = &self.model {
                    self.acp.config.push_back(("model".into(), model.clone()));
                }
                // Setting model changes the effort catalog. Validate the desired
                // effort only when preceding configuration replies have arrived.
                let variant = self
                    .route
                    .opencode_variant
                    .clone()
                    .or_else(|| self.reasoning_effort.wire().map(str::to_owned));
                self.acp
                    .config
                    .push_back(("effort".into(), variant.unwrap_or_else(|| "default".into())));
                self.acp.stage = Stage::Configure;
                self.acp_next(&mut step);
            }
            Stage::Configure => {
                self.acp_catalog(result, &mut step);
                self.acp_next(&mut step);
            }
            Stage::Prompt => {
                if let Some(usage) = result.get("usage") {
                    self.acp_usage(usage, &mut step);
                }
                self.acp.stage = Stage::Done;
                step.done = true;
                match result["stopReason"].as_str() {
                    Some("end_turn" | "cancelled") => {}
                    Some("max_tokens") => step.error = Some("OpenCode 达到模型输出上限".into()),
                    Some("refusal") => step.error = Some("OpenCode 模型拒绝了本次请求".into()),
                    other => step.error = Some(format!("OpenCode 返回未支持的结束原因：{other:?}")),
                }
            }
            Stage::Done => {}
        }
        step
    }

    fn acp_request(&mut self, method: &str, params: Value, step: &mut Step) {
        self.acp.next_id = self.acp.next_id.max(1) + 1;
        self.acp.pending_id = self.acp.next_id;
        step.writes.push(
            json!({"jsonrpc":"2.0","id":self.acp.pending_id,"method":method,"params":params}),
        );
    }

    fn acp_catalog(&mut self, result: &Value, step: &mut Step) {
        if let Some(options) = result.get("configOptions").filter(|v| v.is_array()) {
            self.acp.options.options = options.clone();
            // Expose model/effort only. The private permission agent is not a
            // user-selectable permission mode and should never enter UI state.
            let options: Vec<Value> = options
                .as_array()
                .unwrap()
                .iter()
                .filter(|o| matches!(o["id"].as_str(), Some("model" | "effort")))
                .cloned()
                .collect();
            step.events.push(Event::OpenCodeConfig(json!(options)));
        }
    }

    fn acp_next(&mut self, step: &mut Step) {
        while let Some((key, selection)) = self.acp.config.pop_front() {
            let option = self.acp.options.option(&key);
            if key == "effort" && selection == "default" && option.is_null() {
                continue;
            }
            let valid = if key == "model" {
                self.acp.options.models().iter().any(|m| m.id == selection)
            } else {
                option["options"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["value"] == selection))
            };
            if !valid {
                step.done = true;
                step.error = Some(format!(
                    "OpenCode 不支持 {key}={selection}；请刷新模型配置后重新选择"
                ));
                return;
            }
            self.acp_request(
                "session/set_config_option",
                json!({"sessionId":self.acp.session_id,"configId":key,"value":selection}),
                step,
            );
            return;
        }
        self.acp.stage = Stage::Prompt;
        self.acp_request(
            "session/prompt",
            json!({"sessionId":self.acp.session_id,"prompt":[{"type":"text","text":self.prompt}]}),
            step,
        );
    }

    fn acp_permission(&mut self, value: Value, step: &mut Step) {
        let params = &value["params"];
        let options = params["options"].as_array();
        let allow_once =
            options.is_some_and(|items| items.iter().any(|o| o["kind"] == "allow_once"));
        let approval = Approval {
            id: format!("acp:{}", id_text(&value["id"])),
            title: params["toolCall"]["title"].as_str().unwrap_or("请求执行 OpenCode 工具").into(),
            detail: json!({"toolCallId":params["toolCall"]["toolCallId"],"toolCall":params["toolCall"],"options":params["options"]}).to_string(),
            wire_id: value["id"].clone(), input: params.clone(), allow_once, kind: ApprovalKind::OpenCodeTool,
        };
        if params["sessionId"].as_str() != self.acp.session_id.as_deref()
            || self.acp.stage != Stage::Prompt
            || self.text_only
        {
            step.writes.push(approval_response(&approval, false));
        } else {
            step.events.push(Event::Approval(approval));
        }
    }

    fn acp_update(&mut self, params: &Value, step: &mut Step) {
        if params["sessionId"].as_str() != self.acp.session_id.as_deref() {
            return;
        }
        let update = &params["update"];
        if update["sessionUpdate"] == "config_option_update" {
            self.acp_catalog(update, step);
            return;
        }
        // session/load replays history before its response; local transcripts
        // already contain that history. Only append the new prompt's events.
        if self.acp.stage != Stage::Prompt {
            return;
        }
        match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") if update["content"]["type"] == "text" => {
                step.events.push(Event::Delta {
                    key: update["messageId"].as_str().unwrap_or("acp-reply").into(),
                    text: text(&update["content"], "text").into(),
                });
            }
            Some("tool_call" | "tool_call_update") => {
                let Some(id) = update["toolCallId"].as_str() else {
                    return;
                };
                let merged = self.acp.tools.entry(id.into()).or_insert_with(|| json!({}));
                for (k, v) in update.as_object().into_iter().flatten() {
                    merged[k] = v.clone();
                }
                let status = match merged["status"].as_str() {
                    Some("completed") => "已完成",
                    Some("failed") => "失败",
                    _ => "运行中",
                };
                step.events.push(Event::Tool {
                    key: id.into(),
                    title: merged["title"].as_str().unwrap_or("OpenCode 工具").into(),
                    detail: pretty(&json!({"acpKind":merged["kind"],"content":merged["content"],"output":merged["rawOutput"],"locations":merged["locations"]})),
                    status: status.into(),
                });
                if let Some(input) = merged.get("rawInput") {
                    step.events.push(Event::ToolInput {
                        key: id.into(),
                        input: pretty(input),
                    });
                }
            }
            Some("plan") => {
                let steps = update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| {
                        Some((
                            v["content"].as_str()?.to_owned(),
                            v["status"].as_str()?.to_owned(),
                        ))
                    })
                    .collect();
                step.events.push(Event::Plan { steps });
            }
            Some("usage_update") => {
                self.acp.context_window = update["size"].as_u64().unwrap_or(0);
                // Full token counts arrive with the prompt response. Context
                // updates are not a cumulative session token counter.
            }
            _ => {}
        }
    }

    fn acp_usage(&self, usage: &Value, step: &mut Step) {
        step.events.push(Event::Usage {
            input_tokens: usage["inputTokens"].as_u64().unwrap_or(0),
            cached_input_tokens: usage["cachedReadTokens"].as_u64().unwrap_or(0),
            output_tokens: usage["outputTokens"].as_u64().unwrap_or(0),
            total_tokens: usage["totalTokens"].as_u64().unwrap_or(0),
            context_window: self.acp.context_window,
        });
    }
}

pub(super) fn approval_response(approval: &Approval, allow: bool) -> Value {
    let kind = if allow { "allow_once" } else { "reject_once" };
    let option = approval.input["options"]
        .as_array()
        .and_then(|items| items.iter().find(|v| v["kind"] == kind))
        .and_then(|v| v["optionId"].as_str());
    let outcome = match option {
        Some(id) => json!({"outcome":"selected","optionId":id}),
        None => json!({"outcome":"cancelled"}),
    };
    json!({"jsonrpc":"2.0","id":approval.wire_id,"result":{"outcome":outcome}})
}

#[cfg(test)]
mod tests;
