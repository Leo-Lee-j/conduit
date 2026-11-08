//! A CLI with no Provider variant or Protocol implementation exercises the host.
use super::*;
use crate::protocol::{ApprovalKind, Step};
use serde_json::json;
use std::collections::BTreeMap;

struct FixtureAdapter {
    script: String,
    text_input: Option<String>,
    completion: Completion,
    started: Arc<AtomicBool>,
    released: Sender<bool>,
    resource: PathBuf,
    ready: bool,
    ready_signal: Option<Sender<()>>,
    panic_on_ready: bool,
    fail_launch: bool,
}

impl Drop for FixtureAdapter {
    fn drop(&mut self) {
        // The fixture child keeps this file exclusively open. It can only be
        // deleted after process cleanup, including cancellation and unwinding.
        let removed = !self.resource.exists() || std::fs::remove_file(&self.resource).is_ok();
        let _ = self.released.send(removed);
    }
}

impl CliAdapter for FixtureAdapter {
    fn launch(&mut self) -> Result<LaunchSpec> {
        self.started.store(true, Ordering::Release);
        std::fs::write(&self.resource, b"owned by fixture")?;
        if self.fail_launch {
            bail!("fixture launch failed");
        }
        Ok(LaunchSpec {
            args: ["-NoProfile", "-NonInteractive", "-Command", &self.script]
                .map(String::from)
                .into(),
            environment: BTreeMap::from([(
                "CONDUIT_ADAPTER_RESOURCE".into(),
                self.resource.to_string_lossy().into_owned(),
            )]),
            input: self
                .text_input
                .clone()
                .map(InitialInput::TextToEof)
                .unwrap_or_else(|| InitialInput::Json(json!({"hello":true}))),
            completion: self.completion,
        })
    }
    fn accept(&mut self, value: Value) -> Step {
        let mut step = Step {
            writes: vec![],
            events: vec![],
            done: false,
            error: None,
        };
        match value["fixture"].as_str() {
            Some("ready") => {
                self.ready = true;
                if let Some(ready) = self.ready_signal.take() {
                    let _ = ready.send(());
                }
                assert!(!self.panic_on_ready, "fixture adapter panic");
                step.events.push(Event::NativeId("fixture-session".into()));
            }
            Some("permission") => step.events.push(Event::Approval(Approval {
                id: "wire-ticket".into(),
                title: "fixture approval".into(),
                detail: String::new(),
                wire_id: json!(7),
                input: json!({}),
                allow_once: true,
                kind: ApprovalKind::Command,
            })),
            Some("reply") => {
                step.events.push(Event::Text {
                    key: "reply".into(),
                    text: value["text"].as_str().unwrap().into(),
                });
                step.done = self.completion == Completion::ProtocolEvent;
            }
            _ => {}
        }
        step
    }
    fn startup_pending(&self) -> bool {
        !self.ready
    }
    fn cancel_message(&self) -> Option<Value> {
        self.ready.then(|| json!({"stop":true}))
    }
    fn route_response(&mut self, _: &Approval, response: Value) -> Result<Option<Value>> {
        Ok(Some(json!({"routed":response})))
    }
    fn automatic_approval(&self, _: &Approval) -> Option<bool> {
        None
    }
    fn approval_response(&self, approval: &Approval, allow: bool) -> Value {
        json!({"ticket":approval.wire_id,"allowed":allow})
    }
    fn user_input_response(
        &self,
        _: &Approval,
        _: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        None
    }
    fn enable_full_access(&mut self) {}
}

fn fixture(script: &str) -> (FixtureAdapter, Receiver<bool>) {
    let (released, receiver) = mpsc::channel();
    let resource = std::env::temp_dir().join(format!(
        "conduit-adapter-{}-{}",
        std::process::id(),
        ChatRuntime::next_identity()
    ));
    (
        FixtureAdapter {
            script: script.into(),
            text_input: None,
            completion: Completion::ProtocolEvent,
            started: Arc::new(AtomicBool::new(false)),
            released,
            resource,
            ready: false,
            ready_signal: None,
            panic_on_ready: false,
            fail_launch: false,
        },
        receiver,
    )
}

