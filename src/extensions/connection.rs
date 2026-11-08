//! Explicit MCP handshake diagnostics. Never invokes tools or samples models.
use super::*;
use anyhow::{Result, bail, ensure};
use serde_json::json;
use std::collections::HashSet;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolReport {
    pub names: Vec<String>,
}
impl ToolReport {
    pub fn count(&self) -> usize {
        self.names.len()
    }
    pub fn summary(&self) -> String {
        let names = self
            .names
            .iter()
            .take(12)
            .map(|name| clipped(name, 80))
            .collect::<Vec<_>>();
        conduit::i18n::format(
            "extensions.connection.connected_available_tools_no_tools_were_invoked",
            &[
                ("arg0", format!("{}", self.count())),
                (
                    "arg1",
                    (if names.is_empty() { "" } else { "：" }).to_string(),
                ),
                ("arg2", names.join("、").to_string()),
            ],
        )
    }
}

pub struct ConnectionProbe {
    pub id: String,
    pub events: Receiver<Result<ToolReport, String>>,
    cancelled: Arc<AtomicBool>,
}
impl ConnectionProbe {
    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn test_pending(id: String) -> (Self, mpsc::Sender<Result<ToolReport, String>>) {
        let (sender, events) = mpsc::channel();
        (
            Self {
                id,
                events,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
            sender,
        )
    }

    pub fn start(entry: Entry, context: Context) -> Self {
        let id = entry.id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(check(&entry, &context, &cancel).map_err(|e| e.to_string()));
        });
        Self {
            id,
            events,
            cancelled,
        }
    }
}
impl Drop for ConnectionProbe {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

struct TestProcess(crate::child::Child);
impl Drop for TestProcess {
    fn drop(&mut self) {
        let _ = self.0.terminate();
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2)
            && (self.0.exit_code().ok().flatten().is_none()
                || self.0.active_processes().is_ok_and(|count| count > 0))
        {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn config(entry: &Entry) -> Result<Value> {
    let text = edit::read(&entry.path)?.ok_or_else(|| {
        anyhow::anyhow!(conduit::i18n::text(
            "extensions.connection.configuration_file_no_longer_exists_refresh_to"
        ))
    })?;
    let value: Value = if entry.path.extension().is_some_and(|e| e == "toml") {
        serde_json::to_value(toml::from_str::<toml::Table>(&text).map_err(|_| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.invalid_toml_format"
            ))
        })?)?
    } else {
        jsonc(&text).ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.invalid_json_configuration"
            ))
        })?
    };
    for key in ["mcp_servers", "mcpServers", "mcp"] {
        if let Some(server) = value[key].get(&entry.name).filter(|s| s.is_object()) {
            return Ok(server.clone());
        }
    }
    bail!(conduit::i18n::text(
        "extensions.connection.this_source_requires_native_cli_parsing_test"
    ))
}

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Conduit settings check","version":env!("CARGO_PKG_VERSION")}}})
}
fn initialized() -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/initialized"})
}
fn tools_request(id: i64, cursor: Option<&str>) -> Value {
    let mut params = json!({});
    if let Some(cursor) = cursor {
        params["cursor"] = json!(cursor);
    }
    json!({"jsonrpc":"2.0","id":id,"method":"tools/list","params":params})
}

