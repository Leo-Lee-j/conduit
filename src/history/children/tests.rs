use super::*;
use crate::domain::ItemKind;
use serde_json::json;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "conduit-child-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("workspace")).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn cwd(&self) -> PathBuf {
        self.0.join("workspace")
    }
    fn binding(&self, provider: Provider) -> ChildBinding {
        ChildBinding {
            root: 1,
            root_native: "root".into(),
            native: "child".into(),
            parent: "root".into(),
            source: self.0.join(if provider == Provider::Claude {
                "claude"
            } else {
                "opencode.db"
            }),
            idle: false,
        }
    }
    fn database(&self) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(self.0.join("opencode.db")).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL;
            CREATE TABLE session(id TEXT PRIMARY KEY, directory TEXT, parent_id TEXT, time_created INTEGER);
            CREATE TABLE message(id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, data TEXT);
            CREATE TABLE part(id TEXT PRIMARY KEY, session_id TEXT, message_id TEXT, time_created INTEGER, data TEXT);").unwrap();
        for (id, parent) in [("root", None), ("child", Some("root")), ("foreign", None)] {
            c.execute(
                "INSERT INTO session VALUES (?1,?2,?3,1000)",
                rusqlite::params![id, self.cwd().to_string_lossy(), parent],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO message VALUES ('user','child',1,?1)",
            [json!({"role":"user"}).to_string()],
        )
        .unwrap();
        c.execute(
            "INSERT INTO message VALUES ('assistant','child',2,?1)",
            [json!({"role":"assistant","finish":"stop","time":{"completed":3000}}).to_string()],
        )
        .unwrap();
        for i in 0..300 {
            c.execute(
                "INSERT INTO part VALUES (?1,'child',?2,1,?3)",
                rusqlite::params![
                    format!("part-{i:04}"),
                    if i == 0 { "user" } else { "assistant" },
                    json!({"type":"text","text":format!("message {i}")}).to_string()
                ],
            )
            .unwrap();
        }
        c
    }
    fn claude(&self) -> PathBuf {
        let project = self.0.join("claude/projects/project");
        let dir = project.join("root/subagents/nested");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            project.join("root.jsonl"),
            json!({"sessionId":"root","cwd":self.cwd(),"type":"user"}).to_string() + "\n",
        )
        .unwrap();
        let entries = [
            json!({"uuid":"u","parentUuid":null,"type":"user","message":{"content":"do the work"}}),
            json!({"uuid":"a","parentUuid":"u","type":"assistant","message":{"content":[{"type":"thinking","thinking":"public reasoning"},{"type":"tool_use","id":"tool","name":"Read","input":{"file_path":"README.md"}}]}}),
            json!({"uuid":"t","parentUuid":"a","type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool","content":[{"type":"text","text":"file contents"},{"type":"image","source":{"data":"do not expose"}}]}]}}),
            json!({"uuid":"old","parentUuid":"t","type":"assistant","message":{"content":[{"type":"text","text":"abandoned branch"}]}}),
            json!({"uuid":"final","parentUuid":"t","type":"assistant","message":{"content":[{"type":"text","text":"finished"}]}}),
        ];
        let text = entries
            .into_iter()
            .map(|mut v| {
                v["agentId"] = json!("child");
                v["sessionId"] = json!("root");
                v["isSidechain"] = json!(true);
                v["cwd"] = json!(self.cwd());
                v.to_string() + "\n"
            })
            .collect::<String>();
        let path = dir.join("agent-child.jsonl");
        std::fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opencode_reads_entire_child_without_importing_siblings_or_writing_source() {
    let f = Fixture::new();
    let connection = f.database();
    let before: i64 = connection
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    let Snapshot::Native {
        messages,
        idle,
        created_at,
    } = read(
        Provider::OpenCode,
        &f.cwd(),
        &f.binding(Provider::OpenCode),
        &AtomicBool::new(false),
    )
    .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 300);
    assert!(idle);
    assert_eq!(created_at, 1);
    assert_eq!(messages[0].kind, ItemKind::User);
    assert_eq!(messages.last().unwrap().text, "message 299");
    assert!(messages.iter().all(|v| v.key.starts_with("native-")));
    assert_eq!(
        before,
        connection
            .query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap()
    );
    connection
        .execute(
            "UPDATE message SET data = ?1 WHERE id='assistant'",
            [
                json!({"role":"assistant","finish":"tool-calls","time":{"completed":3000}})
                    .to_string(),
            ],
        )
        .unwrap();
    assert!(matches!(
        read(
            Provider::OpenCode,
            &f.cwd(),
            &f.binding(Provider::OpenCode),
            &AtomicBool::new(false)
        )
        .unwrap(),
        Snapshot::Native { idle: false, .. }
    ));
}

