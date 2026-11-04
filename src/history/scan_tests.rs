use super::*;
use std::io::Write;

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    index: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "conduit-history-tests-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        Self {
            workspace: root.join("workspace"),
            index: root.join("history.sqlite3"),
            root,
        }
    }
    fn file(&self, provider: Provider, name: &str) -> PathBuf {
        let directory = self.root.join(match provider {
            Provider::Claude => ".claude/projects/project",
            _ => ".codex/sessions/2026/11/04",
        });
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(format!("{name}.jsonl"))
    }
    fn scanner(&self) -> Scanner {
        Scanner::open(
            Some(&self.root),
            vec![self.workspace.clone()],
            &self.index,
            &AtomicBool::new(false),
        )
    }
    fn scan(&self) -> Batch {
        let mut scanner = self.scanner();
        let cancel = AtomicBool::new(false);
        let mut batch = scanner.next(&cancel);
        while !batch.done {
            let next = scanner.next(&cancel);
            batch.sessions[0].extend(next.sessions.into_iter().next().unwrap());
            batch.warnings.extend(next.warnings);
            batch.bytes_read += next.bytes_read;
            batch.verified_bytes += next.verified_bytes;
            batch.cache_hits += next.cache_hits;
            batch.completed = next.completed;
            batch.done = next.done;
        }
        batch
    }
    fn claude(&self, id: &str, text: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({"type":"user", "sessionId":id, "cwd":self.workspace, "promptSource":"sdk", "message":{"content":text}})
        )
    }
    fn codex_meta(&self) -> String {
        format!(
            "{}\n",
            serde_json::json!({"type":"session_meta", "payload":{"id":"native-codex", "cwd":self.workspace, "timestamp":"2026-11-04T00:00:00Z"}})
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        assert_eq!(
            self.root.parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        assert!(
            self.root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("conduit-history-tests-")
        );
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn append(path: &Path, text: &str) {
    File::options()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}
fn messages(batch: &Batch) -> Value {
    serde_json::to_value(&batch.sessions[0][0].messages).unwrap()
}

#[test]
fn persistent_cache_and_append_match_a_fresh_claude_parse() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "native");
    let first = fixture.claude("native", "first");
    std::fs::write(&path, &first).unwrap();
    let cold = fixture.scan();
    assert!(cold.warnings.is_empty(), "{:?}", cold.warnings);
    assert_eq!(cold.bytes_read, first.len() as u64);
    let warm = fixture.scan();
    assert_eq!(warm.bytes_read, 0);
    assert_eq!(warm.verified_bytes, 0);
    assert_eq!(warm.cache_hits, 1);
    assert_eq!(messages(&cold), messages(&warm));
    let suffix = fixture.claude("native", "second");
    append(&path, &suffix);
    let incremental = fixture.scan();
    assert_eq!(incremental.bytes_read, suffix.len() as u64);
    assert_eq!(incremental.verified_bytes, first.len() as u64);
    let expected = parse_claude_lines((first + &suffix).lines().map(str::to_owned))
        .unwrap()
        .1;
    assert_eq!(
        messages(&incremental),
        serde_json::to_value(expected.messages).unwrap()
    );
}

#[test]
fn codex_incremental_state_preserves_fallback_dedup_and_turn_timing() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Codex, "native");
    let first = fixture.codex_meta() + &[
        serde_json::json!({"type":"turn_context","payload":{"model":"model-a"}}),
        serde_json::json!({"type":"event_msg","timestamp":"2026-11-04T00:00:00Z","payload":{"type":"task_started","turn_id":"turn"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"question"}]}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}}),
    ].iter().map(|value| format!("{value}\n")).collect::<String>();
    std::fs::write(&path, &first).unwrap();
    assert_eq!(fixture.scan().sessions[0][0].messages.len(), 2);
    let suffix = [
        serde_json::json!({"type":"event_msg","payload":{"type":"item_completed","item":{"type":"AgentMessage","id":"answer","text":"answer"}}}),
        serde_json::json!({"type":"event_msg","timestamp":"2026-11-04T00:00:02Z","payload":{"type":"task_complete","turn_id":"turn"}}),
    ].iter().map(|value| format!("{value}\n")).collect::<String>();
    append(&path, &suffix);
    let batch = fixture.scan();
    assert_eq!(batch.bytes_read, suffix.len() as u64);
    let expected = parse_codex_lines((first + &suffix).lines().map(str::to_owned), None)
        .unwrap()
        .1;
    assert_eq!(
        messages(&batch),
        serde_json::to_value(expected.messages).unwrap()
    );
    assert_eq!(batch.sessions[0][0].messages.len(), 2);
    assert_eq!(batch.sessions[0][0].messages[1].elapsed_ms, Some(2000));
}

