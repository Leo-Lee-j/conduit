//! Opt-in real-binary checks using a local synthetic OpenAI-compatible endpoint.
//! All provider configuration and history are isolated; no account is contacted.
use super::local_http::read_request;
use super::*;
use crate::{cli, domain::ItemKind};
use serde_json::json;
use std::net::TcpListener;

struct MockModel {
    address: String,
    stopped: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    requests: Arc<std::sync::Mutex<Vec<Value>>>,
}

impl MockModel {
    fn start(file: PathBuf) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let requests = Arc::new(std::sync::Mutex::new(vec![]));
        let log = requests.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let file = file.clone();
                        let log = log.clone();
                        thread::spawn(move || {
                            let Some(body) = read_request(&mut socket) else {
                                return;
                            };
                            log.lock().unwrap().push(body.clone());
                            let messages = body["messages"].as_array().unwrap();
                            let latest = messages
                                .iter()
                                .rev()
                                .find(|v| v["role"] == "user")
                                .map(Value::to_string)
                                .unwrap_or_default();
                            if latest.contains("CONDUIT_API_FAILURE") {
                                let body = json!({"error":{"message":"Free usage limit reached","type":"quota_exceeded"}}).to_string();
                                let response = format!(
                                    "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                    body.len()
                                );
                                let _ = socket.write_all(response.as_bytes());
                                return;
                            }
                            let tool_result = messages.last().is_some_and(|v| v["role"] == "tool");
                            let has_write = body["tools"].as_array().is_some_and(|tools| {
                                tools.iter().any(|v| v["function"]["name"] == "write")
                            });
                            let has_question = body["tools"].as_array().is_some_and(|tools| {
                                tools.iter().any(|v| v["function"]["name"] == "question")
                            });
                            let task = body["tools"].as_array().and_then(|tools| {
                                tools.iter().find(|tool| tool["function"]["name"] == "task")
                            });
                            let delta = if latest.contains("CONDUIT_SUBAGENT_PARENT")
                                && !tool_result
                                && let Some(task) = task
                            {
                                let description = task["function"]["description"].as_str().unwrap();
                                let start = description
                                    .find("conduit-")
                                    .expect("managed worker advertised");
                                let worker: String = description[start..]
                                    .chars()
                                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                                    .collect();
                                json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_conduit_task","type":"function","function":{"name":"task","arguments":json!({"description":"Review local fixture","prompt":"CONDUIT_SUBAGENT_CHILD CONDUIT_HOLD","subagent_type":worker}).to_string()}}]})
                            } else if latest.contains("CONDUIT_QUESTION")
                                && has_question
                                && !tool_result
                            {
                                json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_conduit_question","type":"function","function":{"name":"question","arguments":json!({"questions":[
                                    {"header":"方案","question":"选择方案？","options":[{"label":"方案 A","description":"第一个方案"},{"label":"方案 B","description":"第二个方案"}]},
                                    {"header":"名称","question":"项目叫什么？","options":[{"label":"默认名称","description":"使用默认名称"}]}
                                ]}).to_string()}}]})
                            } else if latest.contains("CONDUIT_TOOL") && has_write && !tool_result {
                                json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_conduit_write","type":"function","function":{"name":"write","arguments":json!({"filePath":file,"content":"APPROVED_WRITE"}).to_string()}}]})
                            } else {
                                let text = if latest.contains("CONDUIT_CHILD_RESUME") {
                                    if messages.iter().any(|v| {
                                        v["role"] == "user"
                                            && v.to_string().contains("CONDUIT_SUBAGENT_CHILD")
                                    }) && !messages.iter().any(|v| {
                                        v["role"] == "user"
                                            && v.to_string().contains("CONDUIT_SUBAGENT_PARENT")
                                    }) {
                                        "CHILD_RESUME_OK"
                                    } else {
                                        "CHILD_RESUME_WRONG_HISTORY"
                                    }
                                } else if latest.contains("CONDUIT_RESUME") {
                                    if messages.iter().any(|v| {
                                        v["role"] == "user"
                                            && v.to_string().contains("CONDUIT_REMEMBER_ORANGE")
                                    }) {
                                        "RESUME_OK"
                                    } else {
                                        "RESUME_MISSING_HISTORY"
                                    }
                                } else {
                                    "CONDUIT_LOCAL_OK"
                                };
                                json!({"role":"assistant","content":text})
                            };
                            let reason = if delta.get("tool_calls").is_some() {
                                "tool_calls"
                            } else {
                                "stop"
                            };
                            let chunk = |delta: Value, finish: Value| {
                                json!({"id":"chatcmpl-conduit-local","object":"chat.completion.chunk","created":1,"model":"mock","choices":[{"index":0,"delta":delta,"finish_reason":finish}]}).to_string()
                            };
                            let stream = format!(
                                "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                                chunk(delta, Value::Null),
                                chunk(json!({}), json!(reason))
                            );
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                stream.len()
                            );
                            let _ = socket.write_all(header.as_bytes());
                            if latest.contains("CONDUIT_HOLD") {
                                let boundary = stream.find("\n\n").unwrap() + 2;
                                let _ = socket.write_all(&stream.as_bytes()[..boundary]);
                                thread::sleep(Duration::from_secs(2));
                                let _ = socket.write_all(&stream.as_bytes()[boundary..]);
                            } else {
                                let _ = socket.write_all(stream.as_bytes());
                            }
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("mock accept: {error}"),
                }
            }
        });
        Self {
            address,
            stopped,
            worker: Some(worker),
            requests,
        }
    }
}

