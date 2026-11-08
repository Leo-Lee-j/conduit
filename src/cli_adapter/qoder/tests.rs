use super::*;

fn request() -> AdapterRequest {
    AdapterRequest {
        cwd: PathBuf::from("C:/work"),
        prompt: "hello".into(),
        native_id: None,
        model: None,
        permission: PermissionLevel::WorkspaceWrite,
        effort: ReasoningEffort::Default,
        route: RuntimeRoute::default(),
        text_only: false,
    }
}
fn init(resume: bool) -> Value {
    json!({"id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"qoder-cli","version":"1.1.85"},
        "agentCapabilities":{"loadSession":true,"sessionCapabilities":if resume {json!({"resume":{}})} else {json!({})}}}})
}
fn catalog() -> Value {
    json!([{"id":"model","currentValue":"auto","options":[{"value":"auto","name":"Automatic"},{"value":"test/model","name":"Test"}]},
        {"id":"reasoning_effort","currentValue":"low","options":[{"value":"low","name":"Low"},{"value":"high","name":"High"}]}])
}
fn ready(request: AdapterRequest, resume: bool) -> (QoderAdapter, Vec<Value>) {
    let mut adapter = QoderAdapter::new(request);
    adapter.launch().unwrap();
    let mut writes = adapter.accept(init(resume)).writes;
    let step =
        adapter.accept(json!({"id":2,"result":{"sessionId":"original","configOptions":catalog()}}));
    assert!(step.error.is_none(), "{:?}", step.error);
    let mut next = step;
    while next
        .writes
        .first()
        .is_some_and(|v| v["method"] != "session/prompt")
    {
        let message = next.writes[0].clone();
        writes.push(message.clone());
        next = adapter.accept(json!({"id":message["id"],"result":{"configOptions":catalog()}}));
        assert!(next.error.is_none(), "{:?}", next.error);
    }
    writes.extend(next.writes);
    assert_eq!(writes.last().unwrap()["method"], "session/prompt");
    (adapter, writes)
}
fn notification(session: &str, update: Value) -> Value {
    json!({"method":"session/update","params":{"sessionId":session,"update":update}})
}
fn permission(session: &str) -> Value {
    json!({"id":"permission-7","method":"session/request_permission","params":{"sessionId":session,
        "toolCall":{"toolCallId":"tool-1","title":"Edit file","kind":"edit"},
        "options":[{"optionId":"yes-7","kind":"allow_once"},{"optionId":"no-7","kind":"reject_once"}]}})
}
fn form(session: &str) -> Value {
    json!({"id":73,"method":"elicitation/create","params":{"sessionId":session,"mode":"form","message":"Choose",
        "requestedSchema":{"type":"object","properties":{
            "choice":{"type":"string","title":"Choice","oneOf":[{"const":"a","title":"Alpha"},{"const":"b","title":"Beta"}]},
            "many":{"type":"array","title":"Many","maxItems":2,"items":{"type":"string","anyOf":[{"const":"x","title":"X"},{"const":"y","title":"Y"}]}},
            "name":{"type":"string","title":"Name","minLength":2,"maxLength":10}}}}})
}

#[test]
fn qoder_launch_preserves_cwd_and_never_puts_prompt_or_secret_on_command_line() {
    let mut req = request();
    req.prompt = "private prompt".into();
    req.route
        .env
        .insert("PRIVATE_TOKEN".into(), "private secret".into());
    let mut adapter = QoderAdapter::new(req);
    let launch = adapter.launch().unwrap();
    assert_eq!(launch.args[0], "--acp");
    assert!(launch.args.windows(2).any(|w| w == ["--cwd", "C:/work"]));
    assert!(!launch.args.join(" ").contains("private"));
    assert_eq!(launch.environment["PRIVATE_TOKEN"], "private secret");
    let InitialInput::Json(init) = launch.input else {
        panic!("JSON handshake required")
    };
    assert_eq!(init["params"]["clientCapabilities"]["terminal"], false);
    assert!(init["params"]["clientCapabilities"]["elicitation"]["form"].is_object());
    assert!(launch.completion == Completion::ProtocolEvent);
}