#[test]
fn incomplete_tail_is_replayed_without_duplicates_after_append() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "tail");
    let first = fixture.claude("tail", "first");
    let suffix = fixture.claude("tail", "中文 continuation");
    let split = suffix.find("中文").unwrap() + 1;
    let mut initial = first.as_bytes().to_vec();
    initial.extend_from_slice(&suffix.as_bytes()[..split]);
    std::fs::write(&path, initial).unwrap();
    let batch = fixture.scan();
    assert!(batch.warnings.is_empty());
    assert_eq!(batch.sessions[0][0].messages.len(), 1);
    File::options()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(&suffix.as_bytes()[split..])
        .unwrap();
    let batch = fixture.scan();
    assert_eq!(batch.bytes_read, suffix.len() as u64);
    assert_eq!(batch.sessions[0][0].messages.len(), 2);
    assert_eq!(batch.sessions[0][0].messages[1].text, "中文 continuation");
}

#[test]
fn truncation_growing_rewrite_version_change_and_corrupt_state_rebuild() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "rewrite");
    std::fs::write(&path, fixture.claude("rewrite", "original long text")).unwrap();
    fixture.scan();
    std::fs::write(&path, fixture.claude("rewrite", "short")).unwrap();
    let shortened = fixture.scan();
    assert_eq!(shortened.sessions[0][0].messages[0].text, "short");
    let longer = fixture.claude(
        "rewrite",
        "a completely different and much longer replacement",
    );
    std::fs::write(&path, &longer).unwrap();
    let rewritten = fixture.scan();
    assert_eq!(rewritten.bytes_read, longer.len() as u64);
    assert_eq!(rewritten.sessions[0][0].messages.len(), 1);
    let connection = rusqlite::Connection::open(&fixture.index).unwrap();
    connection
        .execute("UPDATE history_files SET state=x'ff'", [])
        .unwrap();
    assert_eq!(fixture.scan().bytes_read, longer.len() as u64);
    let mut metadata = Index::open(&fixture.index)
        .unwrap()
        .metadata(&path)
        .unwrap();
    metadata.version += 1;
    connection
        .execute(
            "UPDATE history_files SET metadata=?1",
            [serde_json::to_string(&metadata).unwrap()],
        )
        .unwrap();
    assert_eq!(fixture.scan().bytes_read, longer.len() as u64);
}

#[test]
fn workspace_index_filters_foreign_sessions_but_allows_later_workspace_import() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Codex, "foreign");
    let text = fixture.codex_meta()
        + &format!(
            "{}\n",
            serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}})
        );
    std::fs::write(path, text).unwrap();
    let cancel = AtomicBool::new(false);
    let mut other = Scanner::open(
        Some(&fixture.root),
        vec![fixture.root.join("other")],
        &fixture.index,
        &cancel,
    );
    assert!(other.next(&cancel).sessions[0].is_empty());
    drop(other);
    let mut other = Scanner::open(
        Some(&fixture.root),
        vec![fixture.root.join("other")],
        &fixture.index,
        &cancel,
    );
    assert_eq!(other.next(&cancel).total, 0);
    drop(other);
    assert_eq!(fixture.scan().sessions[0][0].messages[0].text, "hello");
}

