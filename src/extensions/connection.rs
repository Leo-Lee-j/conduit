//! Explicit MCP handshake diagnostics. Never invokes tools or samples models.
use super::*;
use anyhow::{Result, bail, ensure};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
};

pub struct ConnectionProbe {
    pub id: String,
    pub events: Receiver<Result<String, String>>,
    cancelled: Arc<AtomicBool>,
}
impl ConnectionProbe {
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
    let text =
        edit::read(&entry.path)?.ok_or_else(|| anyhow::anyhow!("配置文件已不存在，请刷新"))?;
    let value: Value = if entry.path.extension().is_some_and(|e| e == "toml") {
        serde_json::to_value(
            toml::from_str::<toml::Table>(&text).map_err(|_| anyhow::anyhow!("TOML 格式无效"))?,
        )?
    } else {
        jsonc(&text).ok_or_else(|| anyhow::anyhow!("JSON 配置格式无效"))?
    };
    for key in ["mcp_servers", "mcpServers", "mcp"] {
        if let Some(server) = value[key].get(&entry.name).filter(|s| s.is_object()) {
            return Ok(server.clone());
        }
    }
    bail!("此来源需要 CLI 自身解析，请在原生 CLI 中测试连接")
}

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Conduit settings check","version":env!("CARGO_PKG_VERSION")}}})
}
fn initialized() -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/initialized"})
}
fn tools_request() -> Value {
    json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
}