#[test]
fn qoder_new_resume_and_load_configure_model_and_effort_before_prompt() {
    for (native, resume) in [
        (None, true),
        (Some("original"), true),
        (Some("original"), false),
    ] {
        let mut req = request();
        req.native_id = native.map(str::to_owned);
        req.model = Some("test/model".into());
        req.route.acp_variant = Some("high".into());
        let (mut adapter, writes) = ready(req, resume);
        assert_eq!(
            writes[0]["method"],
            if native.is_none() {
                "session/new"
            } else if resume {
                "session/resume"
            } else {
                "session/load"
            }
        );
        if native.is_some() {
            assert_eq!(writes[0]["params"]["sessionId"], "original");
        }
        assert_eq!(writes[1]["method"], "session/set_mode");
        assert_eq!(writes[2]["params"]["modelId"], "test/model");
        assert_eq!(writes[3]["params"]["configId"], "reasoning_effort");
        assert_eq!(writes[3]["params"]["value"], "high");
        assert_eq!(
            writes.last().unwrap()["params"]["prompt"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(!adapter.startup_pending());
        assert!(
            !adapter
                .accept(json!({"id":2,"result":{"stopReason":"end_turn"}}))
                .done
        );
        assert!(
            adapter
                .accept(
                    json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":"end_turn"}})
                )
                .done
        );
    }
}

#[test]
fn qoder_resume_never_silently_replaces_session_or_replays_history() {
    let mut req = request();
    req.native_id = Some("original".into());
    let mut adapter = QoderAdapter::new(req);
    adapter.accept(init(false));
    let replay = notification(
        "original",
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old"}}),
    );
    assert!(adapter.accept(replay).events.is_empty());
    let step = adapter.accept(json!({"id":2,"result":{"sessionId":"different"}}));
    assert!(step.done && step.error.is_some() && step.writes.is_empty());
    assert!(!step.events.iter().any(|e| matches!(e, Event::NativeId(_))));
    let mut req = request();
    req.native_id = Some("original".into());
    let mut adapter = QoderAdapter::new(req);
    let step = adapter
        .accept(json!({"id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"qoder-cli"}}}));
    assert!(step.done && step.error.is_some() && step.writes.is_empty());
}

#[test]
fn qoder_streams_merge_tools_filter_sessions_and_keep_private_thoughts_out() {
    let (mut adapter, writes) = ready(request(), true);
    let text =
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}});
    assert!(
        adapter
            .accept(notification("foreign", text.clone()))
            .events
            .is_empty()
    );
    assert!(
        matches!(&adapter.accept(notification("original",text)).events[0],Event::Delta {text,..} if text=="hello")
    );
    assert!(adapter.accept(notification("original",json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"private"}}))).events.is_empty());
    adapter.accept(notification("original",json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"Read file","rawInput":{"path":"x"}})));
    let tool = adapter.accept(notification(
        "original",
        json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed"}),
    ));
    assert!(matches!(&tool.events[0],Event::Tool {title,..} if title=="Read file"));
    assert!(
        tool.events
            .iter()
            .any(|e| matches!(e,Event::ToolInput{input,..} if input.contains("path")))
    );
    adapter.accept(notification(
        "original",
        json!({"sessionUpdate":"usage_update","size":1000,"used":250}),
    ));
    let done=adapter.accept(json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":"end_turn","usage":{"inputTokens":100,"outputTokens":10,"totalTokens":110}}}));
    assert!(matches!(
        &done.events[0],
        Event::Usage(Usage {
            context_tokens: Some(250),
            total_tokens: 110,
            ..
        })
    ));
    assert!(done.done && done.error.is_none());
}