#[test]
fn batches_resume_until_every_file_is_imported_and_cancellation_is_safe() {
    let fixture = Fixture::new();
    for number in 0..(BATCH_FILES + 4) {
        let name = format!("session-{number}");
        std::fs::write(
            fixture.file(Provider::Claude, &name),
            fixture.claude(&name, "hello"),
        )
        .unwrap();
    }
    let mut scanner = fixture.scanner();
    let cancel = AtomicBool::new(false);
    let first = scanner.next(&cancel);
    assert!(!first.done);
    assert!(first.completed <= BATCH_FILES);
    let mut count = first.sessions[0].len();
    loop {
        let batch = scanner.next(&cancel);
        count += batch.sessions[0].len();
        assert!(batch.warnings.is_empty());
        if batch.done {
            assert_eq!(batch.completed, batch.total);
            break;
        }
    }
    assert_eq!(count, BATCH_FILES + 4);
    cancel.store(true, Ordering::Release);
    let batch = scanner.next(&cancel);
    assert!(batch.sessions[0].is_empty());
    assert_eq!(fixture.scan().cache_hits, BATCH_FILES + 4);
}

#[test]
fn large_files_stream_while_oversized_lines_preserve_previous_checkpoint() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "large");
    let mut file = std::io::BufWriter::new(File::create(&path).unwrap());
    file.write_all(fixture.claude("large", "hello").as_bytes())
        .unwrap();
    let padding = format!("{{\"padding\":\"{}\"}}\n", "x".repeat(4096));
    for _ in 0..8300 {
        file.write_all(padding.as_bytes()).unwrap();
    }
    file.flush().unwrap();
    drop(file);
    let batch = fixture.scan();
    assert!(batch.bytes_read > 32 * 1024 * 1024);
    assert!(batch.warnings.is_empty(), "{:?}", batch.warnings);
    assert_eq!(batch.sessions[0][0].messages[0].text, "hello");
    let before = Index::open(&fixture.index)
        .unwrap()
        .metadata(&path)
        .unwrap()
        .offset;
    append(&path, &"x".repeat(MAX_LINE + 1));
    let batch = fixture.scan();
    assert!(batch.sessions[0].is_empty());
    assert!(
        batch
            .warnings
            .iter()
            .any(|warning| warning.contains("单行"))
    );
    assert_eq!(
        Index::open(&fixture.index)
            .unwrap()
            .metadata(&path)
            .unwrap()
            .offset,
        before
    );
}

#[test]
fn unavailable_index_falls_back_to_streaming() {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.file(Provider::Claude, "fallback"),
        fixture.claude("fallback", "hello"),
    )
    .unwrap();
    std::fs::create_dir(&fixture.index).unwrap();
    let batch = fixture.scan();
    assert_eq!(batch.sessions[0][0].messages[0].text, "hello");
    assert!(
        batch
            .warnings
            .iter()
            .any(|warning| warning.contains("索引不可用"))
    );
}

#[test]
fn a_middle_rewrite_followed_by_append_invalidates_the_old_parser() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "middle");
    let padding = format!("{{\"padding\":\"{}\"}}\n", "x".repeat(70 * 1024));
    let original = padding.clone() + &fixture.claude("middle", "original") + &padding;
    std::fs::write(&path, &original).unwrap();
    fixture.scan();
    let replacement =
        original.replace("original", "modified") + &fixture.claude("middle", "appended");
    std::fs::write(&path, &replacement).unwrap();
    let batch = fixture.scan();
    assert_eq!(batch.verified_bytes, original.len() as u64);
    assert_eq!(batch.bytes_read, replacement.len() as u64);
    assert_eq!(batch.sessions[0][0].messages[0].text, "modified");
    assert_eq!(batch.sessions[0][0].messages[1].text, "appended");
}

#[test]
fn valid_json_without_a_final_newline_is_imported_and_rebuilt_on_append() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "unterminated");
    let initial = fixture
        .claude("unterminated", "first")
        .trim_end()
        .to_owned();
    std::fs::write(&path, &initial).unwrap();
    let cold = fixture.scan();
    assert_eq!(cold.sessions[0][0].messages.len(), 1);
    assert!(
        !Index::open(&fixture.index)
            .unwrap()
            .metadata(&path)
            .unwrap()
            .terminated
    );
    assert_eq!(messages(&cold), messages(&fixture.scan()));
    let suffix = "\n".to_owned() + &fixture.claude("unterminated", "second");
    append(&path, &suffix);
    let batch = fixture.scan();
    assert_eq!(batch.bytes_read, (initial.len() + suffix.len()) as u64);
    assert_eq!(batch.verified_bytes, 0);
    assert_eq!(batch.sessions[0][0].messages.len(), 2);
}

