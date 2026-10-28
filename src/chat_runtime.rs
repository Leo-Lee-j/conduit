use crate::{
    child::{Child, available},
    domain::{PermissionLevel, Provider},
    protocol::{Approval, Event, Protocol},
};
use anyhow::{Result, bail};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
mod opencode_tests;

enum Control {
    Decision { id: String, allow: bool },
}
#[derive(Default)]
struct ApprovalBroker {
    next: u64,
    pending: HashMap<String, Approval>,
    tickets: HashMap<String, String>,
}
impl ApprovalBroker {
    fn issue(&mut self, mut approval: Approval) -> Result<Approval> {
        let source = approval.id.clone();
        if self.pending.contains_key(&source) {
            bail!("CLI 重复发送未完成审批 ID，任务已停止");
        }
        self.next += 1;
        approval.id = format!("approval-{}", self.next);
        self.tickets.insert(approval.id.clone(), source.clone());
        self.pending.insert(source, approval.clone());
        Ok(approval)
    }
    fn revoke(&mut self, source: &str) -> Option<String> {
        let approval = self.pending.remove(source)?;
        self.tickets.remove(&approval.id);
        Some(approval.id)
    }
    fn take(&mut self, ticket: &str) -> Option<Approval> {
        let source = self.tickets.remove(ticket)?;
        self.pending.remove(&source)
    }
}
pub struct ChatRuntime {
    pub events: Receiver<Event>,
    controls: Sender<Control>,
    cancelled: Arc<AtomicBool>,
}
impl ChatRuntime {
    /// Controllable event source for UI concurrency regressions; starts no CLI.
    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn test_channel() -> (Self, Sender<Event>) {
        let (sender, events) = mpsc::channel();
        let (controls, _) = mpsc::channel();
        (
            Self {
                events,
                controls,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
            sender,
        )
    }
    /// Isolated helper turn; never resumes or mutates the chat conversation.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn start_text(
        provider: Provider,
        executable: PathBuf,
        prompt: String,
        model: Option<String>,
    ) -> Self {
        Self::start_text_with_route(provider, executable, prompt, model, Default::default())
    }
    pub fn start_text_with_route(
        provider: Provider,
        executable: PathBuf,
        prompt: String,
        model: Option<String>,
        route: crate::providers::RuntimeRoute,
    ) -> Self {
        let cwd = std::env::temp_dir();
        let protocol = Protocol::new(provider, cwd.to_string_lossy().into_owned(), prompt, None)
            .with_model(model)
            .with_options(Default::default(), route)
            .text_only();
        let args = protocol.args();
        Self::start_command(protocol, executable, args, cwd)
    }
    #[cfg(test)]
    pub fn start(
        provider: Provider,
        executable: PathBuf,
        cwd: PathBuf,
        prompt: String,
        native_id: Option<String>,
        model: Option<String>,
        permission: PermissionLevel,
    ) -> Self {
        Self::start_with_options(
            provider,
            executable,
            cwd,
            prompt,
            native_id,
            model,
            permission,
            Default::default(),
            Default::default(),
        )
    }
    #[allow(clippy::too_many_arguments)] // Existing engine runtime entry point.
    pub fn start_with_options(
        provider: Provider,
        executable: PathBuf,
        cwd: PathBuf,
        prompt: String,
        native_id: Option<String>,
        model: Option<String>,
        permission: PermissionLevel,
        effort: crate::providers::ReasoningEffort,
        route: crate::providers::RuntimeRoute,
    ) -> Self {
        let protocol = Protocol::new(
            provider,
            cwd.to_string_lossy().into_owned(),
            prompt,
            native_id,
        )
        .with_model(model)
        .with_permission(permission)
        .with_options(effort, route);
        let args = protocol.args();
        Self::start_command(protocol, executable, args, cwd)
    }

