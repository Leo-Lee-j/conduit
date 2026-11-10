use super::*;
use crate::{
    domain::Provider,
    protocol::{ApprovalKind, Event},
};
use serde_json::json;

fn request() -> AdapterRequest {
    let mut route = RuntimeRoute::default();
    route
        .env
        .insert("OPENCODE_CONFIG_CONTENT".into(), "{}".into());
    AdapterRequest {
        intent: Default::default(),
        cwd: PathBuf::from("C:/fixture work"),
        prompt: "hello".into(),
        native_id: None,
        model: Some("test/model".into()),
        permission: PermissionLevel::WorkspaceWrite,
        effort: Default::default(),
        route,
        text_only: false,
    }
}

#[test]
fn registered_factories_preserve_model_resume_and_handshake_contracts() {
    let _locale = crate::test_support::chinese();
    for provider in [Provider::Claude, Provider::Codex] {
        let mut request = request();
        request.native_id = Some("original".into());
        let mut adapter = (provider.descriptor().adapter)(request);
        let launch = adapter.launch().unwrap();
        assert!(matches!(launch.input, InitialInput::Json(_)));
        assert!(launch.completion == Completion::ProtocolEvent);
        assert!(adapter.startup_pending());
        // Unrelated replies must not suppress the startup timeout.
        adapter.accept(json!({"id":99,"result":{}}));
        assert!(adapter.startup_pending());
        if provider == Provider::Claude {
            assert!(launch.args.iter().any(|a| a == "--resume=original"));
            assert!(
                launch
                    .args
                    .windows(2)
                    .any(|p| p == ["--model", "test/model"])
            );
            adapter.accept(json!({"type":"control_response","response":{"request_id":"other","subtype":"success"}}));
            assert!(adapter.startup_pending());
            let step = adapter.accept(json!({"type":"control_response","response":{"request_id":"conduit-init","subtype":"success"}}));
            assert_eq!(step.writes[0]["message"]["content"], "hello");
            assert!(!adapter.startup_pending());
            assert!(
                adapter
                    .accept(json!({"type":"system","session_id":"wrong"}))
                    .error
                    .is_some()
            );
        } else {
            let step = adapter.accept(json!({"id":1,"result":{}}));
            assert_eq!(step.writes[1]["method"], "thread/resume");
            assert_eq!(step.writes[1]["params"]["threadId"], "original");
            assert_eq!(step.writes[1]["params"]["model"], "test/model");
            assert!(!adapter.startup_pending());
            assert!(
                adapter
                    .accept(json!({"id":2,"result":{"thread":{"id":"wrong"}}}))
                    .error
                    .is_some()
            );
        }
    }
}

#[test]
fn opencode_launch_has_final_port_and_keeps_helper_resources_until_drop() {
    let _locale = crate::test_support::chinese();
    let mut adapter = (Provider::OpenCode.descriptor().adapter)(request());
    let launch = adapter.launch().unwrap();
    let port = launch.args.windows(2).find(|p| p[0] == "--port").unwrap()[1]
        .parse::<u16>()
        .unwrap();
    assert_ne!(port, 0);
    assert!(launch.environment.contains_key("OPENCODE_SERVER_PASSWORD"));
    assert!(
        !launch
            .args
            .iter()
            .any(|a| a.contains(&launch.environment["OPENCODE_SERVER_PASSWORD"]))
    );
    assert!(
        adapter.poll().unwrap().is_empty(),
        "no polling before session is ready"
    );
    drop(adapter);

    let mut helper = request();
    helper.text_only = true;
    let mut adapter = (Provider::OpenCode.descriptor().adapter)(helper);
    let launch = adapter.launch().unwrap();
    assert_eq!(launch.args[..4], ["run", "--pure", "--format", "json"]);
    assert!(matches!(&launch.input, InitialInput::TextToEof(text) if text == "hello"));
    assert!(launch.completion == Completion::SuccessfulExit);
    let resource = PathBuf::from(&launch.environment["OPENCODE_DB"]);
    let directory = resource.parent().unwrap();
    assert!(directory.is_dir());
    let step = adapter.accept(json!({"type":"text","part":{"id":"r","text":"title"}}));
    assert!(matches!(&step.events[0], Event::Text {text,..} if text == "title"));
    assert!(!adapter.startup_pending());
    assert!(adapter.cancel_message().is_none());
    drop(adapter);
    assert!(!directory.exists());
}