#[test]
fn qoder_permissions_preserve_wire_ids_one_shot_choices_and_helper_restrictions() {
    for permission_level in [
        PermissionLevel::ReadOnly,
        PermissionLevel::WorkspaceWrite,
        PermissionLevel::FullAccess,
    ] {
        let mut req = request();
        req.permission = permission_level;
        let (mut adapter, _) = ready(req, true);
        let step = adapter.accept(permission("original"));
        let Event::Approval(approval) = &step.events[0] else {
            panic!("approval")
        };
        assert_eq!(
            adapter.automatic_approval(approval),
            match permission_level {
                PermissionLevel::ReadOnly => Some(false),
                PermissionLevel::WorkspaceWrite => None,
                PermissionLevel::FullAccess => Some(true),
            }
        );
        let response = adapter.approval_response(approval, true);
        assert_eq!(response["id"], "permission-7");
        assert_eq!(
            response["result"]["outcome"]["optionId"],
            if permission_level == PermissionLevel::ReadOnly {
                "no-7"
            } else {
                "yes-7"
            }
        );
        assert!(adapter.accept(permission("foreign")).events.is_empty());
    }
    let mut req = request();
    req.text_only = true;
    req.permission = PermissionLevel::FullAccess;
    let (mut adapter, _) = ready(req, true);
    adapter.enable_full_access();
    let launch = adapter.launch().unwrap();
    assert!(launch.args.windows(2).any(|w| w == ["--tools", ""]));
    assert!(launch.args.iter().any(|w| w == "--strict-mcp-config"));
    let step = adapter.accept(permission("original"));
    let Event::Approval(a) = &step.events[0] else {
        panic!("approval")
    };
    assert_eq!(adapter.automatic_approval(a), Some(false));
    assert_eq!(
        adapter.approval_response(a, true)["result"]["outcome"]["optionId"],
        "no-7"
    );
    let cancelled = adapter.accept(form("original"));
    assert!(cancelled.events.is_empty());
    assert_eq!(cancelled.writes[0]["result"]["action"], "cancel");
}

#[test]
fn qoder_elicitation_supports_choices_multi_select_validation_and_cancellation() {
    let mut req = request();
    req.permission = PermissionLevel::FullAccess;
    let (mut adapter, _) = ready(req, true);
    let step = adapter.accept(form("original"));
    let Event::Approval(approval) = &step.events[0] else {
        panic!("question")
    };
    assert!(approval.is_user_input());
    assert_eq!(adapter.automatic_approval(approval), None);
    assert!(
        approval
            .questions()
            .iter()
            .any(|q| q.id == "many" && q.multiple && !q.custom)
    );
    let mut answers = BTreeMap::from([
        ("choice".into(), vec!["Beta".into()]),
        ("many".into(), vec!["X".into(), "Y".into()]),
        ("name".into(), vec!["name".into()]),
    ]);
    let response = adapter.user_input_response(approval, &answers).unwrap();
    assert_eq!(response["id"], 73);
    assert_eq!(
        response["result"]["content"],
        json!({"choice":"b","many":["x","y"],"name":"name"})
    );
    answers.insert("choice".into(), vec!["unknown".into()]);
    assert!(adapter.user_input_response(approval, &answers).is_none());
    assert_eq!(
        adapter.approval_response(approval, false)["result"]["action"],
        "cancel"
    );
    assert!(adapter.accept(form("foreign")).events.is_empty());
    let mut malformed = form("original");
    malformed["params"]["requestedSchema"]["properties"]["name"]["type"] = json!("object");
    let step = adapter.accept(malformed);
    assert!(step.events.is_empty());
    assert_eq!(step.writes[0]["error"]["code"], -32602);
}

#[test]
fn qoder_cancel_errors_and_unsupported_selections_fail_without_sending_a_prompt() {
    let (adapter, _) = ready(request(), true);
    assert_eq!(
        adapter.cancel_message().unwrap()["params"]["sessionId"],
        "original"
    );
    for (model, effort) in [(Some("missing"), None), (None, Some("max"))] {
        let mut req = request();
        req.model = model.map(str::to_owned);
        req.route.acp_variant = effort.map(str::to_owned);
        let mut adapter = QoderAdapter::new(req);
        adapter.accept(init(true));
        adapter.accept(json!({"id":2,"result":{"sessionId":"original","configOptions":catalog()}}));
        let step = adapter.accept(json!({"id":3,"result":{}}));
        assert!(step.done && step.error.is_some() && step.writes.is_empty());
    }
    let mut adapter = QoderAdapter::new(request());
    adapter.accept(init(true));
    let step = adapter.accept(json!({"id":2,"error":{"code":-32000,"message":"secret-token"}}));
    assert!(step.done && step.writes.is_empty());
    assert!(!step.error.unwrap().contains("secret-token"));
}

