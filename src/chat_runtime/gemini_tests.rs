//! Exercise the registered Gemini adapter through the real process/save/approval runtime.
use super::*;

struct PolicyFixture(Box<dyn CliAdapter>);
impl CliAdapter for PolicyFixture {
    fn launch(&mut self) -> Result<LaunchSpec> {
        let mut launch = self.0.launch()?;
        let policy = launch
            .args
            .windows(2)
            .find(|w| w[0] == "--admin-policy")
            .unwrap()[1]
            .clone();
        launch
            .environment
            .insert("CONDUIT_GEMINI_FIXTURE_POLICY".into(), policy);
        Ok(launch)
    }
    fn accept(&mut self, value: Value) -> crate::protocol::Step {
        self.0.accept(value)
    }
    fn startup_pending(&self) -> bool {
        self.0.startup_pending()
    }
    fn cancel_message(&self) -> Option<Value> {
        self.0.cancel_message()
    }
    fn automatic_approval(&self, a: &Approval) -> Option<bool> {
        self.0.automatic_approval(a)
    }
    fn approval_response(&self, a: &Approval, allow: bool) -> Value {
        self.0.approval_response(a, allow)
    }
    fn user_input_response(
        &self,
        a: &Approval,
        answers: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        self.0.user_input_response(a, answers)
    }
    fn enable_full_access(&mut self) {
        self.0.enable_full_access();
    }
}

fn fixture(cancel: bool, gate: Receiver<()>) -> ChatRuntime {
    let cwd = std::env::temp_dir();
    let mut route = crate::providers::RuntimeRoute::default();
    route.env.insert(
        "GEMINI_CLI_HOME".into(),
        cwd.join("conduit-gemini-missing-runtime-home")
            .to_string_lossy()
            .into_owned(),
    );
    route.env.insert(
        "GEMINI_CLI_SYSTEM_SETTINGS_PATH".into(),
        cwd.join("conduit-gemini-missing-runtime-fixture/settings.json")
            .to_string_lossy()
            .into_owned(),
    );
    let adapter = (Provider::Gemini.descriptor().adapter)(AdapterRequest {
        intent: Default::default(),
        cwd: cwd.clone(),
        prompt: "fixture prompt".into(),
        native_id: Some("gemini-original".into()),
        model: None,
        permission: crate::domain::PermissionLevel::WorkspaceWrite,
        tool_policy: Default::default(),
        effort: crate::providers::ReasoningEffort::Default,
        route,
        text_only: false,
    });
    let script = r#"
        $ErrorActionPreference = 'Stop'
        function Send($value) { [Console]::WriteLine(($value | ConvertTo-Json -Depth 20 -Compress)) }
        function Read-Request($method) {
            $value = [Console]::ReadLine() | ConvertFrom-Json
            if ($value.method -ne $method) { throw "Unexpected method: $($value.method)" }
            return $value
        }
        $init = Read-Request 'initialize'
        Send @{jsonrpc='2.0';id=$init.id;result=@{protocolVersion=1;agentInfo=@{name='gemini-cli'};agentCapabilities=@{loadSession=$true}}}
        $resume = Read-Request 'session/load'
        if ($resume.params.sessionId -ne 'gemini-original') { throw 'Wrong session' }
        Send @{jsonrpc='2.0';method='session/update';params=@{sessionId='gemini-original';update=@{sessionUpdate='agent_message_chunk';content=@{type='text';text='old history'}}}}
        Send @{jsonrpc='2.0';id=$resume.id;result=@{modes=@{availableModes=@(@{id='default'})}}}
        $mode = Read-Request 'session/set_mode'
        if ($mode.params.modeId -ne 'default') { throw 'Wrong mode' }
        Send @{jsonrpc='2.0';id=$mode.id;result=@{}}
        $prompt = Read-Request 'session/prompt'
        if ($prompt.params.prompt[0].text -ne 'fixture prompt') { throw 'Wrong prompt' }
        $lock = [System.IO.File]::Open($env:CONDUIT_GEMINI_FIXTURE_POLICY,'Open','Read','None')
        Send @{jsonrpc='2.0';method='session/update';params=@{sessionId='gemini-original';update=@{sessionUpdate='agent_message_chunk';content=@{type='text';text=$env:CONDUIT_GEMINI_FIXTURE_POLICY}}}}
        Send @{jsonrpc='2.0';id='permission';method='session/request_permission';params=@{sessionId='gemini-original';toolCall=@{toolCallId='tool';title='Read file'};options=@(@{optionId='yes-native';kind='allow_once'},@{optionId='no-native';kind='reject_once'})}}
    "#;
    let ending = if cancel {
        // Hold an exclusive lock until host process cleanup. No prompt result arrives.
        r#"[Console]::In.ReadToEnd() | Out-Null"#
    } else {
        r#"
        $answer = [Console]::ReadLine() | ConvertFrom-Json
        if ($answer.id -ne 'permission' -or $answer.result.outcome.optionId -ne 'yes-native') { throw 'Wrong approval' }
        Send @{jsonrpc='2.0';id=$prompt.id;result=@{stopReason='end_turn';usage=@{inputTokens=1;outputTokens=1;totalTokens=2}}}
        [Console]::In.ReadToEnd() | Out-Null
        "#
    };
    let script = format!("{script}\n{ending}");
    let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    ChatRuntime::start_adapter_gated(
        Box::new(PolicyFixture(adapter)),
        executable,
        Some(
            ["-NoProfile", "-NonInteractive", "-Command", &script]
                .map(String::from)
                .into(),
        ),
        cwd,
        Some(gate),
    )
}

#[test]
fn gemini_runtime_save_resume_approval_cancel_and_config_cleanup() {
    let _locale = crate::test_support::chinese();
    for cancel in [false, true] {
        let (release, gate) = mpsc::channel();
        let runtime = fixture(cancel, gate);
        assert!(matches!(
            runtime.events.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut resource = None;
        let mut approval_count = 0;
        let mut resolved = false;
        loop {
            assert!(Instant::now() < deadline, "Gemini fixture did not complete");
            match runtime.events.recv_timeout(Duration::from_secs(2)) {
                Ok(Event::NativeId(id)) => assert_eq!(id, "gemini-original"),
                Ok(Event::Delta { text, .. }) => {
                    assert_ne!(text, "old history");
                    let path = PathBuf::from(text);
                    assert!(path.is_file());
                    resource = Some(path);
                }
                Ok(Event::Approval(approval)) => {
                    approval_count += 1;
                    if cancel {
                        runtime.cancel();
                    } else {
                        runtime.decide(approval.id, true);
                    }
                }
                Ok(Event::ApprovalResolved { .. }) => resolved = true,
                Ok(Event::Finished { error, cancelled }) => {
                    assert_eq!(error, None);
                    assert_eq!(cancelled, cancel);
                    break;
                }
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("Missing terminal event: {e:?}"),
            }
        }
        assert_eq!(approval_count, 1);
        assert_eq!(
            resolved, !cancel,
            "cancellation expires the ticket without approving it"
        );
        assert!(!runtime.try_decide("stale-ticket".into(), true));
        let resource = resource.unwrap();
        assert!(!resource.exists());
        assert!(!resource.parent().unwrap().exists());
    }
}