#[test]
fn registered_helper_cannot_grant_permissions_after_full_access_selection() {
    let _locale = crate::test_support::chinese();
    let approval = Approval {
        id: "permission".into(),
        title: String::new(),
        detail: String::new(),
        wire_id: json!(1),
        input: json!({}),
        allow_once: true,
        kind: ApprovalKind::Command,
    };
    for provider in Provider::ALL {
        let mut request = request();
        request.text_only = true;
        request.permission = PermissionLevel::FullAccess;
        let mut adapter = (provider.descriptor().adapter)(request);
        adapter.enable_full_access();
        assert_eq!(adapter.automatic_approval(&approval), Some(false));
    }
}

#[test]
fn opencode_side_channel_filters_sessions_routes_answers_and_revokes_questions() {
    let _locale = crate::test_support::chinese();
    use base64::Engine as _;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    };
    let mut adapter = (Provider::OpenCode.descriptor().adapter)(request());
    let launch = adapter.launch().unwrap();
    let port: u16 = launch.args.windows(2).find(|p| p[0] == "--port").unwrap()[1]
        .parse()
        .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let authorization = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(
            "conduit:{}",
            launch.environment["OPENCODE_SERVER_PASSWORD"]
        ))
    );
    let worker = std::thread::spawn(move || {
        let pending = json!([
            {"id":"question/one","sessionID":"fixture","questions":[{"header":"Choice","question":"Pick one","options":[{"label":"A","description":"first"}]}]},
            {"id":"question-two","sessionID":"fixture","questions":[{"header":"Name","question":"Name?","options":[]}]},
            {"id":"unrelated","sessionID":"another-session","questions":[]}
        ]);
        let mut captured = vec![];
        let deadline = Instant::now() + Duration::from_secs(10);
        for response in [pending, json!({}), json!({}), json!([])] {
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(value) => break value,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("fixture connection: {e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut header = vec![];
            while !header.ends_with(b"\r\n\r\n") {
                assert!(header.len() < 32768);
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.lines().any(|line| {
                line.split_once(':').is_some_and(|(key, value)| {
                    key.eq_ignore_ascii_case("authorization") && value.trim() == authorization
                })
            }));
            let len = header
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; len];
            socket.read_exact(&mut body).unwrap();
            captured.push((header.lines().next().unwrap().to_owned(), body));
            let body = response.to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
        captured
    });
    adapter.accept(json!({"id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"OpenCode"},"agentCapabilities":{}}}));
    let config: Value =
        serde_json::from_str(&launch.environment["OPENCODE_CONFIG_CONTENT"]).unwrap();
    let agent = config["agent"].as_object().unwrap().keys().next().unwrap();
    let options = json!([
        {"id":"mode","options":[{"value":agent}]},
        {"id":"model","options":[{"value":"test/model"}]}
    ]);
    let mut step =
        adapter.accept(json!({"id":2,"result":{"sessionId":"fixture","configOptions":options}}));
    while step
        .writes
        .first()
        .is_some_and(|v| v["method"] == "session/set_config_option")
    {
        step =
            adapter.accept(json!({"id":step.writes[0]["id"],"result":{"configOptions":options}}));
    }
    assert_eq!(step.writes[0]["method"], "session/prompt");
    let questions = adapter.poll().unwrap();
    assert_eq!(questions.len(), 2);
    for (index, value) in questions.into_iter().enumerate() {
        let step = adapter.accept(value);
        let Event::Approval(approval) = &step.events[0] else {
            panic!("missing question");
        };
        assert_eq!(adapter.automatic_approval(approval), None);
        let response = if index == 0 {
            adapter
                .user_input_response(approval, &BTreeMap::from([("0".into(), vec!["A".into()])]))
                .unwrap()
        } else {
            adapter.approval_response(approval, false)
        };
        assert!(
            adapter
                .route_response(approval, response)
                .unwrap()
                .is_none()
        );
    }
    let revoked = adapter.poll().unwrap();
    assert_eq!(revoked.len(), 2);
    assert!(
        revoked
            .into_iter()
            .all(|v| matches!(&adapter.accept(v).events[0], Event::ApprovalCancelled(_)))
    );
    let requests = worker.join().unwrap();
    assert!(requests[0].0.starts_with("GET /question?directory="));
    assert!(
        requests[1]
            .0
            .starts_with("POST /question/question%2Fone/reply?directory=")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[1].1).unwrap(),
        json!({"answers":[["A"]]})
    );
    assert!(
        requests[2]
            .0
            .starts_with("POST /question/question-two/reject?directory=")
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[2].1).unwrap(),
        json!({"reject":true})
    );
}