fn result(value: Value, id: i64) -> Result<Value> {
    ensure!(
        value["id"] == id,
        conduit::i18n::text("extensions.connection.server_returned_a_mismatched_response")
    );
    if !value["error"].is_null() {
        bail!(conduit::i18n::format(
            "extensions.connection.server_returned_an_mcp_error_code_check",
            &[(
                "arg0",
                format!("{}", value["error"]["code"].as_i64().unwrap_or(0))
            )]
        ));
    }
    value.get("result").cloned().ok_or_else(|| {
        anyhow::anyhow!(conduit::i18n::text(
            "extensions.connection.server_returned_no_mcp_result"
        ))
    })
}
fn negotiated(value: &Value) -> Result<&str> {
    let version = value["protocolVersion"].as_str().unwrap_or("");
    ensure!(
        matches!(version, "2025-06-18" | "2025-03-26" | "2024-11-05"),
        conduit::i18n::text(
            "extensions.connection.server_uses_an_unsupported_protocol_version_connect"
        )
    );
    Ok(version)
}
fn collect_tools(mut page: impl FnMut(i64, Option<&str>) -> Result<Value>) -> Result<ToolReport> {
    let mut report = ToolReport::default();
    let mut names = HashSet::new();
    let mut cursors = HashSet::new();
    let mut cursor = None;
    for id in 2..102 {
        let result = page(id, cursor.as_deref())?;
        let tools = result["tools"].as_array().ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.invalid_tool_list"
            ))
        })?;
        for tool in tools {
            let name = tool["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "extensions.connection.invalid_tool_name"
                    ))
                })?;
            if names.insert(name.to_string()) {
                report.names.push(name.to_string());
            }
            ensure!(
                report.count() <= 10_000,
                conduit::i18n::text("extensions.connection.tool_count_exceeds_the_detection_limit")
            );
        }
        match result.get("nextCursor").filter(|value| !value.is_null()) {
            None => return Ok(report),
            Some(value) => {
                let next = value
                    .as_str()
                    .filter(|cursor| !cursor.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(conduit::i18n::text(
                            "extensions.connection.invalid_tool_list_pagination_cursor"
                        ))
                    })?;
                ensure!(
                    cursors.insert(next.to_string()),
                    conduit::i18n::text(
                        "extensions.connection.repeated_pagination_cursor_unable_to_determine_the"
                    )
                );
                cursor = Some(next.to_string());
            }
        }
    }
    bail!(conduit::i18n::text(
        "extensions.connection.tool_pagination_exceeds_the_detection_limit_unable"
    ))
}
fn check(entry: &Entry, context: &Context, cancel: &AtomicBool) -> Result<ToolReport> {
    ensure!(
        !cancel.load(Ordering::Acquire),
        conduit::i18n::text("settings.extensions.test_cancelled")
    );
    ensure!(
        !entry.disabled,
        conduit::i18n::text("extensions.connection.this_definition_is_disabled_enable_it_in")
    );
    let server = config(entry)?;
    ensure!(
        server["enabled"] != false && server["disabled"] != true,
        conduit::i18n::text("extensions.connection.configuration_disabled_refresh_the_list")
    );
    if let Some(url) = server["url"].as_str() {
        http(&server, url, cancel)
    } else {
        stdio(
            &server,
            context
                .workspace
                .as_deref()
                .unwrap_or(&std::env::temp_dir()),
            cancel,
        )
    }
}

fn stdio(server: &Value, cwd: &Path, cancel: &AtomicBool) -> Result<ToolReport> {
    let command = server["command"]
        .as_str()
        .or_else(|| server["command"][0].as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.startup_command_missing"
            ))
        })?;
    let args = if server["command"].is_array() {
        strings(&server["command"])
            .into_iter()
            .skip(1)
            .collect::<Vec<_>>()
    } else {
        strings(&server["args"])
    };
    let path = resolve_command(command, cwd)?;
    let mut env = BTreeMap::new();
    for key in ["env", "environment"] {
        if let Some(values) = server[key].as_object() {
            for (key, value) in values {
                let value = value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "extensions.connection.environment_variable_values_must_be_strings"
                    ))
                })?;
                ensure!(
                    !value.contains("${") && !value.contains("{env:"),
                    conduit::i18n::text(
                        "extensions.connection.dynamic_environment_variables_require_cli_parsing_test"
                    )
                );
                env.insert(key.clone(), value.to_string());
            }
        }
    }
    // Some Windows runtimes (notably .NET) fail before initializing when argv[0]
    // uses the extended-length prefix introduced by canonicalization.
    let (exe, args) = crate::cli::command_for(&crate::child::normalized_dir(&path), &args);
    let (child, mut pipes) =
        crate::child::Child::spawn_with_env(&exe, &args, cwd, &env).map_err(|_| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.unable_to_start_mcp_command_check_its"
            ))
        })?;
    let _process = TestProcess(child);
    let started = Instant::now();
    let mut buffer = Vec::new();
    let mut bytes = 0;
    send(&mut pipes.stdin, &initialize())?;
    let init = result(
        receive(&mut pipes, &mut buffer, &mut bytes, 1, started, cancel)?,
        1,
    )?;
    negotiated(&init)?;
    send(&mut pipes.stdin, &initialized())?;
    if init.pointer("/capabilities/tools").is_none() {
        return Ok(ToolReport::default());
    }
    collect_tools(|id, cursor| {
        send(&mut pipes.stdin, &tools_request(id, cursor))?;
        result(
            receive(&mut pipes, &mut buffer, &mut bytes, id, started, cancel)?,
            id,
        )
    })
}

