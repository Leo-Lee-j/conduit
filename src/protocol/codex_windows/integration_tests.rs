use super::codex_windows::tests::{Ace, Fixture, acl};
use super::{PermissionLevel, Protocol, Provider, ToolPolicy};
use crate::{
    child::{Child, Pipes},
    providers::RuntimeRoute,
};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

fn protocol(fixture: &Fixture, native: Option<String>) -> Protocol {
    Protocol::new(
        Provider::Codex,
        fixture.workspace.to_string_lossy().into_owned(),
        "fixture".into(),
        native,
    )
    .with_options(
        Default::default(),
        RuntimeRoute {
            env: fixture.environment(),
            ..Default::default()
        },
    )
}

#[test]
fn readonly_helpers_automation_and_other_providers_keep_existing_acls() {
    let fixture = Fixture::new();
    fixture.cache();
    fixture.protect_git();
    let git = fixture.workspace.join(".git");
    let before = acl(&git);
    let mut variants = [
        protocol(&fixture, None).with_permission(PermissionLevel::ReadOnly),
        protocol(&fixture, None).with_permission(PermissionLevel::FullAccess),
        protocol(&fixture, None).text_only(),
        protocol(&fixture, None).with_tool_policy(ToolPolicy::Automation),
        Protocol::new(
            Provider::Claude,
            fixture.workspace.to_string_lossy().into_owned(),
            "fixture".into(),
            None,
        ),
    ];
    for protocol in &mut variants {
        protocol.prepare().unwrap();
        assert_eq!(acl(&git), before);
    }
}

#[test]
fn workspace_prepare_reconciles_new_and_resumed_codex_sessions() {
    for native in [None, Some("resumed-session".into())] {
        let fixture = Fixture::new();
        fixture.cache();
        fixture.protect_git();
        let git = fixture.workspace.join(".git");
        let mut protocol = protocol(&fixture, native);
        assert!(
            acl(&git).iter().any(Ace::owned_write_deny),
            "construction must not change ACLs"
        );
        protocol.prepare().unwrap();
        assert!(!acl(&git).iter().any(Ace::owned_write_deny));
    }
}