#[test]
fn opencode_rejects_foreign_parent_workspace_cycles_corruption_and_cancellation() {
    let f = Fixture::new();
    let c = f.database();
    let binding = f.binding(Provider::OpenCode);
    let mut wrong = binding.clone();
    wrong.root_native = "foreign".into();
    assert!(
        read(
            Provider::OpenCode,
            &f.cwd(),
            &wrong,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(read(Provider::OpenCode, &f.0, &binding, &AtomicBool::new(false)).is_err());
    wrong = binding.clone();
    wrong.parent = "foreign".into();
    assert!(
        read(
            Provider::OpenCode,
            &f.cwd(),
            &wrong,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert!(
        read(
            Provider::OpenCode,
            &f.cwd(),
            &binding,
            &AtomicBool::new(true)
        )
        .is_err()
    );
    c.execute("UPDATE part SET data='{' WHERE id='part-0299'", [])
        .unwrap();
    assert!(
        read(
            Provider::OpenCode,
            &f.cwd(),
            &binding,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    c.execute("UPDATE session SET parent_id='child' WHERE id='root'", [])
        .unwrap();
    wrong = binding.clone();
    wrong.root_native = "foreign".into();
    assert!(
        read(
            Provider::OpenCode,
            &f.cwd(),
            &wrong,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}

#[test]
fn claude_reconstructs_branch_and_tool_results_without_treating_agent_as_session() {
    let f = Fixture::new();
    let path = f.claude();
    let before = std::fs::read(&path).unwrap();
    let Snapshot::Native { messages, idle, .. } = read(
        Provider::Claude,
        &f.cwd(),
        &f.binding(Provider::Claude),
        &AtomicBool::new(false),
    )
    .unwrap() else {
        panic!()
    };
    assert!(!idle);
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].text, "do the work");
    assert_eq!(messages[1].kind, ItemKind::Reasoning);
    assert_eq!(messages[2].detail, "file contents");
    assert_eq!(
        messages[2].status,
        crate::message_status::ToolStatus::Completed
    );
    assert_eq!(messages[3].text, "finished");
    assert_eq!(before, std::fs::read(path).unwrap());
}

#[test]
fn claude_rejects_mismatched_identity_nested_parent_and_broken_chains() {
    let f = Fixture::new();
    let path = f.claude();
    let binding = f.binding(Provider::Claude);
    assert!(read(Provider::Claude, &f.0, &binding, &AtomicBool::new(false)).is_err());
    let mut bad = binding.clone();
    bad.native = "../child".into();
    assert!(read(Provider::Claude, &f.cwd(), &bad, &AtomicBool::new(false)).is_err());
    bad = binding.clone();
    bad.parent = "other-agent".into();
    assert!(read(Provider::Claude, &f.cwd(), &bad, &AtomicBool::new(false)).is_err());
    let original = std::fs::read_to_string(&path).unwrap();
    for replacement in [
        original.replace("\"agentId\":\"child\"", "\"agentId\":\"foreign\""),
        original.replace("\"parentUuid\":\"t\"", "\"parentUuid\":\"missing\""),
        original.replace("\"parentUuid\":\"t\"", "\"parentUuid\":\"final\""),
    ] {
        std::fs::write(&path, replacement).unwrap();
        assert!(
            read(
                Provider::Claude,
                &f.cwd(),
                &binding,
                &AtomicBool::new(false)
            )
            .is_err()
        );
    }
}

#[test]
fn sources_respect_engine_specific_overrides_and_relative_workspace_paths() {
    let f = Fixture::new();
    let mut env = BTreeMap::from([
        ("CODEX_HOME".into(), "codex".into()),
        ("CLAUDE_CONFIG_DIR".into(), "claude".into()),
        ("XDG_DATA_HOME".into(), "data".into()),
    ]);
    assert_eq!(
        source(Provider::Codex, &env, &f.cwd()),
        Some(f.cwd().join("codex"))
    );
    assert_eq!(
        source(Provider::Claude, &env, &f.cwd()),
        Some(f.cwd().join("claude"))
    );
    assert_eq!(
        source(Provider::OpenCode, &env, &f.cwd()),
        Some(f.cwd().join("data/opencode/opencode.db"))
    );
    assert_eq!(source(Provider::Qoder, &env, &f.cwd()), None);
    env.insert("OPENCODE_DB".into(), "custom.sqlite".into());
    assert_eq!(
        source(Provider::OpenCode, &env, &f.cwd()),
        Some(f.cwd().join("custom.sqlite"))
    );
}

#[test]
fn claude_accepts_inflight_tail_and_verified_nested_parent_but_rejects_corruption() {
    let f = Fixture::new();
    let path = f.claude();
    let mut binding = f.binding(Provider::Claude);
    let original = std::fs::read_to_string(&path).unwrap();
    let read_messages =
        |binding: &ChildBinding| read(Provider::Claude, &f.cwd(), binding, &AtomicBool::new(false));
    std::fs::write(&path, format!("{original}{{\"uuid\":")).unwrap();
    let Snapshot::Native { messages, .. } = read_messages(&binding).unwrap() else {
        panic!()
    };
    assert_eq!(messages.last().unwrap().text, "finished");
    std::fs::write(&path, format!("{original}{{\"uuid\":\n")).unwrap();
    assert!(read_messages(&binding).is_err());
    std::fs::write(&path, &original).unwrap();
    std::fs::write(
        path.with_file_name("agent-child.meta.json"),
        json!({"parentAgentId":"parent-agent"}).to_string(),
    )
    .unwrap();
    assert!(read_messages(&binding).is_err());
    binding.parent = "parent-agent".into();
    assert!(read_messages(&binding).is_ok());
    std::fs::write(
        &path,
        json!({"type":"progress","agentId":"child","sessionId":"root"}).to_string(),
    )
    .unwrap();
    assert!(
        read_messages(&binding).is_err(),
        "metadata alone is not conversation history"
    );
}

#[test]
fn claude_read_budget_covers_parent_and_child_together() {
    let f = Fixture::new();
    let path = f.claude();
    // Preserve valid parent JSONL; blank lines still consume source bytes.
    let parent = f.0.join("claude/projects/project/root.jsonl");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(parent)
        .unwrap();
    use std::io::Write;
    for _ in 0..33 {
        file.write_all(&vec![b' '; 1024 * 1024]).unwrap();
        file.write_all(b"\n").unwrap();
    }
    drop(file);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    let error = read(
        Provider::Claude,
        &f.cwd(),
        &f.binding(Provider::Claude),
        &AtomicBool::new(false),
    )
    .err()
    .expect("combined source bytes must exceed budget");
    assert!(error.to_string().contains("read budget"), "{error}");
}