fn resolve_command(command: &str, cwd: &Path) -> Result<PathBuf> {
    let candidate = Path::new(command);
    let mut candidates = vec![if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        cwd.join(candidate)
    }];
    if candidate.components().count() == 1 {
        for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            candidates.push(directory.join(command));
            if candidate.extension().is_none() {
                for ext in ["exe", "cmd", "bat"] {
                    candidates.push(directory.join(format!("{command}.{ext}")));
                }
            }
        }
    }
    candidates
        .into_iter()
        .find_map(|p| crate::cli::validate_executable(&p).ok())
        .ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.executable_not_found_install_it_or_provide"
            ))
        })
}
fn send(file: &mut fs::File, value: &Value) -> Result<()> {
    writeln!(file, "{value}").map_err(|_| {
        anyhow::anyhow!(conduit::i18n::text(
            "extensions.connection.mcp_process_input_closed"
        ))
    })
}
fn receive(
    pipes: &mut crate::child::Pipes,
    buffer: &mut Vec<u8>,
    bytes: &mut usize,
    id: i64,
    started: Instant,
    cancel: &AtomicBool,
) -> Result<Value> {
    loop {
        ensure!(
            !cancel.load(Ordering::Acquire),
            conduit::i18n::text("settings.extensions.test_cancelled")
        );
        ensure!(
            started.elapsed() < Duration::from_secs(15),
            conduit::i18n::text("extensions.connection.connection_test_timed_out_15_seconds_test")
        );
        while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<_> = buffer.drain(..=end).collect();
            if let Ok(value) = serde_json::from_slice::<Value>(&line)
                && value["id"] == id
            {
                return Ok(value);
            }
        }
        for (index, file) in [&mut pipes.stdout, &mut pipes.stderr]
            .into_iter()
            .enumerate()
        {
            let available = match crate::child::available(file) {
                Ok(count) => count,
                // stderr is not the protocol channel and can close independently.
                Err(_) if index == 1 || !buffer.is_empty() => 0,
                Err(_) => bail!(conduit::i18n::text(
                    "extensions.connection.mcp_process_exited_before_the_connection_test"
                )),
            };
            if available > 0 {
                let mut chunk = [0; 4096];
                let read = file.read(&mut chunk[..available.min(4096) as usize])?;
                *bytes += read;
                ensure!(
                    *bytes <= 2 * 1024 * 1024,
                    conduit::i18n::text(
                        "extensions.connection.mcp_output_exceeded_2_mib_test_stopped"
                    )
                );
                if index == 0 {
                    buffer.extend_from_slice(&chunk[..read]);
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn http(server: &Value, address: &str, cancel: &AtomicBool) -> Result<ToolReport> {
    let url = url::Url::parse(address).map_err(|_| {
        anyhow::anyhow!(conduit::i18n::text("extensions.connection.invalid_mcp_url"))
    })?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none(),
        conduit::i18n::text("extensions.connection.mcp_url_must_use_http_s_and")
    );
    ensure!(
        server["type"] != "sse",
        conduit::i18n::text("extensions.connection.connect_to_legacy_sse_services_in_the")
    );
    let mut headers = BTreeMap::new();
    for key in ["headers", "http_headers"] {
        if let Some(values) = server[key].as_object() {
            for (key, value) in values {
                let value = value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "extensions.connection.invalid_authentication_headers"
                    ))
                })?;
                ensure!(
                    !value.contains("${") && !value.contains("{env:"),
                    conduit::i18n::text(
                        "extensions.connection.dynamic_authentication_headers_require_native_cli_parsing"
                    )
                );
                headers.insert(key.clone(), value.to_string());
            }
        }
    }
    if let Some(values) = server["env_http_headers"].as_object() {
        for (key, name) in values {
            let value = name
                .as_str()
                .and_then(|n| std::env::var(n).ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "extensions.connection.authentication_environment_variable_not_found"
                    ))
                })?;
            headers.insert(key.clone(), value);
        }
    }
    if let Some(name) = server["bearer_token_env_var"].as_str() {
        headers.insert(
            "Authorization".into(),
            format!(
                "Bearer {}",
                std::env::var(name).map_err(|_| anyhow::anyhow!(conduit::i18n::text(
                    "extensions.connection.bearer_token_environment_variable_not_found"
                )))?
            ),
        );
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    let post = |message: &Value,
                session: Option<&str>,
                version: Option<&str>|
     -> Result<(Value, Option<String>)> {
        ensure!(
            !cancel.load(Ordering::Acquire),
            conduit::i18n::text("settings.extensions.test_cancelled")
        );
        let mut request = agent
            .post(address)
            .header("Accept", "application/json, text/event-stream");
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(version) = version {
            request = request.header("MCP-Protocol-Version", version);
        }
        let mut response = request.send_json(message).map_err(|error| match error {
            ureq::Error::StatusCode(code) => {
                anyhow::anyhow!(conduit::i18n::format(
                    "extensions.connection.mcp_http_check_authentication_and_url_complete",
                    &[("code", format!("{}", code))]
                ))
            }
            _ => anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.mcp_http_request_failed_or_timed_out"
            )),
        })?;
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let Some(id) = message["id"].as_i64() else {
            return Ok((Value::Null, session));
        };
        let sse = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        let value = if sse {
            let reader = response.body_mut().as_reader().take(2 * 1024 * 1024);
            sse_response(BufReader::new(reader), id)?
        } else {
            response
                .body_mut()
                .with_config()
                .limit(2 * 1024 * 1024)
                .read_json()
                .map_err(|_| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "extensions.connection.invalid_mcp_response_or_response_exceeds_2"
                    ))
                })?
        };
        Ok((result(value, id)?, session))
    };
    let (init, session) = post(&initialize(), None, None)?;
    let version = negotiated(&init)?;
    let outcome = (|| {
        post(&initialized(), session.as_deref(), Some(version))?;
        if init.pointer("/capabilities/tools").is_none() {
            return Ok(ToolReport::default());
        }
        let started = Instant::now();
        collect_tools(|id, cursor| {
            ensure!(
                started.elapsed() < Duration::from_secs(15),
                conduit::i18n::text(
                    "extensions.connection.tool_list_request_timed_out_please_retry"
                )
            );
            post(
                &tools_request(id, cursor),
                session.as_deref(),
                Some(version),
            )
            .map(|(tools, _)| tools)
        })
    })();
    if let Some(session) = session {
        let mut request = agent
            .delete(address)
            .header("Mcp-Session-Id", &session)
            .header("MCP-Protocol-Version", version);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let _ = request.call();
    }
    outcome
}