#[test]
fn cache_state_must_match_the_metadata_from_discovery() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "concurrent");
    std::fs::write(&path, fixture.claude("concurrent", "first")).unwrap();
    fixture.scan();
    let stale = Index::open(&fixture.index)
        .unwrap()
        .metadata(&path)
        .unwrap();
    let mut scanner = fixture.scanner();
    append(&path, &fixture.claude("concurrent", "second"));
    let fresh = fixture.scan();
    assert!(
        Index::open(&fixture.index)
            .unwrap()
            .state(&path, &stale)
            .is_none()
    );
    let batch = scanner.next(&AtomicBool::new(false));
    assert_eq!(messages(&batch), messages(&fresh));
    assert_eq!(batch.sessions[0][0].messages.len(), 2);
}

#[test]
fn a_single_large_file_resumes_verification_and_parsing_between_batches() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "resumable");
    let mut file = std::io::BufWriter::new(File::create(&path).unwrap());
    file.write_all(fixture.claude("resumable", "first").as_bytes())
        .unwrap();
    let padding = format!("{{\"padding\":\"{}\"}}\n", "x".repeat(4096));
    for _ in 0..8300 {
        file.write_all(padding.as_bytes()).unwrap();
    }
    file.flush().unwrap();
    drop(file);
    let cancel = AtomicBool::new(false);
    let mut scanner = fixture.scanner();
    let first = scanner.next(&cancel);
    assert!(!first.done);
    assert_eq!(first.completed, 0);
    assert!(first.bytes_read <= BATCH_BYTES + padding.len() as u64);
    assert!(first.sessions[0].is_empty());
    assert!(
        Index::open(&fixture.index)
            .unwrap()
            .metadata(&path)
            .is_none()
    );
    loop {
        let batch = scanner.next(&cancel);
        if batch.done {
            assert_eq!(batch.sessions[0][0].messages.len(), 1);
            break;
        }
        assert!(batch.sessions[0].is_empty());
    }
    let previous_size = std::fs::metadata(&path).unwrap().len();
    let suffix = fixture.claude("resumable", "second");
    append(&path, &suffix);
    let mut scanner = fixture.scanner();
    let first = scanner.next(&cancel);
    assert!(!first.done);
    assert_eq!(first.bytes_read, 0);
    assert!(first.verified_bytes > 0 && first.verified_bytes <= BATCH_BYTES);
    let mut verified = first.verified_bytes;
    let mut parsed = 0;
    loop {
        let batch = scanner.next(&cancel);
        verified += batch.verified_bytes;
        parsed += batch.bytes_read;
        if batch.done {
            assert_eq!(batch.sessions[0][0].messages.len(), 2);
            break;
        }
    }
    assert_eq!(verified, previous_size);
    assert_eq!(parsed, suffix.len() as u64);
}

#[test]
fn a_file_changed_between_batches_is_retried_without_publishing_partial_messages() {
    let fixture = Fixture::new();
    let path = fixture.file(Provider::Claude, "changing");
    let padding = format!("{{\"padding\":\"{}\"}}\n", "x".repeat(4096));
    let initial = fixture.claude("changing", "old") + &padding.repeat(4300);
    std::fs::write(&path, initial).unwrap();
    let mut scanner = fixture.scanner();
    let cancel = AtomicBool::new(false);
    assert!(!scanner.next(&cancel).done);
    std::fs::write(&path, fixture.claude("changing", "new")).unwrap();
    let retry = scanner.next(&cancel);
    assert!(!retry.done);
    assert!(retry.sessions[0].is_empty());
    let result = scanner.next(&cancel);
    assert!(result.done);
    assert!(result.warnings.is_empty());
    assert_eq!(result.sessions[0][0].messages[0].text, "new");
}