struct Rpc {
    child: Child,
    input: File,
    events: mpsc::Receiver<Value>,
}
impl Rpc {
    fn start(executable: &Path, protocol: &Protocol, log: &Path) -> Self {
        let (program, args) = crate::cli::command_for(executable, &protocol.args());
        let (
            child,
            Pipes {
                stdin,
                stdout,
                mut stderr,
            },
        ) = Child::spawn_with_env(
            &program,
            &args,
            Path::new(&protocol.cwd),
            protocol.environment(),
        )
        .unwrap();
        let (sender, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str(&line)
                    && sender.send(value).is_err()
                {
                    break;
                }
            }
        });
        let mut log = File::create(log).unwrap();
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut stderr, &mut log);
        });
        let mut rpc = Self {
            child,
            input: stdin,
            events,
        };
        rpc.send(&protocol.initialize());
        let initialized = rpc.response(1);
        assert!(initialized.get("error").is_none(), "{initialized}");
        rpc.send(&json!({"method":"initialized"}));
        rpc
    }
    fn send(&mut self, value: &Value) {
        serde_json::to_writer(&mut self.input, value).unwrap();
        self.input.write_all(b"\n").unwrap();
        self.input.flush().unwrap();
    }
    fn response(&self, id: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let value = self
                .events
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
            if value["id"] == id {
                return value;
            }
        }
    }
    fn exec(
        &mut self,
        id: u64,
        fixture: &Fixture,
        script: &Path,
        args: &[String],
        policy: Value,
    ) -> Value {
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = vec![
            shell.to_string_lossy().into_owned(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-File".into(),
            script.to_string_lossy().into_owned(),
        ];
        command.extend_from_slice(args);
        self.send(&json!({"id":id,"method":"command/exec","params":{
            "command":command,"cwd":fixture.workspace,"sandboxPolicy":policy,"timeoutMs":35000,
        }}));
        self.response(id)
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.terminate();
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.active_processes().unwrap_or(0) != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn assert_denied(response: &Value, target: &str) {
    assert!(
        response.get("error").is_some()
            || response["result"]["exitCode"]
                .as_i64()
                .is_some_and(|code| code != 0),
        "{response}"
    );
    let message = response["error"]["message"]
        .as_str()
        .or_else(|| response["result"]["stderr"].as_str())
        .unwrap();
    let lower = message.to_ascii_lowercase();
    assert!(
        lower.contains("denied")
            || lower.contains("unauthorizedaccess")
            || message.contains("拒绝"),
        "{response}"
    );
    assert!(message.contains(target), "{response}");
}

#[test]
#[ignore = "requires installed Codex and Windows sandbox setup; command/exec only, no model request"]
fn installed_codex_workspace_transition_can_add_and_commit() {
    let _locale = crate::test_support::chinese();
    // Codex refuses to provision helper binaries when CODEX_HOME is in TEMP.
    let fixture = Fixture::in_directory(&Path::new(env!("CARGO_MANIFEST_DIR")).join("target"));
    let executable = Provider::Codex.executable().expect("Codex not installed");
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(&fixture.workspace)
        .output()
        .unwrap();
    assert!(
        git.status.success(),
        "{}",
        String::from_utf8_lossy(&git.stderr)
    );
    fs::write(fixture.workspace.join("sample.txt"), "fixture\n").unwrap();
    let script = fixture.workspace.join("git-probe.ps1");
    fs::write(&script, "git add -- sample.txt\nif ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }\ngit -c core.hooksPath= -c user.name=ConduitProbe -c user.email=probe@localhost -c commit.gpgsign=false commit -m 'workspace permission probe'\nexit $LASTEXITCODE\n").unwrap();
    let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let args = vec![
        "sandbox".into(),
        "-P".into(),
        ":workspace".into(),
        "-C".into(),
        fixture.workspace.to_string_lossy().into_owned(),
        "-c".into(),
        "windows.sandbox=\"elevated\"".into(),
        "--".into(),
        shell.to_string_lossy().into_owned(),
        "-NoProfile".into(),
        "-NonInteractive".into(),
        "-Command".into(),
        "git status --short; exit $LASTEXITCODE".into(),
    ];
    // The old builtin also grants TEMP; keep that ACL propagation in the fixture.
    let temporary = fixture.root.join("temp");
    fs::create_dir(&temporary).unwrap();
    let mut bootstrap_environment = fixture.environment();
    for key in ["TEMP", "TMP"] {
        bootstrap_environment.insert(key.into(), temporary.to_string_lossy().into_owned());
    }
    crate::cli::capture_command(
        &executable,
        &args,
        &fixture.workspace,
        &bootstrap_environment,
        &AtomicBool::new(false),
        Duration::from_secs(45),
        64 * 1024,
    )
    .unwrap();
    let mut protocol = protocol(&fixture, None);
    protocol
        .route
        .config
        .insert("windows.sandbox".into(), json!("elevated"));
    let policy = json!({"type":"workspaceWrite",
        "writableRoots":protocol.codex_workspace_writable_roots(),"networkAccess":true,
        "excludeTmpdirEnvVar":true,"excludeSlashTmp":true});
    let mut before = Rpc::start(&executable, &protocol, &fixture.root.join("before.log"));
    let blocked = before.exec(2, &fixture, &script, &[], policy.clone());
    assert_denied(&blocked, "index.lock");
    drop(before);
    protocol.prepare().unwrap();
    let mut after = Rpc::start(&executable, &protocol, &fixture.root.join("after.log"));
    let committed = after.exec(2, &fixture, &script, &[], policy.clone());
    assert!(committed.get("error").is_none(), "{committed}");
    assert_eq!(committed["result"]["exitCode"], 0, "{committed}");
    assert!(
        committed["result"]["stdout"]
            .as_str()
            .unwrap()
            .contains("workspace permission probe"),
        "{committed}"
    );
    let outside = fixture.root.join("outside.txt");
    fs::write(&outside, "preserved").unwrap();
    let write = fixture.workspace.join("write-probe.ps1");
    fs::write(&write, "param($Destination)\n$ErrorActionPreference = 'Stop'\nSet-Content -LiteralPath $Destination -Value 'unexpected'\n").unwrap();
    let denied = after.exec(
        3,
        &fixture,
        &write,
        &[outside.to_string_lossy().into_owned()],
        policy,
    );
    assert_denied(&denied, "outside.txt");
    assert_eq!(fs::read_to_string(&outside).unwrap(), "preserved");
    let denied = after.exec(4, &fixture, &script, &[], json!({"type":"readOnly"}));
    assert_denied(&denied, "index.lock");
    assert!(!fixture.workspace.join(".git/index.lock").exists());
}