fn sse_response(reader: impl BufRead, id: i64) -> Result<Value> {
    let mut data = String::new();
    for line in reader.lines() {
        let line = line.map_err(|_| {
            anyhow::anyhow!(conduit::i18n::text(
                "extensions.connection.mcp_event_stream_failed_or_timed_out"
            ))
        })?;
        if line.is_empty() {
            if let Ok(value) = serde_json::from_str::<Value>(&data)
                && value["id"] == id
            {
                return Ok(value);
            }
            data.clear();
        } else if let Some(part) = line.strip_prefix("data:") {
            data.push_str(part.trim_start());
            data.push('\n');
        }
    }
    bail!(conduit::i18n::text(
        "extensions.connection.mcp_event_stream_did_not_contain_the"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_counts_follow_all_pages_deduplicate_and_reject_partial_results() {
        let _locale = crate::test_support::chinese();
        let report = collect_tools(|id, cursor| match id {
            2 => {
                assert_eq!(cursor, None);
                Ok(json!({"tools":[{"name":"first"}],"nextCursor":"page2"}))
            }
            3 => {
                assert_eq!(cursor, Some("page2"));
                Ok(json!({"tools":[{"name":"first"},{"name":"second"}]}))
            }
            _ => panic!("unexpected request"),
        })
        .unwrap();
        assert_eq!(report.count(), 2);
        assert!(report.summary().contains("2 个可用工具"));
        assert_eq!(
            collect_tools(|_, _| Ok(json!({"tools":[]})))
                .unwrap()
                .count(),
            0
        );
        assert!(
            collect_tools(|_, _| Ok(json!({"tools":[],"nextCursor":"repeat"})))
                .unwrap_err()
                .to_string()
                .contains("重复")
        );
        assert!(collect_tools(|_, _| Ok(json!({"tools":[{}]}))).is_err());
        assert!(
            collect_tools(|id, _| if id == 2 {
                Ok(json!({"tools":[{"name":"first"}],"nextCursor":"more"}))
            } else {
                bail!("second page failed")
            })
            .is_err()
        );
    }
    #[test]
    fn event_stream_ignores_notifications_and_never_echoes_server_errors() {
        let _locale = crate::test_support::chinese();
        let stream = b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notification\"}\n\ndata: {\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}\n\n";
        assert_eq!(
            result(sse_response(&stream[..], 1).unwrap(), 1).unwrap()["protocolVersion"],
            "2025-06-18"
        );
        assert!(
            !result(json!({"id":1,"error":{"code":-1,"message":"secret"}}), 1)
                .unwrap_err()
                .to_string()
                .contains("secret")
        );
        assert!(negotiated(&json!({"protocolVersion":"future"})).is_err());
    }

    #[test]
    fn http_handshake_lists_tools_and_closes_session_without_invoking_tools() {
        let _locale = crate::test_support::chinese();
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}/mcp", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let started = Instant::now();
            let mut methods = vec![];
            while methods.len() < 5 && started.elapsed() < Duration::from_secs(10) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0;
                let mut has_auth = false;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                        if name.eq_ignore_ascii_case("authorization") {
                            has_auth = value.trim() == "Bearer test-only";
                        }
                    }
                }
                assert!(has_auth);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (status, body, extra) = if first.starts_with("DELETE") {
                    methods.push("DELETE".to_string());
                    (200, "{}".to_string(), "")
                } else {
                    let value: Value = serde_json::from_slice(&body).unwrap();
                    let method = value["method"].as_str().unwrap();
                    methods.push(method.to_owned());
                    match method {
                        "initialize" => (200, json!({"id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}}).to_string(), "Mcp-Session-Id: test-session\r\n"),
                        "notifications/initialized" => (202, String::new(), ""),
                        "tools/list" => {
                            let response = if value["id"] == 2 {
                                assert!(value["params"]["cursor"].is_null());
                                json!({"id":2,"result":{"tools":[{"name":"fixture_tool"}],"nextCursor":"page2"}})
                            } else {
                                assert_eq!(value["params"]["cursor"], "page2");
                                json!({"id":3,"result":{"tools":[{"name":"second_tool"}]}})
                            };
                            (200, response.to_string(), "")
                        },
                        _ => panic!("unexpected method"),
                    }
                };
                write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
            }
            methods
        });
        let result = http(
            &json!({"headers":{"Authorization":"Bearer test-only"}}),
            &address,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.count(), 2);
        assert!(result.summary().contains("fixture_tool"));
        assert_eq!(
            worker.join().unwrap(),
            vec![
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/list",
                "DELETE"
            ]
        );
    }

    #[test]
    fn stdio_handshake_cleans_up_the_test_process() {
        let _locale = crate::test_support::chinese();
        let directory = std::env::temp_dir().join(format!(
            "conduit-mcp-check-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let script = directory.join("server.ps1");
        fs::write(&script, r#"while ($line = [Console]::ReadLine()) {
  $message = $line | ConvertFrom-Json
  if ($message.method -eq 'initialize') { [Console]::WriteLine('{"id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}}') }
  if ($message.method -eq 'tools/list' -and $message.id -eq 2) { [Console]::WriteLine('{"id":2,"result":{"tools":[{"name":"local_fixture"}],"nextCursor":"next"}}') }
  if ($message.method -eq 'tools/list' -and $message.id -eq 3) {
    if ($message.params.cursor -ne 'next') { exit 2 }
    [Console]::WriteLine('{"id":3,"result":{"tools":[{"name":"second_fixture"}]}}')
  }
}"#).unwrap();
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let result = stdio(&json!({"command":powershell,"args":["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File",script]}), &directory, &AtomicBool::new(false)).unwrap();
        assert_eq!(result.count(), 2);
        assert!(result.summary().contains("local_fixture"));
        fs::remove_dir_all(directory).unwrap();
    }
}