#[test]
#[ignore = "runs installed OpenCode against a local mock; no model account is used"]
fn installed_opencode_user_input_answer_skip_and_cancel() {
    let _locale = crate::test_support::chinese();
    let root =
        std::env::temp_dir().join(format!("conduit-opencode-questions-{}", std::process::id()));
    assert!(!root.exists());
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let server = MockModel::start(root.join("unused.txt"));
    for (permission, action) in [
        (PermissionLevel::FullAccess, 0),
        (PermissionLevel::ReadOnly, 1),
        (PermissionLevel::WorkspaceWrite, 2),
    ] {
        let runtime = run(&root, &server, "CONDUIT_QUESTION", None, permission);
        let deadline = Instant::now() + Duration::from_secs(40);
        let mut seen = false;
        let mut resolved = false;
        loop {
            assert!(Instant::now() < deadline, "OpenCode question timed out");
            match runtime.events.recv_timeout(Duration::from_secs(2)) {
                Ok(Event::Approval(approval)) => {
                    assert!(!seen, "question must be queued once");
                    seen = true;
                    assert!(approval.is_user_input());
                    assert_eq!(approval.questions().len(), 2);
                    runtime.enable_full_access();
                    match action {
                        0 => runtime.answer(
                            approval.id,
                            std::collections::BTreeMap::from([
                                ("0".into(), vec!["方案 B".into()]),
                                ("1".into(), vec!["中文项目".into()]),
                            ]),
                        ),
                        1 => runtime.decide(approval.id, false),
                        _ => runtime.cancel(),
                    }
                }
                Ok(Event::ApprovalResolved { allow, .. }) => {
                    resolved = true;
                    assert_eq!(allow, action == 0);
                }
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(error.is_none(), "{error:?}");
                    assert_eq!(cancelled, action == 2);
                    assert!(seen, "the CLI must expose its question tool");
                    assert_eq!(resolved, action != 2);
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("{error}"),
            }
        }
    }
    let requests = server.requests.lock().unwrap();
    assert!(
        requests.iter().any(|r| r["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool"
                && m.to_string().contains("方案 B")
                && m.to_string().contains("中文项目"))),
        "answers must reach the actual CLI tool result"
    );
    drop(requests);
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}

