//! Opt-in native Claude child lifecycle/history checks against a local Anthropic fixture.
//! Configuration, history, credentials and tools are isolated from the user's account.
use super::{local_http::read_request, *};
use crate::{cli_adapter::child_history::Snapshot, domain::ItemKind, subagents::Status};
use serde_json::json;
use std::{net::TcpListener, path::Path, sync::Mutex};

const MODEL: &str = "claude-sonnet-4-6";
const PARENT: &str = "CONDUIT_CLAUDE_PARENT";
const CHILD: &str = "CONDUIT_CLAUDE_CHILD";
const PARENT_DONE: &str = "CONDUIT_PARENT_DONE";
const CHILD_DONE: &str = "CONDUIT_CHILD_DONE";
const FILE_TEXT: &str = "CONDUIT_CHILD_FILE_OK";

struct MockModel {
    address: String,
    stopped: Arc<AtomicBool>,
    release_child: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MockModel {
    fn start(file: PathBuf, background: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let release_child = Arc::new(AtomicBool::new(!background));
        let release = release_child.clone();
        let requests = Arc::new(Mutex::new(vec![]));
        let log = requests.clone();
        let worker = thread::spawn(move || {
            let mut handlers = vec![];
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let (file, stop, release, log) =
                            (file.clone(), stop.clone(), release.clone(), log.clone());
                        handlers.push(thread::spawn(move || {
                            let Some(body) = read_request(&mut socket) else { return };
                            let sequence = {
                                let mut requests = log.lock().unwrap();
                                requests.push(body.clone());
                                requests.len()
                            };
                            if sequence > 24 {
                                let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                                return;
                            }
                            let Some(messages) = body["messages"].as_array() else { return };
                            let latest = messages.iter().rev().find(|m| m["role"] == "user")
                                .map(|m| &m["content"]).unwrap();
                            let result = latest.as_array().and_then(|blocks| blocks.iter()
                                .find(|block| block["type"] == "tool_result"));
                            let block = if result.is_some_and(|v| v["tool_use_id"] == "toolu_read") {
                                let deadline = Instant::now() + Duration::from_secs(30);
                                while !release.load(Ordering::Acquire) && !stop.load(Ordering::Acquire)
                                    && Instant::now() < deadline {
                                    thread::sleep(Duration::from_millis(10));
                                }
                                if stop.load(Ordering::Acquire) { return }
                                assert!(release.load(Ordering::Acquire), "parent response never released child");
                                json!({"type":"text","text":CHILD_DONE})
                            } else if result.is_some() {
                                json!({"type":"text","text":PARENT_DONE})
                            } else if latest.to_string().contains(PARENT) {
                                let name = body["tools"].as_array().unwrap().iter()
                                    .find_map(|v| v["name"].as_str().filter(|n| matches!(*n, "Agent" | "Task")))
                                    .expect("native CLI must advertise its Agent tool");
                                json!({"type":"tool_use","id":"toolu_child","name":name,"input":{
                                    "description":"Inspect fixture", "prompt":format!("{CHILD} Read fixture.txt then report the result."),
                                    "subagent_type":"general-purpose","run_in_background":background
                                }})
                            } else if latest.to_string().contains(CHILD) {
                                json!({"type":"tool_use","id":"toolu_read","name":"Read","input":{"file_path":file}})
                            } else {
                                // A background task notification may initiate a final parent reply.
                                json!({"type":"text","text":PARENT_DONE})
                            };
                            let data = response(block, sequence);
                            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len());
                            let _ = socket.write_all(header.as_bytes());
                            let _ = socket.write_all(data.as_bytes());
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("local model: {error}"),
                }
            }
            for handler in handlers {
                handler.join().unwrap();
            }
        });
        Self {
            address,
            stopped,
            release_child,
            requests,
            worker: Some(worker),
        }
    }
}

impl Drop for MockModel {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

fn response(block: Value, sequence: usize) -> String {
    let tool = block["type"] == "tool_use";
    let stop = if tool { "tool_use" } else { "end_turn" };
    let mut initial = block.clone();
    let delta = if tool {
        initial["input"] = json!({});
        json!({"type":"input_json_delta","partial_json":block["input"].to_string()})
    } else {
        initial["text"] = json!("");
        json!({"type":"text_delta","text":block["text"]})
    };
    [
        json!({"type":"message_start","message":{"id":format!("msg_{sequence}"),"type":"message","role":"assistant","model":MODEL,"content":[],"stop_reason":null,"usage":{"input_tokens":100,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":initial}),
        json!({"type":"content_block_delta","index":0,"delta":delta}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null},"usage":{"output_tokens":10}}),
        json!({"type":"message_stop"}),
    ].iter().map(|v| format!("event: {}\ndata: {v}\n\n", v["type"].as_str().unwrap())).collect()
}

fn route(root: &Path, server: &MockModel) -> crate::providers::RuntimeRoute {
    let mut route = crate::providers::RuntimeRoute::default();
    // Reset inherited provider endpoints/credentials without changing the test process.
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if name.starts_with("ANTHROPIC_") || name.starts_with("CLAUDE_") {
            route.env.insert(name.into_owned(), String::new());
        }
    }
    for (key, value) in [
        (
            "CLAUDE_CONFIG_DIR",
            root.join("config").to_string_lossy().into_owned(),
        ),
        ("ANTHROPIC_BASE_URL", server.address.clone()),
        ("ANTHROPIC_API_KEY", "conduit-local-unused".into()),
        ("ANTHROPIC_MODEL", MODEL.into()),
        ("CLAUDE_CODE_SUBAGENT_MODEL", MODEL.into()),
        ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1".into()),
        ("DISABLE_AUTOUPDATER", "1".into()),
        ("DISABLE_TELEMETRY", "1".into()),
        ("DISABLE_ERROR_REPORTING", "1".into()),
    ] {
        route.env.insert(key.into(), value);
    }
    route
}

fn run_case(background: bool) {
    let executable = std::env::var_os("CONDUIT_CLAUDE_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            crate::cli::resolve(Provider::Claude, &crate::cli::CliSettings::default())
                .expect("install Claude before running this opt-in test")
                .path
        });
    let root = std::env::temp_dir().join(format!(
        "conduit-claude-subagents-{}-{}-{background}",
        std::process::id(),
        crate::subagents::now_ms()
    ));
    let cwd = root.join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(cwd.join("fixture.txt"), FILE_TEXT).unwrap();
    eprintln!("Claude native child fixture: {}", root.display());
    let server = MockModel::start(cwd.join("fixture.txt"), background);
    let route = route(&root, &server);
    let source = crate::history::children::source(Provider::Claude, &route.env, &cwd).unwrap();
    let protocol = Protocol::new(
        Provider::Claude,
        cwd.to_string_lossy().into_owned(),
        format!("{PARENT} Delegate the fixture inspection."),
        None,
    )
    .with_model(Some(MODEL.into()))
    .with_permission(PermissionLevel::WorkspaceWrite)
    .with_options(Default::default(), route);
    let mut args = protocol.args();
    args.extend(
        [
            "--setting-sources",
            "",
            "--tools",
            "Agent,Read",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--settings",
            "{\"disableAllHooks\":true}",
        ]
        .into_iter()
        .map(String::from),
    );
    let runtime = ChatRuntime::start_command(protocol, executable, args, cwd.clone());
    let deadline = Instant::now() + Duration::from_secs(65);
    let mut agents = vec![];
    let mut parent_native = None;
    let mut main_text = String::new();
    let mut parent_replied = None;
    let mut active_child_seen = false;
    let mut trace = File::create(root.join("runtime.log")).unwrap();
    loop {
        assert!(
            Instant::now() < deadline,
            "native child lifecycle timeout: {agents:?}"
        );
        if background
            && parent_replied.is_some_and(|at: Instant| at.elapsed() > Duration::from_millis(500))
        {
            server.release_child.store(true, Ordering::Release);
        }
        match runtime.events.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => {
                writeln!(trace, "{event:?}").unwrap();
                match event {
                    Event::NativeId(id) => parent_native = Some(id),
                    Event::Subagent(agent) => {
                        active_child_seen |= agent.status.active();
                        if background && agent.status == Status::Completed {
                            assert!(
                                parent_replied.is_some(),
                                "child completed before parent reply"
                            );
                            assert!(
                                server.release_child.load(Ordering::Acquire),
                                "child completed before model result"
                            );
                        }
                        agents.push(agent);
                    }
                    Event::Approval(approval) => {
                        runtime.decide(approval.id, true);
                    }
                    Event::Text { text, .. } | Event::Delta { text, .. } => {
                        main_text.push_str(&text);
                        if main_text.contains(PARENT_DONE) && parent_replied.is_none() {
                            if background {
                                assert!(
                                    active_child_seen,
                                    "parent replied before child was observed"
                                );
                                assert!(
                                    !server.release_child.load(Ordering::Acquire),
                                    "child response was released before the parent reply"
                                );
                            }
                            parent_replied = Some(Instant::now());
                        }
                    }
                    Event::Finished { error, cancelled } => {
                        assert!(error.is_none() && !cancelled, "{error:?}");
                        assert!(
                            !background || server.release_child.load(Ordering::Acquire),
                            "runtime finished while background child was waiting for its result"
                        );
                        break;
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("runtime disconnected: {error}"),
        }
    }
    drop(runtime);
    drop(trace);
    assert!(parent_replied.is_some(), "missing main reply: {main_text}");
    assert!(
        !main_text.contains(CHILD_DONE) && !main_text.contains(FILE_TEXT),
        "child leaked into main: {main_text}"
    );
    assert!(
        agents.iter().any(|a| a.status.active()),
        "missing live child: {agents:?}"
    );
    let child = agents
        .iter()
        .rev()
        .find(|a| a.status == Status::Completed && a.conversation_id.is_some())
        .expect("completed child with verified native identity");
    let identities: std::collections::BTreeSet<_> = agents.iter().map(|a| &a.id).collect();
    assert_eq!(identities.len(), 1, "duplicate child records: {agents:?}");
    assert_eq!(
        child.direct_input, None,
        "no independent control capability was advertised"
    );
    let native = child.conversation_id.clone().unwrap();
    let parent_native = parent_native.expect("root session identity");
    assert_ne!(native, parent_native);
    assert_eq!(child.parent_id, parent_native);
    assert!(
        child.task.contains(CHILD),
        "missing original child prompt: {child:?}"
    );
    let binding = crate::subagents::ChildBinding {
        root: 1,
        root_native: parent_native.clone(),
        native: native.clone(),
        parent: parent_native,
        source,
        idle: false,
    };
    let before = native_files(&root.join("config"));
    let (tx, rx) = mpsc::channel();
    let reader = ChatRuntime::read_native_child_history(Provider::Claude, cwd, binding, tx);
    let snapshot = rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|error| {
            panic!(
                "native read failed: {error}; runtime: {:?}",
                reader.events.recv_timeout(Duration::from_secs(1))
            )
        });
    let Event::Finished { error, cancelled } =
        reader.events.recv_timeout(Duration::from_secs(2)).unwrap()
    else {
        panic!("expected read completion")
    };
    assert!(error.is_none() && !cancelled, "{error:?}");
    drop(reader);
    let Snapshot::Native { messages, idle, .. } = snapshot else {
        panic!("expected native history")
    };
    assert!(
        !idle,
        "Claude child is read-only, never independently resumable"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.kind == ItemKind::User && m.text.contains(CHILD)),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.kind == ItemKind::Tool
            && m.input.contains("fixture.txt")
            && m.detail.contains(FILE_TEXT)),
        "missing Read input/output: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.kind == ItemKind::Assistant && m.text == CHILD_DONE),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .all(|m| !m.text.contains(PARENT) && !m.text.contains(PARENT_DONE)),
        "{messages:?}"
    );
    assert_eq!(
        before,
        native_files(&root.join("config")),
        "history read changed native logs"
    );
    let requests = server.requests.lock().unwrap();
    assert!(
        (4..=12).contains(&requests.len()),
        "unexpected model requests: {}",
        requests.len()
    );
    std::fs::write(
        root.join("requests.json"),
        serde_json::to_vec_pretty(&*requests).unwrap(),
    )
    .unwrap();
    eprintln!(
        "Claude {} verified: child {native}, {} transcript items, {} local requests",
        if background {
            "background"
        } else {
            "foreground"
        },
        messages.len(),
        requests.len()
    );
    drop(requests);
    drop(server);
    std::fs::remove_dir_all(root).unwrap();
}

fn native_files(path: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(native_files(&path));
        } else if path
            .extension()
            .is_some_and(|ext| ext == "jsonl" || ext == "json")
        {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}

#[test]
#[ignore = "runs installed Claude against an isolated local model; no model account"]
fn installed_claude_foreground_subagent_history() {
    run_case(false);
}

#[test]
#[ignore = "runs installed Claude against an isolated local model; no model account"]
fn installed_claude_background_subagent_history() {
    run_case(true);
}