#[test]
fn qoder_native_modes_catalog_refresh_and_non_success_stops_are_checked() {
    let mut req = request();
    req.permission = PermissionLevel::FullAccess;
    let mut adapter = QoderAdapter::new(req);
    adapter.accept(init(true));
    let step = adapter.accept(json!({"id":2,"result":{"sessionId":"original",
        "modes":{"availableModes":[{"id":"default"},{"id":"yolo"}]}, "configOptions":catalog()}}));
    assert_eq!(step.writes[0]["params"]["modeId"], "yolo");
    let mut refreshed = catalog();
    refreshed[1]["options"] = json!([{"value":"medium","name":"Medium"}]);
    refreshed[0]["_meta"] = json!({"secret":"never persisted"});
    adapter.accept(notification(
        "original",
        json!({"sessionUpdate":"config_option_update","configOptions":refreshed}),
    ));
    assert!(!adapter.settings.options.to_string().contains("secret"));
    let step = adapter.accept(json!({"id":3,"result":{}}));
    assert_eq!(step.writes[0]["params"]["value"], "medium");
    for reason in [
        "cancelled",
        "refusal",
        "max_tokens",
        "max_turn_requests",
        "invalid",
    ] {
        let (mut adapter, writes) = ready(request(), true);
        let step = adapter
            .accept(json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":reason}}));
        assert!(step.done && step.error.is_some());
    }
    let mut adapter = QoderAdapter::new(request());
    adapter.accept(init(true));
    let step = adapter.accept(json!({"id":2,"result":{"sessionId":"original","modes":{"availableModes":[{"id":"yolo"}]}}}));
    assert!(step.done && step.error.is_some() && step.writes.is_empty());
}

#[test]
fn qoder_forms_reject_unsupported_constraints_and_preserve_numeric_types() {
    let (mut adapter, _) = ready(request(), true);
    let mut message = form("original");
    message["params"].as_object_mut().unwrap().remove("mode");
    message["params"]["requestedSchema"]["properties"] = json!({
        "count":{"type":"integer","minimum":1,"maximum":5},
        "enabled":{"type":"boolean"}
    });
    let step = adapter.accept(message.clone());
    let Event::Approval(approval) = &step.events[0] else {
        panic!("expected form")
    };
    let mut answers = BTreeMap::from([
        ("count".into(), vec!["3".into()]),
        ("enabled".into(), vec!["true".into()]),
    ]);
    let response = adapter.user_input_response(approval, &answers).unwrap();
    assert_eq!(
        response["result"]["content"],
        json!({"count":3,"enabled":true})
    );
    answers.insert("count".into(), vec!["6".into()]);
    assert!(adapter.user_input_response(approval, &answers).is_none());
    message["params"]["requestedSchema"]["properties"] =
        json!({"email":{"type":"string","format":"email"}});
    let step = adapter.accept(message);
    assert!(step.events.is_empty());
    assert_eq!(step.writes[0]["error"]["code"], -32602);
}

#[test]
fn qoder_resume_accepts_native_response_without_echoed_session_id() {
    let mut req = request();
    req.native_id = Some("original".into());
    let mut adapter = QoderAdapter::new(req);
    adapter.accept(init(true));
    // Qoder 1.1.85 omits sessionId from successful session/resume responses.
    let step = adapter.accept(json!({"id":2,"result":{"configOptions":catalog(),
        "modes":{"availableModes":[{"id":"default"},{"id":"yolo"}]}}}));
    assert!(step.error.is_none());
    assert!(matches!(&step.events[1], Event::NativeId(id) if id == "original"));
    assert_eq!(step.writes[0]["params"]["sessionId"], "original");
    let mut adapter = QoderAdapter::new(request());
    adapter.accept(init(true));
    let step = adapter.accept(json!({"id":2,"result":{}}));
    assert!(step.done && step.error.is_some() && step.writes.is_empty());
}
