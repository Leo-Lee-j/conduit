use super::{RunState, Runtime, encode_key};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn wait(runtime: &Runtime, predicate: impl Fn(&RunState) -> bool) {
    let started = Instant::now();
    while !predicate(&runtime.state()) {
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "{:?}",
            runtime.state()
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn cmd(cwd: PathBuf) -> Runtime {
    Runtime::command(
        PathBuf::from(std::env::var_os("ComSpec").unwrap()),
        vec!["/d".into(), "/q".into()],
        cwd,
    )
}
#[test]
fn native_terminal_workspace_input_resize_and_exit() {
    let path = std::env::temp_dir().join(format!("conduit terminal 中文 & {}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let runtime = cmd(path.clone());
    wait(&runtime, |s| *s == RunState::Running);
    runtime.send(b"echo TERMINAL_READY\r".to_vec()).unwrap();
    let started = Instant::now();
    loop {
        if runtime
            .snapshot()
            .is_some_and(|s| s.text().contains("TERMINAL_READY"))
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10));
        thread::sleep(Duration::from_millis(10));
    }
    for cols in 80..150 {
        runtime.resize(cols, 35);
    }
    let started = Instant::now();
    while runtime.size() != (149, 35) {
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    runtime
        .send(b"echo cwd-ok>terminal-cwd.txt\rexit 7\r".to_vec())
        .unwrap();
    wait(&runtime, |s| *s == RunState::Exited(7));
    assert!(path.join("terminal-cwd.txt").is_file());
    std::fs::remove_file(path.join("terminal-cwd.txt")).unwrap();
    std::fs::remove_dir(path).unwrap();
}
#[test]
fn startup_cancel_and_invalid_directory_finish() {
    let runtime = cmd(std::env::temp_dir());
    runtime.stop();
    wait(&runtime, |s| !s.active());
    let runtime = cmd(std::env::temp_dir().join("conduit-terminal-missing-directory"));
    wait(&runtime, |s| matches!(s, RunState::Failed(_)));
}
#[test]
fn separate_sessions_keep_output_and_lifetime_independent() {
    let a = cmd(std::env::temp_dir());
    let b = cmd(std::env::temp_dir());
    wait(&a, |s| *s == RunState::Running);
    wait(&b, |s| *s == RunState::Running);
    a.stop();
    wait(&a, |s| !s.active());
    assert_eq!(b.state(), RunState::Running);
    b.stop();
    wait(&b, |s| !s.active());
}
#[test]
fn shell_controls_are_encoded_without_app_actions() {
    for (key, byte) in [
        ("c", 3),
        ("w", 23),
        ("e", 5),
        ("p", 16),
        ("k", 11),
        ("m", 13),
    ] {
        assert_eq!(encode_key(key, true, false, false, false), Some(vec![byte]));
    }
    assert_eq!(
        encode_key("up", false, false, false, true),
        Some(b"\x1bOA".to_vec())
    );
    assert_eq!(encode_key("tab", false, false, false, false), Some(vec![9]));
}

fn powershell(script: &str) -> Runtime {
    let (exe, _) = super::ShellProfile::PowerShell.resolve().unwrap();
    Runtime::command(
        exe,
        vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-Command".into(),
            script.into(),
        ],
        std::env::temp_dir(),
    )
}

#[test]
fn termination_unblocks_full_input_and_output_pipes() {
    let runtime = powershell(
        "[Console]::Write('FLOOD_READY'); while ($true) { [Console]::Write(('x' * 4096)) }",
    );
    wait(&runtime, |s| *s == RunState::Running);
    // The child never reads stdin. Reserve two full transactions and stop while
    // its writer may be blocked inside WriteFile and output is still flowing.
    runtime.send(vec![b'a'; 1024 * 1024]).unwrap();
    let _ = runtime.send(vec![b'b'; 1024 * 1024]);
    thread::sleep(Duration::from_millis(150));
    runtime.stop();
    wait(&runtime, |s| !s.active());
    assert!(
        matches!(runtime.state(), RunState::Exited(_)),
        "{:?}",
        runtime.state()
    );
    assert!(runtime.send(vec![b'x']).is_err());
}

#[test]
fn stopping_terminal_reaps_descendant_processes() {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        },
    };
    let runtime = powershell(
        "$p=Start-Process -FilePath $env:ComSpec -ArgumentList '/d','/q','/c','ping -n 120 127.0.0.1' -NoNewWindow -PassThru; [Console]::WriteLine(('DESCENDANT_ID=' + $p.Id + ';')); Start-Sleep -Seconds 120",
    );
    wait(&runtime, |s| *s == RunState::Running);
    let start = Instant::now();
    let pid = loop {
        let text = runtime.snapshot().map(|s| s.text()).unwrap_or_default();
        if let Some(pid) = text
            .split("DESCENDANT_ID=")
            .nth(1)
            .and_then(|s| s.split(';').next())
            .and_then(|s| s.parse::<u32>().ok())
        {
            break pid;
        }
        assert!(start.elapsed() < Duration::from_secs(15), "{text}");
        thread::sleep(Duration::from_millis(10));
    };
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    };
    assert!(!process.is_null());
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
        WAIT_TIMEOUT
    );
    runtime.stop();
    wait(&runtime, |s| !s.active());
    assert_eq!(
        unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
        WAIT_OBJECT_0
    );
}
