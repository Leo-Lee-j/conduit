//! Opt-in native Claude file-tool validation against an isolated local model.
//! The default test suite replays the sanitized events in protocol::file_proposals.
use std::{path::PathBuf, process::Command};

#[test]
#[ignore = "requires Python and CONDUIT_CLAUDE_EXE; uses an isolated local model stub"]
fn installed_claude_file_tool_stream() {
    let binary = PathBuf::from(
        std::env::var_os("CONDUIT_CLAUDE_EXE")
            .expect("set CONDUIT_CLAUDE_EXE to the native Claude executable"),
    );
    assert!(
        binary.is_absolute() && binary.is_file(),
        "expected an absolute native CLI path"
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = root
        .join("target")
        .join(format!("claude-file-stream-test-{}", std::process::id()));
    let result = Command::new("python")
        .arg(root.join("scripts/probe-file-tool-stream.py"))
        .arg("--claude")
        .arg(binary)
        .arg("--output")
        .arg(output)
        .current_dir(root)
        .output()
        .expect("start the local native protocol probe");
    assert!(
        result.status.success(),
        "native protocol probe failed: {}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
