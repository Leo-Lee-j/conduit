use super::*;

fn options(agent: &str) -> Value {
    json!([
        {"id":"mode","options":[{"value":agent,"name":"Conduit"}]},
        {"id":"model","currentValue":"test/model","options":[{"value":"test/model","name":"Test model"}]},
        {"id":"effort","options":[{"value":"default","name":"Default"},{"value":"max","name":"Max"}]}
    ])
}

fn connected(native: Option<&str>, resume: bool, variant: Option<&str>) -> (Protocol, Vec<Value>) {
    let mut protocol = Protocol::new(
        Provider::OpenCode,
        "C:/work".into(),
        "hello".into(),
        native.map(str::to_owned),
    )
    .with_model(Some("test/model".into()))
    .with_options(
        Default::default(),
        crate::providers::RuntimeRoute {
            opencode_variant: variant.map(str::to_owned),
            ..Default::default()
        },
    );
    protocol.prepare().unwrap();
    let session_caps = if resume {
        json!({"resume":{}})
    } else {
        json!({})
    };
    let init = protocol.accept(json!({"id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"OpenCode"},"agentCapabilities":{"loadSession":true,"sessionCapabilities":session_caps}}}));
    let mut writes = init.writes;
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
    // A history replay must never be appended to the current turn.
    let replay = protocol.accept(json!({"method":"session/update","params":{"sessionId":"ses_test","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old text"}}}}));
    assert!(replay.events.is_empty());
    let options = options(&protocol.acp.agent);
    let session =
        protocol.accept(json!({"id":2,"result":{"sessionId":"ses_test","configOptions":options}}));
    assert!(
        session
            .events
            .iter()
            .any(|e| matches!(e, Event::NativeId(id) if id == "ses_test"))
    );
    let mut next = session;
    while next
        .writes
        .first()
        .is_some_and(|v| v["method"] == "session/set_config_option")
    {
        let request = next.writes[0].clone();
        writes.push(request.clone());
        next = protocol.accept(json!({"id":request["id"],"result":{"configOptions":options}}));
    }
    assert!(!next.done, "{:?}", next.error);
    writes.extend(next.writes);
    assert_eq!(writes.last().unwrap()["method"], "session/prompt");
    (protocol, writes)
}

#[test]
fn opencode_new_load_and_resume_apply_model_and_variant_before_prompt() {
    for (native, resume) in [
        (None, true),
        (Some("ses_test"), true),
        (Some("ses_test"), false),
    ] {
        let (mut protocol, writes) = connected(native, resume, Some("max"));
        let configs: Vec<_> = writes
            .iter()
            .filter(|v| v["method"] == "session/set_config_option")
            .collect();
        assert_eq!(configs.len(), 3);
        assert_eq!(configs[0]["params"]["configId"], "mode");
        assert_eq!(configs[1]["params"]["configId"], "model");
        assert_eq!(configs[2]["params"]["value"], "max");
        let stale = protocol.accept(json!({"id":2,"result":{"stopReason":"end_turn"}}));
        assert!(!stale.done);
        let chunk = protocol.accept(json!({"method":"session/update","params":{"sessionId":"ses_test","update":{"sessionUpdate":"agent_message_chunk","messageId":"msg_a","content":{"type":"text","text":"new text"}}}}));
        assert!(
            matches!(&chunk.events[0], Event::Delta {key,text} if key == "msg_a" && text == "new text")
        );
        let done = protocol.accept(json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":"end_turn","usage":{"inputTokens":10,"outputTokens":2,"cachedReadTokens":4,"totalTokens":16}}}));
        assert!(done.done && done.error.is_none());
        assert!(matches!(
            done.events[0],
            Event::Usage(Usage {
                total_tokens: 16,
                ..
            })
        ));
    }
}

#[test]
fn opencode_unsupported_effort_fails_without_prompt_or_silent_fallback() {
    let (mut protocol, _) = connected(None, true, None);
    protocol.acp.stage = Stage::Configure;
    protocol
        .acp
        .config
        .push_back(("effort".into(), "xhigh".into()));
    let mut step = Step::new();
    protocol.acp_next(&mut step);
    assert!(step.done);
    assert!(step.error.unwrap().contains("xhigh"));
    assert!(step.writes.is_empty());
}

#[test]
fn opencode_automatic_effort_uses_medium_only_when_advertised() {
    let (mut protocol, _) = connected(None, true, None);
    for (levels, explicit, expected) in [
        (vec!["default", "medium", "high"], None, Some("medium")),
        (vec!["default", "high"], None, Some("default")),
        (vec![], None, None),
        (vec!["default", "medium"], Some("default"), Some("default")),
        (
            vec!["default", "medium", "high"],
            Some("high"),
            Some("high"),
        ),
    ] {
        protocol.acp.stage = Stage::Configure;
        protocol.route.opencode_variant = explicit.map(str::to_owned);
        let options: Vec<_> = levels
            .into_iter()
            .map(|value| json!({"value":value,"name":value}))
            .collect();
        protocol.acp.options.options = if options.is_empty() {
            json!([])
        } else {
            json!([{"id":"effort","options":options}])
        };
        protocol
            .acp
            .config
            .push_back(("effort".into(), explicit.unwrap_or("default").into()));
        let mut step = Step::new();
        protocol.acp_next(&mut step);
        assert!(!step.done, "{:?}", step.error);
        let request = &step.writes[0];
        if let Some(expected) = expected {
            assert_eq!(request["method"], "session/set_config_option");
            assert_eq!(request["params"]["value"], expected);
        } else {
            assert_eq!(request["method"], "session/prompt");
        }
    }
}

fn permission(session: &str) -> Value {
    json!({"jsonrpc":"2.0","id":"permission-7","method":"session/request_permission","params":{
        "sessionId":session,"toolCall":{"toolCallId":"tool-1","title":"write file"},
        "options":[{"kind":"allow_always","optionId":"permanent"},{"kind":"reject_once","optionId":"no"},{"kind":"allow_once","optionId":"yes"}]}})
}

#[test]
fn opencode_permission_uses_option_kind_and_preserves_tool_and_wire_ids() {
    let (mut protocol, _) = connected(None, true, None);
    let step = protocol.accept(permission("ses_test"));
    let Event::Approval(approval) = &step.events[0] else {
        panic!("missing approval")
    };
    assert_eq!(approval.wire_id, "permission-7");
    assert_eq!(
        serde_json::from_str::<Value>(&approval.detail).unwrap()["toolCallId"],
        "tool-1"
    );
    assert_eq!(
        protocol.approval_response(approval, true)["result"]["outcome"]["optionId"],
        "yes"
    );
    assert_eq!(
        protocol.approval_response(approval, false)["result"]["outcome"]["optionId"],
        "no"
    );
    let wrong = protocol.accept(permission("ses_other"));
    assert!(wrong.events.is_empty());
    assert_eq!(wrong.writes[0]["result"]["outcome"]["optionId"], "no");
    let request = json!({"id":99,"method":"fs/write_text_file","params":{"path":"C:/forbidden"}});
    assert_eq!(protocol.accept(request).writes[0]["error"]["code"], -32601);
}

#[test]
fn opencode_tool_updates_merge_input_and_title_and_ignore_other_session() {
    let (mut protocol, _) = connected(None, true, None);
    for update in [
        json!({"sessionUpdate":"tool_call","toolCallId":"tool","title":"read file","status":"pending","rawInput":{"path":"a.rs"}}),
        json!({"sessionUpdate":"tool_call_update","toolCallId":"tool","status":"completed","rawOutput":"contents"}),
    ] {
        let step = protocol.accept(
            json!({"method":"session/update","params":{"sessionId":"ses_test","update":update}}),
        );
        assert!(matches!(&step.events[0], Event::Tool {title,..} if title == "read file"));
        assert!(matches!(&step.events[1], Event::ToolInput {input,..} if input.contains("a.rs")));
    }
    assert!(protocol.accept(json!({"method":"session/update","params":{"sessionId":"other","update":{"sessionUpdate":"tool_call","toolCallId":"tool"}}})).events.is_empty());
    assert_eq!(
        protocol.cancel_message().unwrap()["params"]["sessionId"],
        "ses_test"
    );
}

#[test]
fn opencode_managed_agents_override_inherited_rules_and_helpers_have_private_storage() {
    for permission in [
        PermissionLevel::ReadOnly,
        PermissionLevel::WorkspaceWrite,
        PermissionLevel::FullAccess,
    ] {
        let mut protocol = Protocol::new(Provider::OpenCode, "C:/work".into(), "hi".into(), None)
            .with_permission(permission);
        protocol.route.env.insert(
            "OPENCODE_CONFIG_CONTENT".into(),
            json!({"permission":{"bash":"allow","custom_tool":"allow"},"provider":{"keep":{}}})
                .to_string(),
        );
        protocol.prepare().unwrap();
        let config: Value =
            serde_json::from_str(&protocol.environment()["OPENCODE_CONFIG_CONTENT"]).unwrap();
        assert!(config["provider"].get("keep").is_some());
        let agent = &config["agent"][&protocol.acp.agent];
        assert_eq!(agent["permission"]["question"], "allow");
        assert_eq!(
            protocol.environment()["OPENCODE_ENABLE_QUESTION_TOOL"],
            "true"
        );
        assert_eq!(
            agent["permission"]["*"],
            match permission {
                PermissionLevel::ReadOnly => "deny",
                PermissionLevel::WorkspaceWrite => "ask",
                PermissionLevel::FullAccess => "allow",
            }
        );
        assert!(protocol.args().contains(&"--pure".to_string()));
        let approval = Approval {
            id: "a".into(),
            title: String::new(),
            detail: String::new(),
            wire_id: json!(1),
            input: Value::Null,
            allow_once: true,
            kind: ApprovalKind::OpenCodeTool,
        };
        assert_eq!(
            protocol.automatic_approval(&approval),
            match permission {
                PermissionLevel::ReadOnly => Some(false),
                PermissionLevel::FullAccess => Some(true),
                _ => None,
            }
        );
    }
    let mut helper =
        Protocol::new(Provider::OpenCode, "C:/work".into(), "title".into(), None).text_only();
    helper.prepare().unwrap();
    let path = helper.acp.helper_directory.clone().unwrap();
    assert!(path.is_dir());
    assert!(helper.environment()["OPENCODE_DB"].starts_with(path.to_str().unwrap()));
    assert!(
        !helper
            .environment()
            .contains_key("OPENCODE_DISABLE_PROJECT_CONFIG"),
        "helpers must retain workspace-native provider configuration"
    );
    let config: Value =
        serde_json::from_str(&helper.environment()["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(
        config["agent"][&helper.acp.agent]["permission"]["*"],
        "deny"
    );
    drop(helper);
    assert!(!path.exists());
}

#[test]
fn opencode_user_input_answers_preserve_order_and_wait_in_full_access() {
    let (mut protocol, _) = connected(None, true, None);
    protocol.enable_full_access();
    let input = json!({"id":"que_test","sessionID":"ses_test","questions":[
        {"header":"功能","question":"需要哪些功能？","multiple":true,"custom":false,"options":[
            {"label":"A","description":"功能 A"},{"label":"B","description":"功能 B"}]},
        {"header":"名称","question":"项目叫什么？","options":[]}
    ]});
    let step = protocol.accept(json!({"id":"que_test","method":"conduit/question","params":input}));
    assert!(step.writes.is_empty());
    let Event::Approval(approval) = &step.events[0] else {
        panic!("question not queued");
    };
    assert_eq!(protocol.automatic_approval(approval), None);
    let answers = std::collections::BTreeMap::from([
        ("0".into(), vec!["A".into(), "B".into()]),
        ("1".into(), vec!["中文项目".into()]),
    ]);
    assert_eq!(
        protocol.user_input_response(approval, &answers).unwrap(),
        json!({"answers":[["A","B"],["中文项目"]]})
    );
    assert_eq!(
        protocol.approval_response(approval, false),
        json!({"reject":true})
    );
    assert!(
        protocol
            .accept(
                json!({"id":"other","method":"conduit/question","params":{"sessionID":"other"}})
            )
            .events
            .is_empty()
    );
}

#[test]
fn opencode_text_helpers_report_api_errors_without_waiting_for_acp_idle() {
    let mut helper = Protocol::new(Provider::OpenCode, "C:/work".into(), "title".into(), None)
        .with_model(Some("test/model".into()))
        .text_only();
    helper.prepare().unwrap();
    assert_eq!(&helper.args()[..4], ["run", "--pure", "--format", "json"]);
    assert_eq!(helper.opencode_text_input().as_deref(), Some("title"));
    assert!(helper.cancel_message().is_none());
    let config: Value =
        serde_json::from_str(&helper.environment()["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(config["tools"]["*"], false);
    assert_eq!(config["agent"]["title"]["disable"], true);
    let text = helper.accept(json!({"type":"text","part":{"id":"reply","text":"短标题"}}));
    assert!(matches!(&text.events[0], Event::Text { text, .. } if text == "短标题"));
    let failed = helper.accept(json!({"type":"error","error":{"name":"APIError","data":{"message":"Free usage limit reached"}}}));
    assert!(failed.done);
    assert!(failed.error.unwrap().contains("Free usage limit reached"));
}

#[test]
fn opencode_context_uses_reported_used_not_prompt_total() {
    let (mut protocol, writes) = connected(None, true, None);
    protocol.accept(
        json!({"method":"session/update","params":{"sessionId":"ses_test","update":{
            "sessionUpdate":"usage_update","size":200000,"used":40000
        }}}),
    );
    let done = protocol.accept(
        json!({"id":writes.last().unwrap()["id"],"result":{"stopReason":"end_turn","usage":{
            "inputTokens":290000,"outputTokens":10000,"totalTokens":300000
        }}}),
    );
    let usage = done
        .events
        .iter()
        .find_map(|event| match event {
            Event::Usage(usage) => Some(usage),
            _ => None,
        })
        .unwrap();
    assert_eq!(usage.context_tokens, Some(40_000));
    assert_eq!(usage.context_ratio(), Some(0.2));
    assert_eq!(usage.turn_output_tokens, Some(10_000));
}

#[test]
fn full_access_opencode_permission_never_requires_a_ui_gate() {
    for (options, allowed, expected) in [
        (
            json!([{"kind":"allow_always","optionId":"permanent"},
            {"kind":"allow_once","optionId":"yes"}]),
            true,
            json!({"outcome":"selected","optionId":"yes"}),
        ),
        (
            json!([{"kind":"allow_always","optionId":"permanent"},
            {"kind":"reject_once","optionId":"no"}]),
            true,
            json!({"outcome":"selected","optionId":"permanent"}),
        ),
        (
            json!([{"kind":"reject_once","optionId":"no"}]),
            false,
            json!({"outcome":"selected","optionId":"no"}),
        ),
        (
            json!([{"kind":"allow_once"}]),
            false,
            json!({"outcome":"cancelled"}),
        ),
    ] {
        let (protocol, _) = connected(Some("ses_test"), true, None);
        let mut full = protocol.with_permission(PermissionLevel::FullAccess);
        let mut request = permission("ses_test");
        request["params"]["options"] = options;
        let step = full.accept(request);
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing approval")
        };
        assert_eq!(full.automatic_approval(approval), Some(allowed));
        assert_eq!(
            full.approval_response(approval, allowed)["result"]["outcome"],
            expected
        );
        // A user-facing "allow once" action may never become a permanent grant.
        let normal = Protocol::new(Provider::OpenCode, "C:/work".into(), "hello".into(), None);
        assert_eq!(normal.automatic_approval(approval), None);
        if !approval.allow_once {
            assert_eq!(
                normal.approval_response(approval, true)["result"]["outcome"]["outcome"],
                "cancelled"
            );
        }
        let readonly = full.with_permission(PermissionLevel::ReadOnly);
        assert_eq!(readonly.automatic_approval(approval), Some(false));
    }
}