impl Drop for MockModel {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn route(root: &std::path::Path, server: &MockModel) -> crate::providers::RuntimeRoute {
    let mut route = crate::providers::RuntimeRoute::default();
    for (name, subdirectory) in [
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
        ("XDG_STATE_HOME", "state"),
    ] {
        let path = root.join(subdirectory);
        std::fs::create_dir_all(&path).unwrap();
        route
            .env
            .insert(name.into(), path.to_string_lossy().into_owned());
    }
    route.env.insert(
        "OPENCODE_DB".into(),
        root.join("isolated.sqlite").to_string_lossy().into_owned(),
    );
    route
        .env
        .insert("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "1".into());
    route
        .env
        .insert("OPENCODE_DISABLE_MODELS_FETCH".into(), "1".into());
    route.env.insert("OPENCODE_CONFIG_CONTENT".into(),json!({
        "model":"conduit-test/mock","small_model":"conduit-test/mock",
        "permission":{"*":"allow","edit":"allow","bash":"allow"},
        "provider":{"conduit-test":{"npm":"@ai-sdk/openai-compatible","name":"Local test","options":{"baseURL":server.address,"apiKey":"local-test-unused"},"models":{"mock":{"name":"Mock","tool_call":true,"limit":{"context":8192,"output":1024},"variants":{"high":{},"max":{}}}}}}
    }).to_string());
    route
}

fn run(
    root: &std::path::Path,
    server: &MockModel,
    prompt: &str,
    native: Option<String>,
    permission: PermissionLevel,
) -> ChatRuntime {
    let executable = crate::cli::resolve(Provider::OpenCode, &crate::cli::CliSettings::default())
        .expect("install OpenCode before running this opt-in test")
        .path;
    ChatRuntime::start_request(super::StartRequest {
        intent: Default::default(),
        provider: Provider::OpenCode,
        executable,
        cwd: root.join("workspace"),
        prompt: prompt.into(),
        native_id: native,
        model: Some("conduit-test/mock".into()),
        permission,
        effort: Default::default(),
        route: route(root, server),
    })
}

#[derive(Clone, Copy)]
enum Decision {
    Allow,
    Reject,
    Cancel,
}

#[test]
#[ignore = "runs installed OpenCode against an isolated local mock; no model account is used"]
fn installed_opencode_commands_discover_resume_and_execute_locally() {
    use crate::commands::{Catalog, CommandContext, RequestIntent};
    let _locale = crate::test_support::chinese();
    let root =
        std::env::temp_dir().join(format!("conduit-opencode-commands-{}", std::process::id()));
    assert!(!root.exists());
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let server = MockModel::start(root.join("unused.txt"));
    let mut configured = route(&root, &server);
    let mut config: Value =
        serde_json::from_str(&configured.env["OPENCODE_CONFIG_CONTENT"]).unwrap();
    config["command"] = json!({"conduit-test-review":{"template":"CONDUIT_REMEMBER_ORANGE $ARGUMENTS", "description":"Offline command fixture"}});
    configured
        .env
        .insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
    let launch = |intent, text: String, native, route| {
        ChatRuntime::start_request(super::StartRequest {
            intent,
            provider: Provider::OpenCode,
            executable: cli::resolve(Provider::OpenCode, &Default::default())
                .unwrap()
                .path,
            cwd: root.join("workspace"),
            prompt: text,
            native_id: native,
            model: Some("conduit-test/mock".into()),
            permission: PermissionLevel::WorkspaceWrite,
            effort: Default::default(),
            route,
        })
    };
    let discovery = launch(
        RequestIntent::Discover,
        "never sent".into(),
        None,
        configured.clone(),
    );
    let mut native = None;
    let mut items = None;
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        assert!(Instant::now() < deadline);
        match discovery.events.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::NativeId(id)) => native = Some(id),
            Ok(Event::CommandCatalog(result)) => items = Some(result.unwrap()),
            Ok(Event::Finished { error, cancelled }) => {
                assert!(error.is_none() && !cancelled, "{error:?}");
                break;
            }
            Ok(Event::Approval(_)) => panic!("discovery must not request tool access"),
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("missing completion"),
            _ => {}
        }
    }
    assert!(
        server.requests.lock().unwrap().is_empty(),
        "discovery submitted a model prompt"
    );
    let native = native.unwrap();
    let items = items.unwrap();
    let command = items
        .iter()
        .find(|item| item.name == "conduit-test-review")
        .unwrap()
        .clone();
    let context = CommandContext {
        workspace: root.join("workspace"),
        session: Some(1),
        native_id: Some(native.clone()),
        source: "opencode".into(),
        registration: 1,
        configuration: 1,
    };
    let mut catalog = Catalog::default();
    let ticket = catalog.begin(context.clone());
    catalog.publish(&ticket, Ok(items));
    let text = "/conduit-test-review 中文 arguments";
    let call = catalog.prepare(&context, &command, text.into()).unwrap();
    let invocation = launch(
        RequestIntent::Command(Box::new(call)),
        text.into(),
        Some(native.clone()),
        configured.clone(),
    );
    let (same_native, output, approvals) = finish(&invocation, Decision::Reject);
    assert_eq!(same_native, native);
    assert!(output.contains("CONDUIT_LOCAL_OK"));
    assert_eq!(approvals, 0);
    let requests = server.requests.lock().unwrap().len();
    assert!(requests > 0);
    // Removing a command after discovery must not send its text to the model.
    config["command"] = json!({});
    configured
        .env
        .insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
    let call = catalog.prepare(&context, &command, text.into()).unwrap();
    let removed = launch(
        RequestIntent::Command(Box::new(call)),
        text.into(),
        Some(native),
        configured,
    );
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        assert!(Instant::now() < deadline);
        match removed.events.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::Finished { error, cancelled }) => {
                assert!(error.is_some() && !cancelled);
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("missing completion"),
            _ => {}
        }
    }
    assert_eq!(server.requests.lock().unwrap().len(), requests);
    std::fs::remove_dir_all(&root).unwrap();
}