fn run(adapter: FixtureAdapter, gate: Option<Receiver<()>>) -> ChatRuntime {
    let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    ChatRuntime::start_adapter_gated(
        Box::new(adapter),
        executable,
        None,
        std::env::temp_dir(),
        gate,
    )
}

fn finished(runtime: &ChatRuntime, decide: bool) -> (Option<String>, bool, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut texts = vec![];
    loop {
        assert!(Instant::now() < deadline, "fixture did not finish");
        match runtime.events.recv_timeout(Duration::from_secs(2)) {
            Ok(Event::Approval(approval)) if decide => runtime.decide(approval.id, true),
            Ok(Event::Text { text, .. }) => texts.push(text),
            Ok(Event::Finished { error, cancelled }) => return (error, cancelled, texts),
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("missing terminal event: {e:?}"),
        }
    }
}

#[test]
fn independent_adapter_streams_and_routes_approval_responses_through_generic_runtime() {
    let _locale = crate::test_support::chinese();
    let (adapter, released) = fixture(
        r#"
        $hello = [Console]::ReadLine() | ConvertFrom-Json
        if (-not $hello.hello) { exit 12 }
        [Console]::WriteLine('{"fixture":"ready"}')
        [Console]::WriteLine('{"fixture":"permission"}')
        $reply = [Console]::ReadLine() | ConvertFrom-Json
        if (-not $reply.routed.allowed -or $reply.routed.ticket -ne 7) { exit 13 }
        [Console]::WriteLine('{"fixture":"reply","text":"adapter result"}')
    "#,
    );
    let runtime = run(adapter, None);
    let (error, cancelled, texts) = finished(&runtime, true);
    assert_eq!(error, None);
    assert!(!cancelled);
    assert_eq!(texts, ["adapter result"]);
    assert!(released.recv_timeout(Duration::from_secs(2)).unwrap());
}

