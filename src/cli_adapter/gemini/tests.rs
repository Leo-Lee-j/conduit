use super::*;
use std::fs;

pub(super) fn request() -> AdapterRequest {
    let mut route = RuntimeRoute::default();
    route.env.insert(
        "GEMINI_CLI_HOME".into(),
        std::env::temp_dir()
            .join("conduit-gemini-missing-home-fixture")
            .to_string_lossy()
            .into_owned(),
    );
    // Fixture launch must not depend on machine-wide Gemini settings.
    route.env.insert(
        "GEMINI_CLI_SYSTEM_SETTINGS_PATH".into(),
        std::env::temp_dir()
            .join("conduit-gemini-missing-system-fixture/settings.json")
            .to_string_lossy()
            .into_owned(),
    );
    AdapterRequest {
        intent: Default::default(),
        cwd: std::env::temp_dir(),
        prompt: "hello".into(),
        native_id: None,
        model: None,
        permission: PermissionLevel::WorkspaceWrite,
        tool_policy: Default::default(),
        effort: ReasoningEffort::Default,
        route,
        text_only: false,
    }
}
fn init() -> Value {
    json!({"id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"gemini-cli","version":"0.71.1"},"agentCapabilities":{"loadSession":true}}})
}
fn session(resume: bool) -> Value {
    let mut result = json!({"modes":{"availableModes":[{"id":"default"},{"id":"yolo"}]},
        "models":{"currentModelId":"auto","availableModels":[{"modelId":"auto","name":"Automatic"},{"modelId":"test-model","name":"Test"}]}});
    if !resume {
        result["sessionId"] = json!("original");
    }
    json!({"id":2,"result":result})
}
fn notification(session: &str, update: Value) -> Value {
    json!({"method":"session/update","params":{"sessionId":session,"update":update}})
}
fn permission(session: &str) -> Value {
    json!({"id":"permission-7","method":"session/request_permission","params":{"sessionId":session,
        "toolCall":{"toolCallId":"tool-1","title":"Edit file","kind":"edit"},
        "options":[{"optionId":"yes-7","kind":"allow_once"},{"optionId":"no-7","kind":"reject_once"}]}})
}
fn ready(req: AdapterRequest) -> (GeminiAdapter, Vec<Value>) {
    let resume = req.native_id.is_some();
    let mut adapter = GeminiAdapter::new(req);
    adapter.launch().unwrap();
    let mut writes = adapter.accept(init()).writes;
    let replay = notification(
        "original",
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old history"}}),
    );
    assert!(adapter.accept(replay).events.is_empty());
    let mut step = adapter.accept(session(resume));
    loop {
        assert!(step.error.is_none(), "{:?}", step.error);
        let message = step.writes[0].clone();
        writes.push(message.clone());
        if message["method"] == "session/prompt" {
            break;
        }
        step = adapter.accept(json!({"id":message["id"],"result":{}}));
    }
    (adapter, writes)
}

#[test]
fn new_and_restored_sessions_negotiate_mode_model_and_suppress_replay() {
    for resume in [false, true] {
        let mut req = request();
        req.native_id = resume.then(|| "original".into());
        req.model = Some("test-model".into());
        let (adapter, writes) = ready(req);
        assert_eq!(
            writes[0]["method"],
            if resume {
                "session/load"
            } else {
                "session/new"
            }
        );
        assert_eq!(
            writes[0]["params"]["cwd"],
            crate::child::normalized_dir(&std::env::temp_dir())
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(writes[1]["method"], "session/set_mode");
        assert_eq!(writes[2]["params"]["modelId"], "test-model");
        assert_eq!(
            writes[3]["params"]["prompt"],
            json!([{"type":"text","text":"hello"}])
        );
        assert_eq!(adapter.session.as_deref(), Some("original"));
        assert_eq!(adapter.settings.models().len(), 2);
        assert!(adapter.settings.variants().is_empty());
        assert!(!adapter.startup_pending());
    }
}

#[test]
fn invalid_handshake_failed_restore_and_mismatched_session_never_send_prompt() {
    for response in [
        json!({"id":1,"result":{}}),
        json!({"id":1,"result":{"protocolVersion":2,"agentInfo":{"name":"gemini-cli"}}}),
    ] {
        let mut adapter = GeminiAdapter::new(request());
        let step = adapter.accept(response);
        assert!(step.done && step.error.is_some() && step.writes.is_empty());
    }
    for response in [
        json!({"id":2,"error":{"code":-32603,"message":"secret credential"}}),
        json!({"id":2,"result":{"sessionId":"different"}}),
    ] {
        let mut req = request();
        req.native_id = Some("original".into());
        let mut adapter = GeminiAdapter::new(req);
        adapter.accept(init());
        let step = adapter.accept(response);
        assert!(step.done && step.writes.is_empty());
        assert!(!step.error.unwrap().contains("secret credential"));
    }
}

#[test]
fn permission_responses_use_advertised_one_time_ids_and_cancel_revokes_grants() {
    let (mut adapter, _) = ready(request());
    let step = adapter.accept(permission("original"));
    let Event::Approval(approval) = &step.events[0] else {
        panic!("approval missing")
    };
    assert_eq!(approval.kind, ApprovalKind::GeminiTool);
    assert_eq!(adapter.automatic_approval(approval), None);
    assert_eq!(
        adapter.approval_response(approval, true)["result"]["outcome"]["optionId"],
        "yes-7"
    );
    assert_eq!(
        adapter.approval_response(approval, false)["result"]["outcome"]["optionId"],
        "no-7"
    );
    let foreign = adapter.accept(permission("other"));
    assert!(foreign.events.is_empty());
    assert_eq!(foreign.writes[0]["result"]["outcome"]["optionId"], "no-7");
    let mut always = approval.clone();
    always.input["options"] = json!([{"kind":"allow_always","optionId":"persistent"}]);
    assert_eq!(
        adapter.approval_response(&always, true)["result"]["outcome"]["outcome"],
        "cancelled"
    );
    assert_eq!(
        adapter.cancel_message().unwrap()["method"],
        "session/cancel"
    );
    assert_eq!(adapter.automatic_approval(approval), Some(false));
    assert_eq!(
        adapter.approval_response(approval, true)["result"]["outcome"]["optionId"],
        "no-7"
    );
}

#[test]
fn cancellation_during_startup_does_not_start_a_turn() {
    let mut adapter = GeminiAdapter::new(request());
    assert!(adapter.cancel_message().is_none());
    let step = adapter.accept(init());
    assert!(step.done && step.writes.is_empty());
}

#[test]
fn launch_preserves_auth_home_and_uses_native_policy_until_drop() {
    let mut req = request();
    req.prompt = "private prompt".into();
    req.route
        .env
        .insert("PRIVATE_TOKEN".into(), "private secret".into());
    let original_environment = req.route.env.clone();
    let mut adapter = GeminiAdapter::new(req);
    let launch = adapter.launch().unwrap();
    assert_eq!(launch.args[..3], ["--acp", "--approval-mode", "default"]);
    assert!(!launch.args.join(" ").contains("private"));
    assert_eq!(launch.environment, original_environment);
    let InitialInput::Json(init) = launch.input else {
        panic!("expected JSON handshake")
    };
    assert_eq!(init["params"]["clientCapabilities"]["terminal"], false);
    let policy = PathBuf::from(&launch.args[4]);
    assert!(fs::read_to_string(&policy).unwrap().contains("ask_user"));
    assert_eq!(launch.args[3], "--admin-policy");
    assert!(
        launch
            .args
            .windows(2)
            .any(|w| w == ["--extensions", "none"])
    );
    drop(adapter);
    assert!(!policy.exists() && !policy.parent().unwrap().exists());
}

#[test]
fn restricted_policies_keep_reads_and_block_mutating_tools_even_after_elevation() {
    for (text_only, automation) in [(false, false), (false, true), (true, false)] {
        let mut req = request();
        req.permission = if automation {
            PermissionLevel::FullAccess
        } else {
            PermissionLevel::ReadOnly
        };
        req.tool_policy = if automation {
            ToolPolicy::Automation
        } else {
            ToolPolicy::default()
        };
        req.text_only = text_only;
        let (mut adapter, _) = ready(req);
        let run = adapter.config.as_ref().unwrap();
        let policy = fs::read_to_string(&run.args[1]).unwrap();
        let parsed: toml::Value = policy.parse().unwrap();
        let rules = parsed["rule"].as_array().unwrap();
        assert_eq!(rules[0]["toolName"].as_str(), Some("*"));
        assert_eq!(rules[0]["decision"].as_str(), Some("deny"));
        assert_eq!(rules.len(), if text_only { 1 } else { 5 });
        if !text_only {
            for name in config::READ_TOOLS {
                assert!(rules.iter().any(|r| r["toolName"].as_str() == Some(name)
                    && r["decision"].as_str() == Some("allow")));
            }
        }
        assert!(run.args.iter().any(|a| a == "--allowed-mcp-server-names"));
        adapter.enable_full_access();
        let step = adapter.accept(permission("original"));
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing approval")
        };
        assert_eq!(adapter.automatic_approval(approval), Some(false));
        assert_eq!(
            adapter.approval_response(approval, true)["result"]["outcome"]["optionId"],
            "no-7"
        );
    }
}

#[test]
fn messages_tools_usage_and_unsupported_requests_have_stable_events() {
    let (mut adapter, writes) = ready(request());
    let update = |u| notification("original", u);
    assert!(!adapter.accept(update(json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking"}}))).events.is_empty());
    let step = adapter.accept(update(
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"answer"}}),
    ));
    assert!(
        step.events
            .iter()
            .any(|e| matches!(e,Event::Delta{text,..} if text=="answer"))
    );
    adapter.accept(update(json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"Read file","kind":"read","rawInput":{"path":"a.txt"}})));
    let step = adapter.accept(update(json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed","content":[{"type":"content","content":{"type":"image","data":"SECRET_MEDIA"}}],"_meta":{"secret":"SECRET_META"}})));
    assert!(step.events.iter().any(|e| matches!(e,Event::Tool{title,detail,..} if title=="Read file" && !detail.contains("SECRET"))));
    let step = adapter.accept(json!({"id":"reverse","method":"terminal/create","params":{}}));
    assert_eq!(step.writes[0]["error"]["code"], -32601);
    adapter.accept(update(
        json!({"sessionUpdate":"usage_update","size":1000,"used":100}),
    ));
    let step = adapter.accept(json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":"end_turn","usage":{"inputTokens":0,"outputTokens":10,"totalTokens":10}}}));
    assert!(step.done && step.error.is_none());
    assert!(step.events.iter().any(|e| matches!(e,Event::Usage(u) if u.input_tokens_known && u.output_tokens_known && u.output_tokens==10 && u.context_tokens==Some(100) && u.context_window==1000)));
    let (mut adapter, writes) = ready(request());
    let step = adapter
        .accept(json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":"max_tokens","usage":{"inputTokens":8,"outputTokens":5}}}));
    assert!(step.done && step.error.is_some());
    assert!(
        step.events
            .iter()
            .any(|e| matches!(e, Event::Usage(u) if u.input_tokens==8 && u.output_tokens==5))
    );
}

#[test]
fn command_and_unsupported_route_inputs_are_rejected_before_launch() {
    for prompt in [" /help", "$danger"] {
        let mut req = request();
        req.prompt = prompt.into();
        assert!(GeminiAdapter::new(req).launch().is_err());
    }
    let mut req = request();
    req.route.config.insert("unsupported".into(), json!(true));
    assert!(GeminiAdapter::new(req).launch().is_err());
    let mut req = request();
    req.native_id = Some("../escape".into());
    assert!(GeminiAdapter::new(req).launch().is_err());
}