fn finish(runtime: &ChatRuntime, decision: Decision) -> (String, String, usize) {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut native = None;
    let mut output = String::new();
    let mut approvals = 0;
    loop {
        assert!(
            Instant::now() < deadline,
            "local OpenCode test timed out: {output}"
        );
        match runtime.events.recv_timeout(Duration::from_secs(2)) {
            Ok(Event::NativeId(id)) => native = Some(id),
            Ok(Event::Delta { text, .. }) => output.push_str(&text),
            Ok(Event::Text { text, .. }) => output.push_str(&text),
            Ok(Event::Approval(approval)) => {
                approvals += 1;
                match decision {
                    Decision::Allow => runtime.decide(approval.id, true),
                    Decision::Reject => runtime.decide(approval.id, false),
                    Decision::Cancel => runtime.cancel(),
                }
            }
            Ok(Event::Finished { error, cancelled }) => {
                assert!(error.is_none(), "{error:?}; output={output}");
                assert_eq!(cancelled, matches!(decision, Decision::Cancel));
                return (native.unwrap_or_default(), output, approvals);
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
#[ignore = "runs installed OpenCode against a local mock; no model account is used"]
fn installed_opencode_local_protocol_smoke() {
    let _locale = crate::test_support::chinese();
    let root = std::env::temp_dir().join(format!("conduit-opencode-local-{}", std::process::id()));
    assert!(
        !root.exists(),
        "remove previous failed test workspace first: {}",
        root.display()
    );
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let file = root.join("workspace/approved.txt");
    let server = MockModel::start(file.clone());

    let first = run(
        &root,
        &server,
        "CONDUIT_REMEMBER_ORANGE",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    let (id, output, _) = finish(&first, Decision::Reject);
    assert!(output.contains("CONDUIT_LOCAL_OK"), "{output}");
    let resumed = run(
        &root,
        &server,
        "CONDUIT_RESUME",
        Some(id.clone()),
        PermissionLevel::WorkspaceWrite,
    );
    let (resumed_id, output, _) = finish(&resumed, Decision::Reject);
    assert_eq!(resumed_id, id);
    assert!(output.contains("RESUME_OK"), "{output}");

    let denied = run(
        &root,
        &server,
        "CONDUIT_TOOL_REJECT",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    assert_eq!(finish(&denied, Decision::Reject).2, 1);
    assert!(!file.exists());
    let cancelled = run(
        &root,
        &server,
        "CONDUIT_TOOL_CANCEL",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    assert_eq!(finish(&cancelled, Decision::Cancel).2, 1);
    assert!(!file.exists());
    let allowed = run(
        &root,
        &server,
        "CONDUIT_TOOL_ALLOW",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    assert_eq!(finish(&allowed, Decision::Allow).2, 1);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "APPROVED_WRITE");
    std::fs::remove_file(&file).unwrap();

    let read_only = run(
        &root,
        &server,
        "CONDUIT_TOOL_READONLY",
        None,
        PermissionLevel::ReadOnly,
    );
    assert_eq!(finish(&read_only, Decision::Reject).2, 0);
    assert!(!file.exists());
    let requests = server.requests.lock().unwrap();
    let readonly = requests
        .iter()
        .find(|v| {
            v["messages"].to_string().contains("CONDUIT_TOOL_READONLY") && v["tools"].is_array()
        })
        .expect("readonly model request missing");
    assert!(
        !readonly["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| matches!(
                v["function"]["name"].as_str(),
                Some("write" | "edit" | "bash")
            ))
    );
    drop(requests);
    let full = run(
        &root,
        &server,
        "CONDUIT_TOOL_FULL",
        None,
        PermissionLevel::FullAccess,
    );
    assert_eq!(finish(&full, Decision::Reject).2, 0);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "APPROVED_WRITE");
    let first = run(
        &root,
        &server,
        "CONDUIT_HOLD_FIRST",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    let second = run(
        &root,
        &server,
        "CONDUIT_HOLD_SECOND",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    loop {
        match first.events.recv_timeout(Duration::from_secs(15)).unwrap() {
            Event::Delta { .. } => break,
            Event::Finished { error, .. } => {
                panic!("first parallel turn ended before cancellation: {error:?}")
            }
            _ => {}
        }
    }
    first.cancel();
    loop {
        if let Event::Finished { error, cancelled } =
            first.events.recv_timeout(Duration::from_secs(5)).unwrap()
        {
            assert!(cancelled);
            assert!(error.is_none(), "{error:?}");
            break;
        }
    }
    assert!(
        finish(&second, Decision::Reject)
            .1
            .contains("CONDUIT_LOCAL_OK")
    );
    let helper_executable =
        crate::cli::resolve(Provider::OpenCode, &crate::cli::CliSettings::default())
            .unwrap()
            .path;
    let helper = ChatRuntime::start_text_with_route(
        Provider::OpenCode,
        helper_executable,
        "CONDUIT_TOOL_HELPER".into(),
        Some("conduit-test/mock".into()),
        route(&root, &server),
    );
    let (_, output, approvals) = finish(&helper, Decision::Reject);
    assert_eq!(approvals, 0);
    assert!(output.contains("CONDUIT_LOCAL_OK"), "{output}");
    let requests = server.requests.lock().unwrap();
    let helper_request = requests
        .iter()
        .find(|v| v["messages"].to_string().contains("CONDUIT_TOOL_HELPER"))
        .expect("helper request missing");
    assert!(
        helper_request["tools"]
            .as_array()
            .is_none_or(|tools| tools.is_empty())
    );
    drop(requests);
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "runs installed OpenCode against a local mock; no model account is used"]
fn installed_opencode_helper_reports_model_failure_promptly() {
    let _locale = crate::test_support::chinese();
    let root = std::env::temp_dir().join(format!("conduit-opencode-error-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let server = MockModel::start(root.join("unused.txt"));
    let started = Instant::now();
    let runtime = ChatRuntime::start_text_with_route(
        Provider::OpenCode,
        cli::resolve(Provider::OpenCode, &Default::default())
            .unwrap()
            .path,
        "CONDUIT_API_FAILURE".into(),
        Some("conduit-test/mock".into()),
        route(&root, &server),
    );
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        assert!(Instant::now() < deadline, "helper swallowed API error");
        match runtime.events.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::Finished { error, cancelled }) => {
                assert!(!cancelled);
                assert!(
                    error
                        .as_deref()
                        .is_some_and(|e| e.contains("Free usage limit reached")),
                    "{error:?}"
                );
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("{error}"),
        }
    }
    assert!(started.elapsed() < Duration::from_secs(25));
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "runs installed OpenCode against an isolated local mock; no model account"]
fn installed_opencode_subagent_lifecycle() {
    let _locale = crate::test_support::chinese();
    let root = std::env::temp_dir().join(format!(
        "conduit-opencode-subagents-{}-{}",
        std::process::id(),
        crate::subagents::now()
    ));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let server = MockModel::start(root.join("workspace/unused.txt"));
    let runtime = run(
        &root,
        &server,
        "CONDUIT_SUBAGENT_PARENT",
        None,
        PermissionLevel::WorkspaceWrite,
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut agents = vec![];
    let mut parent_native = None;
    let mut approved = 0;
    loop {
        assert!(
            Instant::now() < deadline,
            "child lifecycle timeout: {agents:?}"
        );
        match runtime.events.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::NativeId(id)) => parent_native = Some(id),
            Ok(Event::Subagent(agent)) => agents.push(agent),
            Ok(Event::Approval(approval)) => {
                approved += 1;
                runtime.decide(approval.id, true);
            }
            Ok(Event::Finished { error, cancelled }) => {
                assert!(error.is_none() && !cancelled, "{error:?}");
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("{error}"),
        }
    }
    assert!(
        approved > 0,
        "task dispatch must honor workspace-write approval"
    );
    assert!(
        agents.iter().any(|a| a.status.active()),
        "must observe live child: {agents:?}"
    );
    assert!(
        agents
            .iter()
            .any(|a| a.status == crate::subagents::Status::Completed
                && a.result.contains("CONDUIT_LOCAL_OK")),
        "must observe confirmed result: {agents:?}"
    );
    // Finished is the runtime cleanup barrier; release the parent before loading
    // the native child history and creating an independent ACP connection.
    drop(runtime);
    let parent_native = parent_native.expect("parent native identity");
    let child = agents
        .iter()
        .rev()
        .find(|a| a.status == crate::subagents::Status::Completed)
        .unwrap();
    assert_ne!(child.id, parent_native);
    let source = crate::history::children::source(
        Provider::OpenCode,
        &route(&root, &server).env,
        &root.join("workspace"),
    )
    .unwrap();
    let binding = crate::subagents::ChildBinding {
        root: 1,
        root_native: parent_native.clone(),
        native: child.id.clone(),
        parent: parent_native.clone(),
        source: source.clone(),
        idle: false,
    };
    let read_child = || {
        let crate::cli_adapter::child_history::Snapshot::Native { messages, idle, .. } =
            crate::history::children::read(
                Provider::OpenCode,
                &root.join("workspace"),
                &binding,
                &AtomicBool::new(false),
            )
            .unwrap()
        else {
            panic!("expected native child history")
        };
        assert!(idle, "finished child must be idle");
        messages
    };
    let messages = read_child();
    assert!(messages.iter().any(|item| {
        item.kind == ItemKind::User && item.text.contains("CONDUIT_SUBAGENT_CHILD")
    }));
    assert!(messages.iter().any(|item| item.text == "CONDUIT_LOCAL_OK"));
    let parent_messages = || {
        let database = rusqlite::Connection::open_with_flags(
            &source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        database
            .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY id")
            .unwrap()
            .query_map([&parent_native], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let before = parent_messages();
    let resumed = run(
        &root,
        &server,
        "CONDUIT_CHILD_RESUME",
        Some(child.id.clone()),
        PermissionLevel::WorkspaceWrite,
    );
    let (native, output, _) = finish(&resumed, Decision::Allow);
    drop(resumed);
    assert_eq!(native, child.id);
    assert!(output.contains("CHILD_RESUME_OK"), "{output}");
    assert!(!output.contains("CHILD_RESUME_WRONG_HISTORY"), "{output}");
    assert_eq!(before, parent_messages(), "child resume changed its parent");
    let messages = read_child();
    assert!(
        messages
            .iter()
            .any(|item| item.text == "CONDUIT_CHILD_RESUME")
    );
    assert!(messages.iter().any(|item| item.text == "CHILD_RESUME_OK"));
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}