#[test]
fn adapter_stays_unlaunched_until_save_and_handles_cancel_or_failed_save() {
    let _locale = crate::test_support::chinese();
    for cancel in [false, true] {
        let (adapter, released) = fixture("exit 99");
        let started = adapter.started.clone();
        let (release, gate) = mpsc::channel();
        let runtime = run(adapter, Some(gate));
        assert!(matches!(
            runtime.events.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(!started.load(Ordering::Acquire));
        if cancel {
            runtime.cancel();
        } else {
            drop(release);
        }
        let (error, cancelled, _) = finished(&runtime, false);
        assert_eq!(cancelled, cancel);
        assert_eq!(error.is_some(), !cancel);
        assert!(!started.load(Ordering::Acquire));
        assert!(released.recv_timeout(Duration::from_secs(2)).unwrap());
    }
    let (mut adapter, released) = fixture("unused");
    adapter.fail_launch = true;
    let started = adapter.started.clone();
    let (release, gate) = mpsc::channel();
    let runtime = run(adapter, Some(gate));
    release.send(()).unwrap();
    let (error, _, _) = finished(&runtime, false);
    assert!(error.unwrap().contains("fixture launch failed"));
    assert!(started.load(Ordering::Acquire));
    assert!(released.recv_timeout(Duration::from_secs(2)).unwrap());
}

#[test]
fn adapter_resources_are_released_after_child_cleanup_on_cancel_and_panic() {
    let _locale = crate::test_support::chinese();
    for panic_on_ready in [false, true] {
        let (mut adapter, released) = fixture(
            r#"
            $ErrorActionPreference = 'Stop'
            [Console]::ReadLine() | Out-Null
            $lock = [IO.File]::Open($env:CONDUIT_ADAPTER_RESOURCE, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
            [Console]::WriteLine('{"fixture":"ready"}')
            Start-Sleep -Seconds 60
        "#,
        );
        adapter.panic_on_ready = panic_on_ready;
        let (ready, startup) = mpsc::channel();
        adapter.ready_signal = Some(ready);
        let runtime = run(adapter, None);
        // PowerShell cold start is not process cleanup. Allow the runtime's
        // 60-second initialization deadline plus its cleanup barrier to report
        // a startup failure, and only then start measuring cancel/panic cleanup.
        // Both branches must prove the child acquired the exclusive file lock.
        if let Err(error) = startup.recv_timeout(Duration::from_secs(75)) {
            runtime.cancel();
            let terminal = finished(&runtime, false);
            panic!("fixture startup did not acquire its resource: {error:?}; {terminal:?}");
        }
        if !panic_on_ready {
            assert!(matches!(
                released.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            runtime.cancel();
        }
        let (error, cancelled, _) = finished(&runtime, false);
        assert_eq!(cancelled, !panic_on_ready);
        if panic_on_ready {
            assert!(error.unwrap().contains(&conduit::i18n::text(
                "chat_runtime.cli_protocol_thread_failed_process_cleanup_started"
            )));
        } else {
            assert_eq!(error, None);
        }
        assert!(
            released.recv_timeout(Duration::from_secs(2)).unwrap(),
            "adapter released before child closed its resource"
        );
    }
}

#[test]
fn text_input_eof_and_successful_exit_are_explicit_adapter_policies() {
    let _locale = crate::test_support::chinese();
    for success_on_exit in [true, false] {
        let (mut adapter, released) = fixture(
            r#"
            $text = [Console]::In.ReadToEnd()
            if ($text -ne 'fixture prompt') { exit 14 }
            [Console]::WriteLine('{"fixture":"ready"}')
        "#,
        );
        adapter.text_input = Some("fixture prompt".into());
        adapter.completion = if success_on_exit {
            Completion::SuccessfulExit
        } else {
            Completion::ProtocolEvent
        };
        let runtime = run(adapter, None);
        let (error, cancelled, _) = finished(&runtime, false);
        assert!(!cancelled);
        assert_eq!(error.is_none(), success_on_exit);
        assert!(released.recv_timeout(Duration::from_secs(2)).unwrap());
    }
}

fn qoder_fixture(script: &str, gate: Option<Receiver<()>>) -> ChatRuntime {
    let cwd = std::env::temp_dir();
    let adapter = (Provider::Qoder.descriptor().adapter)(AdapterRequest {
        cwd: cwd.clone(),
        prompt: "fixture prompt".into(),
        native_id: Some("qoder-original".into()),
        model: None,
        permission: crate::domain::PermissionLevel::WorkspaceWrite,
        effort: crate::providers::ReasoningEffort::Default,
        route: Default::default(),
        text_only: false,
    });
    let script = format!(
        r#"
        $ErrorActionPreference = 'Stop'
        function Send($value) {{ [Console]::WriteLine(($value | ConvertTo-Json -Depth 20 -Compress)) }}
        function Read-Request($method) {{
            $value = [Console]::ReadLine() | ConvertFrom-Json
            if ($value.method -ne $method) {{ throw "Unexpected method: $($value.method)" }}
            return $value
        }}
        $init = Read-Request 'initialize'
        Send @{{jsonrpc='2.0';id=$init.id;result=@{{protocolVersion=1;agentInfo=@{{name='qoder-cli'}};agentCapabilities=@{{sessionCapabilities=@{{resume=@{{}}}}}}}}}}
        $resume = Read-Request 'session/resume'
        if ($resume.params.sessionId -ne 'qoder-original') {{ throw 'Wrong session' }}
        Send @{{jsonrpc='2.0';method='session/update';params=@{{sessionId='qoder-original';update=@{{sessionUpdate='agent_message_chunk';content=@{{type='text';text='old history'}}}}}}}}
        Send @{{jsonrpc='2.0';id=$resume.id;result=@{{sessionId='qoder-original';configOptions=@()}}}}
        $mode = Read-Request 'session/set_mode'
        if ($mode.params.modeId -ne 'default') {{ throw 'Wrong mode' }}
        Send @{{jsonrpc='2.0';id=$mode.id;result=@{{}}}}
        $prompt = Read-Request 'session/prompt'
        if ($prompt.params.prompt.Count -ne 2 -or $prompt.params.prompt[0].text -ne 'fixture prompt') {{ throw 'Wrong prompt' }}
        {script}
    "#
    );
    let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    ChatRuntime::start_adapter_gated(
        adapter,
        executable,
        Some(
            ["-NoProfile", "-NonInteractive", "-Command", &script]
                .map(String::from)
                .into(),
        ),
        cwd,
        gate,
    )
}

#[test]
fn qoder_runtime_resumes_routes_permissions_and_typed_answers_after_save() {
    let _locale = crate::test_support::chinese();
    let (release, gate) = mpsc::channel();
    let runtime = qoder_fixture(
        r#"
        Send @{jsonrpc='2.0';id='permission';method='session/request_permission';params=@{sessionId='qoder-original';toolCall=@{toolCallId='tool';title='Read file'};options=@(@{optionId='yes';kind='allow_once'},@{optionId='no';kind='reject_once'})}}
        $answer = [Console]::ReadLine() | ConvertFrom-Json
        if ($answer.id -ne 'permission' -or $answer.result.outcome.optionId -ne 'yes') { throw 'Wrong approval' }
        Send @{jsonrpc='2.0';id=73;method='elicitation/create';params=@{sessionId='qoder-original';mode='form';message='Choose';requestedSchema=@{type='object';properties=@{count=@{type='integer';minimum=1;maximum=5}}}}}
        $answer = [Console]::ReadLine() | ConvertFrom-Json
        if ($answer.id -ne 73 -or $answer.result.action -ne 'accept' -or $answer.result.content.count -ne 3) { throw 'Wrong form answer' }
        Send @{jsonrpc='2.0';method='session/update';params=@{sessionId='qoder-original';update=@{sessionUpdate='agent_message_chunk';content=@{type='text';text='new reply'}}}}
        Send @{jsonrpc='2.0';id=$prompt.id;result=@{stopReason='end_turn'}}
        [Console]::ReadLine() | Out-Null
    "#,
        Some(gate),
    );
    assert!(matches!(
        runtime.events.recv_timeout(Duration::from_millis(100)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut approvals = 0;
    let mut resolved = 0;
    let mut text = String::new();
    let mut native = None;
    loop {
        assert!(Instant::now() < deadline, "Qoder fixture did not finish");
        match runtime.events.recv_timeout(Duration::from_secs(2)) {
            Ok(Event::NativeId(id)) => native = Some(id),
            Ok(Event::Approval(approval)) => {
                approvals += 1;
                if approval.is_user_input() {
                    runtime.answer(
                        approval.id,
                        BTreeMap::from([("count".into(), vec!["3".into()])]),
                    );
                } else {
                    runtime.decide(approval.id, true);
                }
            }
            Ok(Event::ApprovalResolved { .. }) => resolved += 1,
            Ok(Event::Delta { text: delta, .. }) => text.push_str(&delta),
            Ok(Event::Finished { error, cancelled }) => {
                assert_eq!(error, None);
                assert!(!cancelled);
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("missing terminal event: {error:?}"),
        }
    }
    assert_eq!(native.as_deref(), Some("qoder-original"));
    assert_eq!((approvals, resolved), (2, 2));
    assert_eq!(text, "new reply");
}

#[test]
fn qoder_runtime_cancel_invalidates_pending_questions_and_cleans_up() {
    let _locale = crate::test_support::chinese();
    let runtime = qoder_fixture(
        r#"
        Send @{jsonrpc='2.0';id=73;method='elicitation/create';params=@{sessionId='qoder-original';mode='form';message='Name';requestedSchema=@{type='object';properties=@{name=@{type='string'}}}}}
        $cancel = Read-Request 'session/cancel'
        if ($cancel.params.sessionId -ne 'qoder-original') { throw 'Wrong cancellation' }
        Send @{jsonrpc='2.0';id=$prompt.id;result=@{stopReason='cancelled'}}
        [Console]::ReadLine() | Out-Null
    "#,
        None,
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut ticket = None;
    let mut resolved = false;
    loop {
        assert!(Instant::now() < deadline, "Qoder fixture did not cancel");
        match runtime.events.recv_timeout(Duration::from_secs(2)) {
            Ok(Event::Approval(approval)) => {
                ticket = Some(approval.id);
                runtime.cancel();
            }
            Ok(Event::ApprovalResolved { .. }) => resolved = true,
            Ok(Event::Finished { error, cancelled }) => {
                assert!(cancelled);
                assert_eq!(error, None);
                break;
            }
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => panic!("missing terminal event: {error:?}"),
        }
    }
    assert!(ticket.is_some());
    assert!(!resolved, "cancelled question must never be accepted");
    assert!(
        !runtime.try_decide(ticket.unwrap(), true),
        "finished runtime rejects stale decisions"
    );
}