    #[cfg(test)]
    fn start_for_approval_test(
        provider: Provider,
        executable: PathBuf,
        cwd: PathBuf,
        prompt: String,
        claude_model: Option<String>,
        claude_permission_mode: Option<String>,
    ) -> Self {
        let mut protocol =
            Protocol::new(provider, cwd.to_string_lossy().into_owned(), prompt, None);
        protocol.approval_test = true;
        protocol.test_model = claude_model;
        protocol.test_permission_mode = claude_permission_mode;
        let args = protocol.args();
        Self::start_command(protocol, executable, args, cwd)
    }
    fn start_command(
        protocol: Protocol,
        executable: PathBuf,
        args: Vec<String>,
        cwd: PathBuf,
    ) -> Self {
        let (tx, events) = mpsc::channel();
        let (controls, rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        thread::spawn(move || {
            let sender = tx.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                execute(protocol, executable, args, cwd, stop.clone(), rx, &tx)
            }));
            let error = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(format!("{error:#}")),
                Err(_) => {
                    let _ = sender.send(Event::Notice("CLI 工作线程异常，进程身份无法继续核实；该会话保持占用，请重启应用后检查。".into()));
                    return;
                }
            };
            let _ = sender.send(Event::Finished {
                error,
                cancelled: stop.load(Ordering::Acquire),
            });
        });
        Self {
            events,
            controls,
            cancelled,
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn decide(&self, id: String, allow: bool) {
        let _ = self.controls.send(Control::Decision { id, allow });
    }
}
impl Drop for ChatRuntime {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Lines {
    buffer: Vec<u8>,
    eof: bool,
}
impl Lines {
    fn new() -> Self {
        Self {
            buffer: vec![],
            eof: false,
        }
    }
    fn poll(&mut self, file: &mut File) -> Result<Vec<Value>> {
        let count = match available(file) {
            Ok(count) => count,
            Err(error) if matches!(error.raw_os_error(), Some(109 | 232 | 233)) => {
                self.eof = true;
                0
            }
            Err(error) => return Err(error.into()),
        };
        let mut chunk = [0; 32768];
        let count = (count as usize).min(chunk.len());
        if count > 0 {
            let count = file.read(&mut chunk[..count])?;
            // Bound each pending message below, not the lifetime of the stream:
            // long turns can emit many small messages without growing this buffer.
            self.buffer.extend_from_slice(&chunk[..count]);
        }
        let mut messages = vec![];
        let mut consumed = 0;
        for (index, byte) in self.buffer.iter().enumerate() {
            if *byte == b'\n' {
                let line = &self.buffer[consumed..index];
                consumed = index + 1;
                if line.len() > 8 * 1024 * 1024 {
                    bail!("CLI 单条协议消息超过 8 MiB 限制");
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                messages.push(serde_json::from_slice(line).map_err(|_| {
                    anyhow::anyhow!("CLI 返回非 JSON 协议消息，未将诊断文本混入模型回复")
                })?);
            }
        }
        self.buffer.drain(..consumed);
        if self.buffer.len() > 8 * 1024 * 1024 {
            bail!("CLI 单条协议消息超过 8 MiB 限制");
        }
        if self.eof && !self.buffer.is_empty() {
            messages.push(serde_json::from_slice(&self.buffer)?);
            self.buffer.clear();
        }
        Ok(messages)
    }
}

fn send(writer: &SyncSender<Vec<u8>>, value: Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    writer
        .try_send(bytes)
        .map_err(|_| anyhow::anyhow!("CLI 输入队列不可用，任务未继续"))
}

fn execute(
    mut protocol: Protocol,
    executable: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    cancel: Arc<AtomicBool>,
    controls: Receiver<Control>,
    events: &Sender<Event>,
) -> Result<()> {
    protocol.prepare()?;
    let (program, full_args) = crate::cli::command_for(&executable, &args);
    let (child, mut pipes) =
        Child::spawn_with_env(&program, &full_args, &cwd, protocol.environment())?;
    let (writer, input) = mpsc::sync_channel::<Vec<u8>>(128);
    let done = Arc::new(AtomicBool::new(false));
    let writer_done = done.clone();
    let (writer_error, failures) = mpsc::channel();
    thread::spawn(move || {
        while !writer_done.load(Ordering::Acquire) {
            match input.recv_timeout(Duration::from_millis(50)) {
                Ok(bytes) => {
                    if let Err(error) = pipes.stdin.write_all(&bytes) {
                        let _ = writer_error.send(error);
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        }
    });
    // Keep the Child outside the unwind boundary so even a protocol-loop panic
    // reaches the same verified cleanup barrier as cancellation and errors.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
        send(&writer, protocol.initialize())?;
        let mut lines = Lines::new();
        let mut stderr = Vec::new();
        let mut approvals = ApprovalBroker::default();
        let mut last_message = Instant::now();
        let mut initialized = false;
        let mut exited_at = None;
        let mut cancel_started: Option<Instant> = None;
        loop {
            if cancel.load(Ordering::Acquire) {
                if cancel_started.is_none() {
                    if let Some(message) = protocol.cancel_message() {
                        send(&writer, message)?;
                        cancel_started = Some(Instant::now());
                    } else {
                        return Ok(());
                    }
                }
                if cancel_started
                    .is_some_and(|started| started.elapsed() >= Duration::from_millis(750))
                {
                    return Ok(());
                }
            }
            if let Ok(error) = failures.try_recv() {
                bail!("CLI 标准输入错误：{error}");
            }
            let messages = lines.poll(&mut pipes.stdout)?;
            for message in messages {
                last_message = Instant::now();
                if message.get("result").is_some()
                    || message.get("type").and_then(Value::as_str) == Some("control_response")
                {
                    initialized = true;
                }
                let step = protocol.accept(message);
                for write in step.writes {
                    // A queued configuration reply must not start a new prompt
                    // after the user has cancelled the turn.
                    if !cancel.load(Ordering::Acquire) {
                        send(&writer, write)?;
                    }
                }
                for event in step.events {
                    let event = match event {
                        Event::Approval(approval) => {
                            if cancel.load(Ordering::Acquire) {
                                send(&writer, protocol.approval_response(&approval, false))?;
                                continue;
                            }
                            let decision = protocol.automatic_approval(&approval);
                            let approval = approvals.issue(approval)?;
                            if let Some(allow) = decision {
                                let pending = approvals.take(&approval.id).unwrap();
                                send(&writer, protocol.approval_response(&pending, allow))?;
                                continue;
                            }
                            Event::Approval(approval)
                        }
                        Event::ApprovalCancelled(source) => match approvals.revoke(&source) {
                            Some(ticket) => Event::ApprovalCancelled(ticket),
                            None => continue,
                        },
                        other => other,
                    };
                    let _ = events.send(event);
                }
                if step.done {
                    if let Some(error) = step.error {
                        bail!("{error}");
                    }
                    return Ok(());
                }
            }
            // Observe queued revocations before submitting UI decisions. Local
            // tickets prevent a delayed click from granting a reused wire ID.
            while let Ok(Control::Decision { id, allow }) = controls.try_recv() {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                if let Some(approval) = approvals.take(&id) {
                    let allow = allow && approval.allow_once;
                    send(&writer, protocol.approval_response(&approval, allow))?;
                    let _ = events.send(Event::ApprovalResolved { id, allow });
                }
            }
            if let Ok(count) = available(&pipes.stderr)
                && count > 0
            {
                let mut chunk = [0; 8192];
                let count = (count as usize).min(chunk.len());
                let count = pipes.stderr.read(&mut chunk[..count])?;
                let keep = count.min(8192_usize.saturating_sub(stderr.len()));
                stderr.extend_from_slice(&chunk[..keep]);
            }
            if let Some(code) = child.exit_code()? {
                let exited = *exited_at.get_or_insert_with(Instant::now);
                if lines.eof || exited.elapsed() > Duration::from_secs(2) {
                    let diagnostic = String::from_utf8_lossy(&stderr);
                    bail!("CLI 在返回任务完成事件前退出（{code}）。{diagnostic}");
                }
            }
            if (!initialized || protocol.startup_pending())
                && last_message.elapsed() > Duration::from_secs(60)
            {
                bail!("CLI 初始化超过 60 秒；请检查登录状态、CLI 配置和 MCP 启动项");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }))
    .unwrap_or_else(|_| {
        Err(anyhow::anyhow!(
            "CLI 协议处理线程异常；已进入进程清理流程，请检查结果后重试"
        ))
    });
    // No Finished event is emitted before the entire Job has been confirmed empty.
    done.store(true, Ordering::Release);
    drop(writer);
    let grace = Instant::now();
    while !cancel.load(Ordering::Acquire) && grace.elapsed() < Duration::from_millis(1500) {
        if matches!(child.active_processes(), Ok(0)) && matches!(child.exit_code(), Ok(Some(_))) {
            break;
        }
        // EOF allows CLI-native session persistence and shutdown to finish. Drain trailing diagnostics.
        for file in [&mut pipes.stdout, &mut pipes.stderr] {
            if let Ok(count) = available(file)
                && count > 0
            {
                let mut buffer = [0; 8192];
                let count = (count as usize).min(buffer.len());
                let _ = file.read(&mut buffer[..count]);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut termination_error_reported = false;
    loop {
        if let Err(error) = child.terminate()
            && !termination_error_reported
        {
            let _ = events.send(Event::Notice(format!(
                "进程树停止失败，该会话保持占用：{error}"
            )));
            termination_error_reported = true;
        }
        if matches!(child.active_processes(), Ok(0)) && matches!(child.exit_code(), Ok(Some(_))) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    drop(child);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_streams_more_than_32_mib_and_parses_final_message_at_eof() {
        use std::os::windows::io::FromRawHandle;
        use windows_sys::Win32::System::Pipes::CreatePipe;

        let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert_ne!(
            unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 65536) },
            0
        );
        let mut read = unsafe { File::from_raw_handle(read) };
        let mut write = unsafe { File::from_raw_handle(write) };
        let payload = "x".repeat(8192);
        let record = format!("{{\"payload\":\"{payload}\"}}\n");
        let records = 4352;
        assert!(record.len() * records > 32 * 1024 * 1024);
        let record_size = record.len();
        let writer = thread::spawn(move || {
            for _ in 0..records {
                write.write_all(record.as_bytes()).unwrap();
            }
            write.write_all(b"{\"done\":true}").unwrap();
        });

        let mut lines = Lines::new();
        let mut parsed = 0;
        let mut done = false;
        let deadline = Instant::now() + Duration::from_secs(30);
        while !lines.eof {
            assert!(Instant::now() < deadline, "stream did not reach EOF");
            for value in lines.poll(&mut read).unwrap() {
                if parsed < records {
                    assert_eq!(value["payload"].as_str(), Some(payload.as_str()));
                    parsed += 1;
                } else {
                    assert_eq!(value, serde_json::json!({"done": true}));
                    assert!(!done);
                    done = true;
                }
            }
            assert!(lines.buffer.len() < record_size);
            thread::yield_now();
        }
        writer.join().unwrap();
        assert_eq!(parsed, records);
        assert!(done);
        assert!(lines.buffer.is_empty());
    }

    fn approval(id: &str) -> Approval {
        Approval {
            id: id.into(),
            title: "test request".into(),
            detail: String::new(),
            wire_id: serde_json::json!(id),
            input: Value::Null,
            allow_once: true,
            kind: crate::protocol::ApprovalKind::Command,
        }
    }

    #[test]
    fn broker_queues_independent_approvals_and_revocation_invalidates_only_its_ticket() {
        let mut broker = ApprovalBroker::default();
        let first = broker.issue(approval("wire-1")).unwrap();
        let second = broker.issue(approval("wire-2")).unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(broker.revoke("wire-1").as_deref(), Some(first.id.as_str()));
        assert!(broker.take(&first.id).is_none());
        assert_eq!(
            broker.take(&second.id).unwrap().wire_id,
            serde_json::json!("wire-2")
        );
        assert!(broker.take(&second.id).is_none());
    }

    fn cleanup_temp_dir(cwd: &std::path::Path) {
        let mut last_error = None;
        for _ in 0..100 {
            match std::fs::remove_dir(cwd) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => {
                    last_error = Some(error);
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
        panic!("temporary workspace could not be removed: {cwd:?}: {last_error:?}");
    }

    fn fixture(command: &str) -> ChatRuntime {
        let cwd = std::env::temp_dir();
        let protocol = Protocol::new(
            Provider::Codex,
            cwd.to_string_lossy().into_owned(),
            "fixture".into(),
            None,
        );
        let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        ChatRuntime::start_command(
            protocol,
            executable,
            ["-NoProfile", "-NonInteractive", "-Command", command]
                .map(String::from)
                .into(),
            cwd,
        )
    }
    #[test]
    fn runtime_cancellation_finishes_after_descendant_exits() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        let runtime = fixture(
            r#"
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
            $nested = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile -NonInteractive -Command Start-Sleep -Seconds 60' -WindowStyle Hidden -PassThru
            $item = @{method='item/started';params=@{item=@{id='fixture-tool';type='commandExecution';command=[string]$nested.Id}}} | ConvertTo-Json -Compress -Depth 5
            [Console]::WriteLine($item)
            Start-Sleep -Seconds 60
        "#,
        );
        let title = loop {
            match runtime
                .events
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
            {
                Event::Tool { title, .. } => break title,
                Event::ToolInput { .. } => {}
                other => panic!("fixture did not start: {other:?}"),
            }
        };
        let pid: u32 = title.parse().unwrap();
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!handle.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        runtime.cancel();
        let Event::Finished { cancelled, error } = runtime
            .events
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
        else {
            panic!("missing completion");
        };
        assert!(cancelled);
        assert!(error.is_none());
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) },
            WAIT_OBJECT_0
        );
    }
    #[test]
    fn cancelling_one_runtime_leaves_another_running() {
        let first = fixture(
            r#"
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
            $item = @{method='item/started';params=@{item=@{id='first-tool';type='commandExecution';command='first'}}} | ConvertTo-Json -Compress -Depth 5
            [Console]::WriteLine($item)
            Start-Sleep -Seconds 60
        "#,
        );
        let second = fixture(
            r#"
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
            $item = @{method='item/started';params=@{item=@{id='second-tool';type='commandExecution';command='second'}}} | ConvertTo-Json -Compress -Depth 5
            [Console]::WriteLine($item)
            Start-Sleep -Milliseconds 800
            [Console]::WriteLine('{"method":"turn/completed","params":{"turn":{"status":"completed"}}}')
        "#,
        );
        for (runtime, expected) in [(&first, "first"), (&second, "second")] {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                assert!(
                    Instant::now() < deadline,
                    "{expected} fixture did not start"
                );
                match runtime.events.recv_timeout(Duration::from_secs(3)) {
                    Ok(Event::Tool { title, .. }) => {
                        assert_eq!(title, expected);
                        break;
                    }
                    Ok(Event::ToolInput { .. }) => {}
                    Ok(other) => panic!("unexpected {expected} event: {other:?}"),
                    Err(error) => panic!("{expected} fixture failed: {error}"),
                }
            }
        }
        first.cancel();
        let Event::Finished { cancelled, error } =
            first.events.recv_timeout(Duration::from_secs(10)).unwrap()
        else {
            panic!("cancelled runtime did not finish");
        };
        assert!(cancelled);
        assert!(error.is_none(), "{error:?}");
        let Event::Finished { cancelled, error } =
            second.events.recv_timeout(Duration::from_secs(10)).unwrap()
        else {
            panic!("second runtime did not finish");
        };
        assert!(!cancelled);
        assert!(error.is_none(), "{error:?}");
    }
    #[test]
    fn runtime_queues_multiple_approvals_and_answers_each_once() {
        let log =
            std::env::temp_dir().join(format!("conduit-approval-queue-{}.log", std::process::id()));
        let command = r#"
            [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding
            [Console]::WriteLine('{"id":41,"method":"item/commandExecution/requestApproval","params":{"itemId":"first"}}')
            [Console]::WriteLine('{"id":42,"method":"item/commandExecution/requestApproval","params":{"itemId":"second"}}')
            $received = @()
            for ($i = 0; $i -lt 3; $i++) {
                $line = [Console]::ReadLine()
                if ($null -eq $line) { break }
                $received += $line
            }
            Set-Content -LiteralPath '__LOG__' -Value $received -Encoding utf8
            [Console]::WriteLine('{"method":"turn/completed","params":{"turn":{"status":"completed"}}}')
            Start-Sleep -Seconds 5
        "#
        .replace("__LOG__", &log.display().to_string());
        let runtime = fixture(&command);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut approvals = vec![];
        while approvals.len() < 2 {
            assert!(Instant::now() < deadline, "approvals did not queue");
            if let Ok(Event::Approval(approval)) =
                runtime.events.recv_timeout(Duration::from_secs(3))
            {
                approvals.push(approval);
            }
        }
        assert_ne!(approvals[0].id, approvals[1].id);
        runtime.decide(approvals[0].id.clone(), true);
        runtime.decide(approvals[1].id.clone(), false);

        let mut resolved = HashMap::new();
        loop {
            assert!(Instant::now() < deadline, "approvals were not resolved");
            match runtime.events.recv_timeout(Duration::from_secs(3)) {
                Ok(Event::ApprovalResolved { id, allow }) => {
                    resolved.insert(id, allow);
                }
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(!cancelled);
                    assert!(error.is_none(), "{error:?}");
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        assert_eq!(resolved.len(), 2);
        let responses: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        let response = |id: u64| {
            responses
                .iter()
                .find(|value| value["id"] == serde_json::json!(id))
                .unwrap_or_else(|| panic!("missing response for {id}: {responses:?}"))
        };
        assert_eq!(response(41)["result"]["decision"], "accept");
        assert_eq!(response(42)["result"]["decision"], "decline");
        std::fs::remove_file(log).unwrap();
    }

    #[test]
    fn full_access_answers_tool_permission_without_showing_an_approval() {
        let log =
            std::env::temp_dir().join(format!("conduit-full-access-{}.jsonl", std::process::id()));
        let command = r#"
            [Console]::WriteLine('{"id":61,"method":"item/commandExecution/requestApproval","params":{"itemId":"tool"}}')
            $lines = @([Console]::ReadLine(), [Console]::ReadLine())
            Set-Content -LiteralPath '__LOG__' -Value $lines -Encoding utf8
            [Console]::WriteLine('{"method":"turn/completed","params":{"turn":{"status":"completed"}}}')
            Start-Sleep -Seconds 5
        "#.replace("__LOG__", &log.display().to_string());
        let protocol = Protocol::new(
            Provider::Codex,
            std::env::temp_dir().to_string_lossy().into_owned(),
            "fixture".into(),
            None,
        )
        .with_permission(PermissionLevel::FullAccess);
        let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let runtime = ChatRuntime::start_command(
            protocol,
            executable,
            ["-NoProfile", "-NonInteractive", "-Command", &command]
                .map(String::from)
                .into(),
            std::env::temp_dir(),
        );
        loop {
            match runtime
                .events
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
            {
                Event::Approval(_) => panic!("full access must not ask for tool permission"),
                Event::Finished { error, cancelled } => {
                    assert!(error.is_none(), "{error:?}");
                    assert!(!cancelled);
                    break;
                }
                _ => {}
            }
        }
        let values: Vec<Value> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter_map(|v| serde_json::from_str(v).ok())
            .collect();
        assert!(
            values
                .iter()
                .any(|v| v["id"] == 61 && v["result"]["decision"] == "accept")
        );
        std::fs::remove_file(log).unwrap();
    }
    #[test]
    fn runtime_reports_premature_exit_and_malformed_protocol() {
        for (command, expected) in [
            ("[Console]::ReadLine() | Out-Null; exit 9", "退出（9）"),
            (
                "[Console]::WriteLine('invalid JSON'); Start-Sleep -Seconds 60",
                "非 JSON",
            ),
        ] {
            let runtime = fixture(command);
            let event = runtime
                .events
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            let Event::Finished {
                error: Some(error),
                cancelled: false,
            } = event
            else {
                panic!("unexpected outcome: {event:?}");
            };
            assert!(error.contains(expected), "{error}");
        }
    }
    #[test]
    fn missing_executable_finishes_with_error() {
        let runtime = ChatRuntime::start(
            Provider::Codex,
            PathBuf::from(r"Z:\missing-conduit-cli.exe"),
            std::env::temp_dir(),
            "test".into(),
            None,
            None,
            PermissionLevel::default(),
        );
        let event = runtime.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            event,
            Event::Finished {
                error: Some(_),
                cancelled: false
            }
        ));
    }
    #[test]
    #[ignore = "requires installed CLIs; handshake only, no model request"]
    fn installed_commit_helpers_initialize_without_model_request() {
        for provider in [Provider::Claude, Provider::Codex] {
            let executable = provider.executable().expect("CLI not installed");
            let mut protocol = Protocol::new(
                provider,
                std::env::temp_dir().to_string_lossy().into_owned(),
                "unused".into(),
                None,
            )
            .text_only();
            let (program, args) = crate::cli::command_for(&executable, &protocol.args());
            let (child, mut pipes) = Child::spawn(&program, &args, &std::env::temp_dir()).unwrap();
            writeln!(pipes.stdin, "{}", protocol.initialize()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(45);
            let mut lines = Lines::new();
            let mut initialized = false;
            while Instant::now() < deadline {
                for value in lines.poll(&mut pipes.stdout).unwrap() {
                    if provider == Provider::Codex && value["id"] == 1 {
                        assert!(
                            value.get("error").is_none(),
                            "helper initialization failed: {value}"
                        );
                        // Start an ephemeral read-only thread but do NOT send turn/start.
                        for write in protocol.accept(value).writes {
                            writeln!(pipes.stdin, "{write}").unwrap();
                        }
                    } else if provider == Provider::Codex && value["id"] == 2 {
                        assert!(
                            value.get("error").is_none(),
                            "helper thread failed: {value}"
                        );
                        initialized = true;
                    } else if provider == Provider::Claude && value["type"] == "control_response" {
                        assert_eq!(value["response"]["subtype"], "success");
                        initialized = true;
                    }
                }
                if initialized {
                    break;
                }
                if let Some(code) = child.exit_code().unwrap() {
                    let mut error = String::new();
                    let _ = pipes.stderr.read_to_string(&mut error);
                    panic!("{provider:?} helper exited {code}: {error}");
                }
                thread::sleep(Duration::from_millis(10));
            }
            child.terminate().unwrap();
            while child.active_processes().unwrap() > 0 {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(initialized, "{provider:?} helper handshake timed out");
            println!("{provider:?} commit helper initialized; no model request sent");
        }
    }

    #[test]
    #[ignore = "requires installed Codex; custom provider handshake only, no model request"]
    fn installed_custom_codex_route_initializes_without_model_request() {
        use crate::providers::{ReasoningEffort, RuntimeRoute};
        let executable = Provider::Codex.executable().expect("Codex not installed");
        for permission in [
            PermissionLevel::ReadOnly,
            PermissionLevel::WorkspaceWrite,
            PermissionLevel::FullAccess,
        ] {
            let route = RuntimeRoute::endpoint(
                Provider::Codex,
                "Conduit fixture",
                "http://127.0.0.1:1/v1",
                Some("fixture-key".into()),
            );
            let mut protocol = Protocol::new(
                Provider::Codex,
                std::env::temp_dir().to_string_lossy().into_owned(),
                "unused".into(),
                None,
            )
            .with_model(Some("fixture-model".into()))
            .with_permission(permission)
            .with_options(ReasoningEffort::High, route);
            let (program, args) = crate::cli::command_for(&executable, &protocol.args());
            let (child, mut pipes) = Child::spawn_with_env(
                &program,
                &args,
                &std::env::temp_dir(),
                protocol.environment(),
            )
            .unwrap();
            writeln!(pipes.stdin, "{}", protocol.initialize()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut lines = Lines::new();
            let mut ready = false;
            while Instant::now() < deadline && !ready {
                for value in lines.poll(&mut pipes.stdout).unwrap() {
                    if value["id"] == 1 {
                        assert!(value.get("error").is_none(), "{value}");
                        for mut write in protocol.accept(value).writes {
                            if write["id"] == 2 {
                                write["params"]["ephemeral"] = serde_json::json!(true);
                            }
                            writeln!(pipes.stdin, "{write}").unwrap();
                        }
                    } else if value["id"] == 2 {
                        assert!(value.get("error").is_none(), "{permission:?}: {value}");
                        assert_eq!(value["result"]["modelProvider"], "conduit");
                        ready = true;
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
            child.terminate().unwrap();
            assert!(ready, "{permission:?} route handshake timed out");
        }
    }
    #[test]
    #[ignore = "requires installed CLIs; handshake only, no model request"]
    fn installed_cli_protocol_handshakes() {
        for provider in [Provider::Claude, Provider::Codex] {
            let executable = provider.executable().expect("CLI not installed");
            let protocol = Protocol::new(
                provider,
                std::env::temp_dir().to_string_lossy().into_owned(),
                "unused".into(),
                None,
            );
            let (program, args) = crate::cli::command_for(&executable, &protocol.args());
            let (child, mut pipes) = Child::spawn(&program, &args, &std::env::temp_dir()).unwrap();
            writeln!(pipes.stdin, "{}", protocol.initialize()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            let mut lines = Lines::new();
            let mut initialized = false;
            while Instant::now() < deadline {
                for value in lines.poll(&mut pipes.stdout).unwrap() {
                    if provider == Provider::Codex && value["id"] == 1 {
                        assert!(
                            value.get("error").is_none(),
                            "Codex initialization failed: {value}"
                        );
                        initialized = true;
                    }
                    if provider == Provider::Claude && value["type"] == "control_response" {
                        assert_eq!(value["response"]["subtype"], "success");
                        initialized = true;
                    }
                }
                if initialized {
                    break;
                }
                if let Some(code) = child.exit_code().unwrap() {
                    let mut error = String::new();
                    let _ = pipes.stderr.read_to_string(&mut error);
                    panic!("{provider:?} exited {code}: {error}");
                }
                thread::sleep(Duration::from_millis(10));
            }
            child.terminate().unwrap();
            while child.active_processes().unwrap() > 0 {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(initialized, "{provider:?} handshake timed out");
            println!("{provider:?} structured handshake OK");
        }
    }
    fn real_turn(
        provider: Provider,
        cwd: PathBuf,
        prompt: &str,
        native_id: Option<String>,
    ) -> String {
        let model = match provider {
            Provider::Claude => std::env::var("CONDUIT_CLAUDE_TEST_MODEL"),
            Provider::Codex => std::env::var("CONDUIT_CODEX_TEST_MODEL"),
            Provider::OpenCode => std::env::var("CONDUIT_OPENCODE_TEST_MODEL"),
        }
        .ok()
        .filter(|value| !value.is_empty());
        let runtime = ChatRuntime::start(
            provider,
            provider.executable().unwrap(),
            cwd,
            prompt.into(),
            native_id,
            model,
            PermissionLevel::default(),
        );
        let mut id = None;
        let mut response = HashMap::<String, String>::new();
        let deadline = Instant::now() + Duration::from_secs(150);
        loop {
            assert!(Instant::now() < deadline, "real model response timeout");
            match runtime.events.recv_timeout(Duration::from_secs(2)) {
                Ok(Event::NativeId(value)) => id = Some(value),
                Ok(Event::Delta { key, text }) => response.entry(key).or_default().push_str(&text),
                Ok(Event::Text { key, text }) => {
                    response.insert(key, text);
                }
                Ok(Event::Approval(approval)) => runtime.decide(approval.id, false),
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(!cancelled);
                    assert!(error.is_none(), "{provider:?}: {error:?}");
                    assert!(
                        response
                            .values()
                            .any(|text| text.contains("CONDUIT_CHAT_OK")),
                        "unexpected synthetic response: {response:?}"
                    );
                    return id.expect("native session ID missing");
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
    }
    #[test]
    #[ignore = "sends two tiny synthetic model prompts through installed Codex CLI"]
    fn real_codex_chat_and_resume() {
        real_chat_and_resume(Provider::Codex);
    }
    #[test]
    #[ignore = "sends two tiny synthetic model prompts through installed Claude CLI"]
    fn real_claude_chat_and_resume() {
        real_chat_and_resume(Provider::Claude);
    }
    fn real_chat_and_resume(provider: Provider) {
        let cwd = std::env::temp_dir().join(format!(
            "conduit-chat-smoke-{}-{provider:?}",
            std::process::id()
        ));
        std::fs::create_dir_all(&cwd).unwrap();
        let id = real_turn(
            provider,
            cwd.clone(),
            "Reply exactly CONDUIT_CHAT_OK. Do not use any tools.",
            None,
        );
        let resumed = real_turn(
            provider,
            cwd,
            "What exact token did I ask you to reply with in my previous message? Reply only that token. Do not use any tools.",
            Some(id.clone()),
        );
        assert_eq!(id, resumed);
        println!("{provider:?}: real chat + exact native resume OK");
    }

    #[test]
    #[ignore = "sends one small tool-use prompt through installed Codex CLI and denies approval"]
    fn real_codex_command_approval_can_be_denied() {
        let cwd =
            std::env::temp_dir().join(format!("conduit-approval-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let before: Vec<_> = std::fs::read_dir(&cwd)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let runtime = ChatRuntime::start_for_approval_test(
            Provider::Codex,
            crate::cli::resolve(Provider::Codex, &crate::cli::CliSettings::default()).unwrap().path,
            cwd.clone(),
            "Use a shell command to print exactly CONDUIT_APPROVAL_DENIED, then tell me the command was denied. Do not create, modify, or delete any files.".into(),
            None,
            None,
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut approval = false;
        let mut resolved = false;
        loop {
            assert!(Instant::now() < deadline, "approval smoke timed out");
            match runtime.events.recv_timeout(Duration::from_secs(3)) {
                Ok(Event::Approval(request)) => {
                    approval = true;
                    runtime.decide(request.id, false);
                }
                Ok(Event::ApprovalResolved { allow: false, .. }) => resolved = true,
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(!cancelled, "unexpected cancellation: {error:?}");
                    assert!(error.is_none(), "Codex approval denial failed: {error:?}");
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        let after: Vec<_> = std::fs::read_dir(&cwd)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(approval, "Codex did not request command approval");
        assert!(resolved, "Conduit did not observe its denial response");
        assert_eq!(
            before, after,
            "approval smoke changed the temporary workspace"
        );
        drop(runtime);
        cleanup_temp_dir(&cwd);
    }

    #[test]
    #[ignore = "sends one tool-use prompt through installed Codex CLI and cancels while approval is pending"]
    fn real_codex_cancel_during_pending_approval() {
        let cwd = std::env::temp_dir().join(format!("conduit-codex-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let target = cwd.join("cancel.txt");
        let runtime = ChatRuntime::start_for_approval_test(
            Provider::Codex,
            crate::cli::resolve(Provider::Codex, &crate::cli::CliSettings::default())
                .unwrap()
                .path,
            cwd.clone(),
            "Use a shell command to create cancel.txt containing exactly CONDUIT_CANCEL_SHOULD_NOT_EXIST. Do not use apply_patch.".into(),
            None,
            None,
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut approval = false;
        let mut resolved = false;
        loop {
            assert!(
                Instant::now() < deadline,
                "Codex did not request approval before cancellation timeout"
            );
            match runtime.events.recv_timeout(Duration::from_secs(3)) {
                Ok(Event::Approval(_)) => {
                    approval = true;
                    runtime.cancel();
                }
                Ok(Event::ApprovalResolved { .. }) => resolved = true,
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(cancelled, "approval cancellation did not cancel the turn");
                    assert!(error.is_none(), "{error:?}");
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        assert!(approval, "Codex did not request approval");
        assert!(!resolved, "cancelled approval was still resolved");
        assert!(!target.exists(), "cancelled command changed the workspace");
        drop(runtime);
        cleanup_temp_dir(&cwd);
    }
    struct CodexApprovalOutcome {
        approvals: usize,
        resolved: bool,
        last_title: String,
    }
    fn codex_approval_turn(
        cwd: &std::path::Path,
        prompt: &str,
        allow: bool,
        stop_after_apply: Option<&std::path::Path>,
    ) -> CodexApprovalOutcome {
        let executable = crate::cli::resolve(Provider::Codex, &crate::cli::CliSettings::default())
            .unwrap()
            .path;
        let runtime = ChatRuntime::start_for_approval_test(
            Provider::Codex,
            executable,
            cwd.to_path_buf(),
            prompt.into(),
            None,
            None,
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut outcome = CodexApprovalOutcome {
            approvals: 0,
            resolved: false,
            last_title: String::new(),
        };
        loop {
            assert!(
                Instant::now() < deadline,
                "Codex approval smoke timed out (approvals={}, title={}, resolved={})",
                outcome.approvals,
                outcome.last_title,
                outcome.resolved
            );
            if outcome.resolved && stop_after_apply.is_some_and(|path| path.exists()) {
                break;
            }
            match runtime.events.recv_timeout(Duration::from_secs(3)) {
                Ok(Event::Approval(request)) => {
                    outcome.approvals += 1;
                    outcome.last_title = request.title;
                    runtime.decide(request.id, allow);
                }
                Ok(Event::ApprovalResolved {
                    allow: resolved_allow,
                    ..
                }) => {
                    assert_eq!(resolved_allow, allow, "approval decision mismatch");
                    outcome.resolved = true;
                }
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(!cancelled, "unexpected cancellation: {error:?}");
                    assert!(error.is_none(), "Codex tool run failed: {error:?}");
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        drop(runtime);
        outcome
    }
    #[test]
    #[ignore = "sends two apply_patch prompts through installed Codex CLI; first denies, then allows"]
    fn real_codex_file_change_approval_allow_and_deny() {
        let cwd = std::env::temp_dir().join(format!("conduit-codex-file-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();

        let denied = cwd.join("deny.txt");
        let outcome = codex_approval_turn(
            &cwd,
            "Use the apply_patch tool to create deny.txt containing exactly CONDUIT_CODEX_FILE_DENIED. Do not use a shell command. Do not modify any other files.",
            false,
            None,
        );
        assert_eq!(outcome.approvals, 1, "Codex did not request file approval");
        assert_eq!(outcome.last_title, "请求修改文件");
        assert!(outcome.resolved, "Conduit did not observe its denial");
        assert!(!denied.exists(), "denied patch still changed the workspace");

        let allowed = cwd.join("allow.txt");
        let outcome = codex_approval_turn(
            &cwd,
            "Use the apply_patch tool to create allow.txt containing exactly CONDUIT_CODEX_FILE_ALLOWED. Do not use a shell command. Do not modify any other files.",
            true,
            Some(&allowed),
        );
        assert_eq!(outcome.approvals, 1, "Codex did not request file approval");
        assert!(outcome.resolved, "Conduit did not observe its approval");
        let content =
            std::fs::read_to_string(&allowed).expect("allowed patch did not create the file");
        assert!(content.contains("CONDUIT_CODEX_FILE_ALLOWED"));

        std::fs::remove_file(&allowed).unwrap();
        cleanup_temp_dir(&cwd);
    }
    struct ClaudeApprovalOutcome {
        approvals: usize,
        resolved: bool,
        tool_events: usize,
        last_notice: String,
    }
    fn claude_approval_turn(
        cwd: &std::path::Path,
        prompt: &str,
        allow: bool,
    ) -> ClaudeApprovalOutcome {
        let executable = crate::cli::resolve(Provider::Claude, &crate::cli::CliSettings::default())
            .unwrap()
            .path;
        let model = std::env::var("CONDUIT_CLAUDE_TEST_MODEL")
            .ok()
            .filter(|value| !value.is_empty());
        // Manual mode forces can_use_tool instead of relying on local allow rules.
        let runtime = ChatRuntime::start_for_approval_test(
            Provider::Claude,
            executable,
            cwd.to_path_buf(),
            prompt.into(),
            model,
            Some("manual".into()),
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut outcome = ClaudeApprovalOutcome {
            approvals: 0,
            resolved: false,
            tool_events: 0,
            last_notice: String::new(),
        };
        loop {
            assert!(
                Instant::now() < deadline,
                "Claude approval smoke timed out (approvals={}, tool_events={}, notice={})",
                outcome.approvals,
                outcome.tool_events,
                outcome.last_notice
            );
            match runtime.events.recv_timeout(Duration::from_secs(3)) {
                Ok(Event::Approval(request)) => {
                    outcome.approvals += 1;
                    runtime.decide(request.id, allow);
                }
                Ok(Event::Tool { .. }) | Ok(Event::ToolInput { .. }) => {
                    outcome.tool_events += 1;
                }
                Ok(Event::Notice(text)) => outcome.last_notice = text,
                Ok(Event::ApprovalResolved {
                    allow: resolved_allow,
                    ..
                }) => {
                    assert_eq!(resolved_allow, allow, "approval decision mismatch");
                    outcome.resolved = true;
                }
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(!cancelled, "unexpected cancellation: {error:?}");
                    assert!(error.is_none(), "Claude tool run failed: {error:?}");
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        drop(runtime);
        outcome
    }
    #[test]
    #[ignore = "sends two small Write-tool prompts through installed Claude CLI; set CONDUIT_CLAUDE_TEST_MODEL if the configured model is unavailable"]
    fn real_claude_tool_approval_allow_and_deny() {
        let cwd =
            std::env::temp_dir().join(format!("conduit-claude-approval-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();

        let denied = cwd.join("deny.txt");
        let outcome = claude_approval_turn(
            &cwd,
            "Use the Write tool to create deny.txt containing exactly CONDUIT_CLAUDE_DENIED. Do not use any other tool. If the tool is denied, reply exactly CONDUIT_DENY_CONFIRMED and stop.",
            false,
        );
        assert_eq!(
            outcome.approvals, 1,
            "Claude did not request can_use_tool for Write (tool_events={}, notice={})",
            outcome.tool_events, outcome.last_notice
        );
        assert!(outcome.resolved, "Conduit did not observe its denial");
        assert!(!denied.exists(), "denied Write still changed the workspace");

        let allowed = cwd.join("allow.txt");
        let outcome = claude_approval_turn(
            &cwd,
            "Use the Write tool to create allow.txt containing exactly CONDUIT_CLAUDE_ALLOWED. Do not use any other tool. After the tool succeeds, reply exactly CONDUIT_ALLOW_CONFIRMED and stop.",
            true,
        );
        assert_eq!(
            outcome.approvals, 1,
            "Claude did not request can_use_tool for Write (tool_events={}, notice={})",
            outcome.tool_events, outcome.last_notice
        );
        assert!(outcome.resolved, "Conduit did not observe its approval");
        let content = std::fs::read_to_string(&allowed)
            .expect("allowed Write did not create the approved file");
        assert!(content.contains("CONDUIT_CLAUDE_ALLOWED"));

        std::fs::remove_file(&allowed).unwrap();
        cleanup_temp_dir(&cwd);
    }

    #[test]
    #[ignore = "sends one small Write-tool prompt through installed Claude CLI and cancels while approval is pending; set CONDUIT_CLAUDE_TEST_MODEL if the configured model is unavailable"]
    fn real_claude_cancel_during_pending_approval() {
        let cwd =
            std::env::temp_dir().join(format!("conduit-claude-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&cwd).unwrap();
        let target = cwd.join("cancel.txt");
        let executable = crate::cli::resolve(Provider::Claude, &crate::cli::CliSettings::default())
            .unwrap()
            .path;
        let model = std::env::var("CONDUIT_CLAUDE_TEST_MODEL")
            .ok()
            .filter(|value| !value.is_empty());
        // Manual mode forces can_use_tool instead of relying on local allow rules.
        let runtime = ChatRuntime::start_for_approval_test(
            Provider::Claude,
            executable,
            cwd.clone(),
            "Use the Write tool to create cancel.txt containing exactly CONDUIT_CANCEL_SHOULD_NOT_EXIST. Do not use any other tool. If the tool is denied, reply exactly CONDUIT_CANCEL_CONFIRMED and stop.".into(),
            model,
            Some("manual".into()),
        );
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut approval = false;
        let mut resolved = false;
        loop {
            assert!(
                Instant::now() < deadline,
                "Claude did not request can_use_tool before cancellation timeout"
            );
            match runtime.events.recv_timeout(Duration::from_secs(3)) {
                Ok(Event::Approval(_)) => {
                    approval = true;
                    runtime.cancel();
                }
                Ok(Event::ApprovalResolved { .. }) => resolved = true,
                Ok(Event::Finished { error, cancelled }) => {
                    assert!(cancelled, "approval cancellation did not cancel the turn");
                    assert!(error.is_none(), "{error:?}");
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
        assert!(approval, "Claude did not request tool approval");
        assert!(!resolved, "cancelled Claude approval was still resolved");
        assert!(!target.exists(), "cancelled Write changed the workspace");
        drop(runtime);
        cleanup_temp_dir(&cwd);
    }
}