fn result(value: Value, id: i64) -> Result<Value> {
    ensure!(value["id"] == id, "服务器返回了不匹配的响应");
    if !value["error"].is_null() {
        bail!(
            "服务器返回 MCP 错误（代码 {}）；请检查原生 CLI 的配置与认证",
            value["error"]["code"].as_i64().unwrap_or(0)
        );
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("服务器未返回 MCP 结果"))
}
fn negotiated(value: &Value) -> Result<&str> {
    let version = value["protocolVersion"].as_str().unwrap_or("");
    ensure!(
        matches!(version, "2025-06-18" | "2025-03-26" | "2024-11-05"),
        "服务器使用暂不支持的协议版本，请使用 CLI 自身连接"
    );
    Ok(version)
}
fn summary(value: &Value) -> Result<String> {
    let tools = value["tools"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("工具列表格式无效"))?;
    let names = tools
        .iter()
        .take(12)
        .filter_map(|tool| tool["name"].as_str())
        .map(|s| clipped(s, 80))
        .collect::<Vec<_>>();
    Ok(format!(
        "连接成功 · 本页 {} 个工具{}{}。未调用工具。",
        tools.len(),
        if names.is_empty() { "" } else { "：" },
        names.join("、")
    ))
}
fn check(entry: &Entry, context: &Context, cancel: &AtomicBool) -> Result<String> {
    ensure!(!cancel.load(Ordering::Acquire), "测试已取消");
    ensure!(!entry.disabled, "此定义已禁用，请先在来源配置中启用");
    let server = config(entry)?;
    ensure!(
        server["enabled"] != false && server["disabled"] != true,
        "配置已禁用，请刷新列表"
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

fn stdio(server: &Value, cwd: &Path, cancel: &AtomicBool) -> Result<String> {
    let command = server["command"]
        .as_str()
        .or_else(|| server["command"][0].as_str())
        .ok_or_else(|| anyhow::anyhow!("缺少启动命令"))?;
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
                let value = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("环境变量值必须是字符串"))?;
                ensure!(
                    !value.contains("${") && !value.contains("{env:"),
                    "动态环境变量需要 CLI 自身解析，请在 CLI 中测试"
                );
                env.insert(key.clone(), value.to_string());
            }
        }
    }
    // Some Windows runtimes (notably .NET) fail before initializing when argv[0]
    // uses the extended-length prefix introduced by canonicalization.
    let (exe, args) = crate::cli::command_for(&crate::child::normalized_dir(&path), &args);
    let (child, mut pipes) = crate::child::Child::spawn_with_env(&exe, &args, cwd, &env)
        .map_err(|_| anyhow::anyhow!("无法启动 MCP 命令，请检查路径、参数和环境"))?;
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
        return Ok("连接成功 · 服务器未声明工具能力。".into());
    }
    send(&mut pipes.stdin, &tools_request())?;
    summary(&result(
        receive(&mut pipes, &mut buffer, &mut bytes, 2, started, cancel)?,
        2,
    )?)
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
        .ok_or_else(|| anyhow::anyhow!("找不到可执行命令；请安装它或填写完整路径"))
}
fn send(file: &mut fs::File, value: &Value) -> Result<()> {
    writeln!(file, "{value}").map_err(|_| anyhow::anyhow!("MCP 进程输入已关闭"))
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
        ensure!(!cancel.load(Ordering::Acquire), "测试已取消");
        ensure!(
            started.elapsed() < Duration::from_secs(15),
            "连接测试超时（15 秒），已停止测试进程"
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
                Err(_) => bail!("MCP 进程已退出，未完成连接测试"),
            };
            if available > 0 {
                let mut chunk = [0; 4096];
                let read = file.read(&mut chunk[..available.min(4096) as usize])?;
                *bytes += read;
                ensure!(*bytes <= 2 * 1024 * 1024, "MCP 输出超过 2 MiB，已停止测试");
                if index == 0 {
                    buffer.extend_from_slice(&chunk[..read]);
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn http(server: &Value, address: &str, cancel: &AtomicBool) -> Result<String> {
    let url = url::Url::parse(address).map_err(|_| anyhow::anyhow!("MCP URL 无效"))?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none(),
        "MCP URL 必须是 HTTP(S)，且不含账号密码"
    );
    ensure!(
        server["type"] != "sse",
        "旧式 SSE 服务请在 CLI 中连接；此处支持 Streamable HTTP"
    );
    let mut headers = BTreeMap::new();
    for key in ["headers", "http_headers"] {
        if let Some(values) = server[key].as_object() {
            for (key, value) in values {
                let value = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("认证头格式无效"))?;
                ensure!(
                    !value.contains("${") && !value.contains("{env:"),
                    "动态认证头需要 CLI 自身解析"
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
                .ok_or_else(|| anyhow::anyhow!("认证环境变量不存在"))?;
            headers.insert(key.clone(), value);
        }
    }
    if let Some(name) = server["bearer_token_env_var"].as_str() {
        headers.insert(
            "Authorization".into(),
            format!(
                "Bearer {}",
                std::env::var(name).map_err(|_| anyhow::anyhow!("Bearer Token 环境变量不存在"))?
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
        ensure!(!cancel.load(Ordering::Acquire), "测试已取消");
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
                anyhow::anyhow!("MCP HTTP {code}；请检查认证和地址。OAuth 登录请在 CLI 中完成。")
            }
            _ => anyhow::anyhow!("MCP HTTP 请求失败或超时，请检查网络和证书"),
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
                .map_err(|_| anyhow::anyhow!("MCP 响应无效或超过 2 MiB"))?
        };
        Ok((result(value, id)?, session))
    };
    let (init, session) = post(&initialize(), None, None)?;
    let version = negotiated(&init)?;
    let outcome = (|| {
        post(&initialized(), session.as_deref(), Some(version))?;
        if init.pointer("/capabilities/tools").is_none() {
            return Ok("连接成功 · 服务器未声明工具能力。".into());
        }
        let (tools, _) = post(&tools_request(), session.as_deref(), Some(version))?;
        summary(&tools)
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
        let line = line.map_err(|_| anyhow::anyhow!("MCP 事件流读取失败或超时"))?;
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
    bail!("MCP 事件流未包含请求的响应")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_stream_ignores_notifications_and_never_echoes_server_errors() {
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
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}/mcp", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let started = Instant::now();
            let mut methods = vec![];
            while methods.len() < 4 && started.elapsed() < Duration::from_secs(10) {
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
                        "tools/list" => (200, json!({"id":2,"result":{"tools":[{"name":"fixture_tool"}]}}).to_string(), ""),
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
        assert!(result.contains("fixture_tool"));
        assert_eq!(
            worker.join().unwrap(),
            vec![
                "initialize",
                "notifications/initialized",
                "tools/list",
                "DELETE"
            ]
        );
    }

    #[test]
    fn stdio_handshake_cleans_up_the_test_process() {
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
  if ($message.method -eq 'tools/list') { [Console]::WriteLine('{"id":2,"result":{"tools":[{"name":"local_fixture"}]}}') }
}"#).unwrap();
        let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let result = stdio(&json!({"command":powershell,"args":["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File",script]}), &directory, &AtomicBool::new(false)).unwrap();
        assert!(result.contains("local_fixture"));
        fs::remove_dir_all(directory).unwrap();
    }
}
