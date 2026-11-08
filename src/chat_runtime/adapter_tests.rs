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
            [Console]::ReadLine() | Out-Null
            $lock = [IO.File]::Open($env:CONDUIT_ADAPTER_RESOURCE, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
            [Console]::WriteLine('{"fixture":"ready"}')
            Start-Sleep -Seconds 60
        "#,
        );
        adapter.panic_on_ready = panic_on_ready;
        let runtime = run(adapter, None);
        if !panic_on_ready {
            assert!(matches!(
                runtime
                    .events
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap(),
                Event::NativeId(_)
            ));
            runtime.cancel();
        }
        let (error, cancelled, _) = finished(&runtime, false);
        assert_eq!(cancelled, !panic_on_ready);
        assert_eq!(error.is_some(), panic_on_ready);
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
