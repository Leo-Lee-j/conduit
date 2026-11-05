use super::*;
use crate::{domain::Workspace, store::Store};

pub(crate) struct Fixture {
    pub root: PathBuf,
    pub state: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "conduit-transcript-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            state: root.join("state.json"),
            root,
        }
    }
    pub fn store(&self, count: usize) -> Store {
        Store {
            version: 2,
            workspaces: vec![Workspace {
                path: self.root.clone(),
                sessions: (1..=count)
                    .map(|id| {
                        let mut session: SessionRecord =
                            serde_json::from_value(serde_json::json!({
                                "id": id, "provider": "Claude", "created_at": 1, "exit_code": null
                            }))
                            .unwrap();
                        session.id = id as u64;
                        session.native_id = Some(format!("native-{id}"));
                        session.messages = vec![
                            message("user", "original prompt", 10),
                            message("reply", "original reply", 20),
                        ];
                        session
                    })
                    .collect(),
            }],
            ..Default::default()
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(directory(&self.state)) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
            let _ = fs::remove_dir(directory(&self.state));
        }
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.root);
    }
}
pub(crate) fn message(key: &str, text: &str, created_at: u64) -> ChatItem {
    serde_json::from_value(serde_json::json!({
        "key": key, "text": text, "created_at": created_at,
        "kind": if key == "user" { "User" } else { "Assistant" }
    }))
    .unwrap()
}

#[test]
fn migration_backs_up_legacy_state_and_starts_with_summaries() {
    for version in [1, 2] {
        let fixture = Fixture::new();
        let mut original = fixture.store(3);
        original.version = version;
        let bytes = serde_json::to_vec(&original).unwrap();
        fs::write(&fixture.state, &bytes).unwrap();
        let state = Store::load_migrating_from(&fixture.state).unwrap();
        assert_eq!(state.version, 4);
        assert_eq!(
            fs::read(fixture.state.with_extension("v2.json")).unwrap(),
            bytes
        );
        assert!(
            !fs::read_to_string(&fixture.state)
                .unwrap()
                .contains("original reply")
        );
        for session in &state.workspaces[0].sessions {
            assert!(!session.transcript.loaded);
            assert!(session.messages.is_empty());
            assert!(session.has_user_messages());
            assert_eq!(session.last_activity_at(), 20);
            let loaded = load(session.clone()).unwrap();
            assert_eq!(loaded.messages[1].text, "original reply");
            assert_eq!(loaded.messages.len(), if version == 1 { 3 } else { 2 });
        }
        let again = Store::load_migrating_from(&fixture.state).unwrap();
        assert_eq!(
            again.workspaces[0].sessions[0].message_count(),
            state.workspaces[0].sessions[0].message_count()
        );
    }
}

#[test]
fn unloaded_save_preserves_body_and_attachment_reachability() {
    let fixture = Fixture::new();
    let mut original = fixture.store(1);
    let attachment = fixture.root.join("image.png");
    let references = vec![crate::file_ref::FileReference {
        path: attachment.clone(),
        mention: attachment.to_string_lossy().replace('\\', "/"),
    }];
    // Use the real prompt encoder, so reference parsing remains part of this regression.
    original.workspaces[0].sessions[0].messages[0].text =
        crate::file_ref::compose_prompt("look", &references);
    original.save_to(&fixture.state).unwrap();
    let mut cold = Store::load_from(&fixture.state).unwrap();
    let before = cold.workspaces[0].sessions[0]
        .transcript
        .reference
        .clone()
        .unwrap();
    assert!(before.attachments.contains(&attachment));
    cold.workspaces[0].sessions[0].title = "renamed without reading".into();
    cold.save_to(&fixture.state).unwrap();
    let restored = Store::load_from(&fixture.state).unwrap();
    assert!(restored.workspaces[0].sessions[0].messages.is_empty());
    assert_eq!(
        restored.workspaces[0].sessions[0]
            .transcript
            .reference
            .as_ref()
            .unwrap()
            .hash,
        before.hash
    );
    assert_eq!(
        read(&restored.workspaces[0].sessions[0]).unwrap()[1].text,
        "original reply"
    );
}

#[test]
fn old_writer_receipt_cannot_evict_a_modified_body() {
    let fixture = Fixture::new();
    let mut current = fixture.store(1);
    let pending = current.clone();
    current.workspaces[0].sessions[0].messages[1].text = "edited at same index".into();
    pending.save_to(&fixture.state).unwrap();
    assert!(!unload(&mut current.workspaces[0].sessions[0]));
    current.save_to(&fixture.state).unwrap();
    assert!(unload(&mut current.workspaces[0].sessions[0]));
    assert_eq!(
        read(&current.workspaces[0].sessions[0]).unwrap()[1].text,
        "edited at same index"
    );
}

#[test]
fn missing_or_corrupt_body_is_reported_without_replacing_the_manifest() {
    let fixture = Fixture::new();
    fixture.store(1).save_to(&fixture.state).unwrap();
    let mut state = Store::load_from(&fixture.state).unwrap();
    let session = &mut state.workspaces[0].sessions[0];
    let body = path(session.transcript.reference.as_ref().unwrap()).unwrap();
    let original = fs::read(&body).unwrap();
    fs::write(&body, b"broken").unwrap();
    assert!(
        load(session.clone())
            .unwrap_err()
            .to_string()
            .contains("校验")
    );
    assert!(session.messages.is_empty());
    fs::remove_file(&body).unwrap();
    assert!(load(state.workspaces[0].sessions[0].clone()).is_err());
    // A metadata save retains the missing reference; it must never turn it into an empty conversation.
    let reference = state.workspaces[0].sessions[0].transcript.reference.clone();
    state.save_to(&fixture.state).unwrap();
    assert_eq!(
        Store::load_from(&fixture.state).unwrap().workspaces[0].sessions[0]
            .transcript
            .reference,
        reference
    );
    fs::write(&body, original).unwrap();
    assert_eq!(
        load(state.workspaces[0].sessions[0].clone())
            .unwrap()
            .messages
            .len(),
        2
    );
}

#[test]
fn failed_manifest_write_keeps_previous_state_and_new_durable_body() {
    let fixture = Fixture::new();
    let mut state = fixture.store(1);
    state.save_to(&fixture.state).unwrap();
    let old = fs::read(&fixture.state).unwrap();
    state.workspaces[0].sessions[0].messages[1].text = "new reply".into();
    fs::create_dir(fixture.state.with_extension("json.tmp")).unwrap();
    assert!(state.save_to(&fixture.state).is_err());
    assert_eq!(fs::read(&fixture.state).unwrap(), old);
    collect_unreferenced(&directory(&fixture.state));
    assert!(unload(&mut state.workspaces[0].sessions[0]));
    assert_eq!(
        read(&Store::load_from(&fixture.state).unwrap().workspaces[0].sessions[0]).unwrap()[1].text,
        "original reply"
    );
    fs::remove_dir(fixture.state.with_extension("json.tmp")).unwrap();
    state.save_to(&fixture.state).unwrap();
    assert_eq!(
        read(&Store::load_from(&fixture.state).unwrap().workspaces[0].sessions[0]).unwrap()[1].text,
        "new reply"
    );
}

#[test]
fn streaming_large_message_writes_only_new_blocks_and_small_roots() {
    let fixture = Fixture::new();
    let mut state = fixture.store(1);
    let body: String = (0..128)
        .map(|index| format!("{index:08x}{}", "x".repeat(64 * 1024 - 8)))
        .collect();
    state.workspaces[0].sessions[0].messages[1].text = body.clone();
    let directory = directory(&fixture.state);
    let first = save(&state.workspaces[0].sessions[0], &directory)
        .unwrap()
        .unwrap();
    let disk_bytes = || {
        fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum::<u64>()
    };
    let baseline = disk_bytes();
    for _ in 0..12 {
        state.workspaces[0].sessions[0].messages[1]
            .text
            .push_str(&"d".repeat(32));
        save(&state.workspaces[0].sessions[0], &directory).unwrap();
    }
    let written = disk_bytes() - baseline;
    eprintln!(
        "incremental transcript: logical=384 bytes, written={written} bytes, baseline={baseline} bytes"
    );
    assert!(
        written < 256 * 1024,
        "wrote {written} bytes for 384 logical bytes"
    );
    assert!(unload(&mut state.workspaces[0].sessions[0]));
    collect_unreferenced(&directory);
    assert_eq!(
        read(&state.workspaces[0].sessions[0]).unwrap()[1].text,
        format!("{body}{}", "d".repeat(384))
    );
    let mut old = state.workspaces[0].sessions[0].clone();
    old.transcript.reference = Some(first);
    assert_eq!(read(&old).unwrap()[1].text, body);
    let mut changed = load(state.workspaces[0].sessions[0].clone()).unwrap();
    changed.messages[1].text.replace_range(100..103, "中");
    changed.messages[1].text.truncate(70_000);
    save(&changed, &directory).unwrap();
    assert!(unload(&mut changed));
    assert_eq!(read(&changed).unwrap()[1].text.len(), 70_000);
    let chunk = fs::read_dir(&directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.extension().is_some_and(|e| e == "chunk")
                && fs::read(p).unwrap().get(100..103) == Some("中".as_bytes())
        })
        .unwrap();
    fs::write(chunk, b"broken").unwrap();
    assert!(read(&changed).is_err());
}

#[test]
fn pages_reuse_unchanged_messages_and_legacy_v3_roots_still_load() {
    let fixture = Fixture::new();
    let mut state = fixture.store(1);
    let session = &mut state.workspaces[0].sessions[0];
    session.messages = (0..200)
        .map(|i| message(&format!("m-{i}"), &format!("body {i}"), i))
        .collect();
    let directory = directory(&fixture.state);
    save(session, &directory).unwrap();
    let before = session.transcript.receipt.lock().unwrap().clone().unwrap();
    session.messages[199].text.push_str(" delta");
    save(session, &directory).unwrap();
    let after = session.transcript.receipt.lock().unwrap().clone().unwrap();
    assert_eq!(&before.pages[..3], &after.pages[..3]);
    assert_ne!(before.pages[3], after.pages[3]);
    assert!(Arc::ptr_eq(&before.parts[0], &after.parts[0]));
    assert!(!Arc::ptr_eq(&before.parts[199], &after.parts[199]));
    let expected = serde_json::to_vec(&session.messages).unwrap();
    assert!(unload(session));
    assert_eq!(
        serde_json::to_vec(&read(session).unwrap()).unwrap(),
        expected
    );
    let mut shortened = load(session.clone()).unwrap();
    shortened.messages.truncate(130);
    save(&shortened, &directory).unwrap();
    assert!(unload(&mut shortened));
    assert_eq!(read(&shortened).unwrap().len(), 130);
    // An existing v3 whole-body JSON remains a valid root.
    let hash = format!("{:x}", Sha256::digest(&expected));
    fs::write(directory.join(format!("{hash}.json")), &expected).unwrap();
    let reference = session.transcript.reference.as_mut().unwrap();
    reference.hash = hash;
    reference.lease = Some(acquire(path(reference).unwrap()));
    let mut loaded = load(session.clone()).unwrap();
    assert_eq!(loaded.messages.len(), 200);
    loaded.messages[0].text = "changed legacy".into();
    save(&loaded, &directory).unwrap();
    assert!(unload(&mut loaded));
    assert_eq!(read(&loaded).unwrap()[0].text, "changed legacy");
}

#[test]
fn collection_keeps_inflight_reads_and_reclaims_obsolete_versions() {
    let fixture = Fixture::new();
    let mut state = fixture.store(1);
    state.save_to(&fixture.state).unwrap();
    let reader = Store::load_from(&fixture.state).unwrap();
    let old = path(
        reader.workspaces[0].sessions[0]
            .transcript
            .reference
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    state.workspaces[0].sessions[0].messages[1].text = "new reply".into();
    state.save_to(&fixture.state).unwrap();
    assert!(old.exists());
    assert_eq!(
        read(&reader.workspaces[0].sessions[0]).unwrap()[1].text,
        "original reply"
    );
    drop(reader);
    state.save_to(&fixture.state).unwrap();
    collect_unreferenced(&directory(&fixture.state));
    assert!(!old.exists());
    assert_eq!(fs::read_dir(directory(&fixture.state)).unwrap().count(), 1);
}

#[test]
fn cold_history_merge_preserves_local_messages_and_remains_cold() {
    let fixture = Fixture::new();
    let mut state = fixture.store(1);
    state.workspaces[0].sessions[0].messages[1].elapsed_ms = Some(4321);
    state.workspaces[0].sessions[0]
        .messages
        .push(message("user", "local unsynced turn", 30));
    state.save_to(&fixture.state).unwrap();
    let cold = Store::load_from(&fixture.state).unwrap();
    let incoming = crate::history::ImportedSession {
        provider: cold.workspaces[0].sessions[0].provider,
        native_id: "native-1".into(),
        created_at: 1,
        title: "native title".into(),
        model: None,
        messages: vec![
            message("user", "original prompt", 10),
            message("import-reply", "original reply", 20),
        ],
    };
    let (records, warnings) = crate::history::prepare_imported(
        &cold.workspaces[0],
        vec![incoming],
        &directory(&fixture.state),
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(records.len(), 1);
    assert!(!records[0].transcript.loaded);
    let loaded = load(records[0].clone()).unwrap();
    assert!(
        loaded
            .messages
            .iter()
            .any(|item| item.text == "local unsynced turn")
    );
    assert!(
        loaded
            .messages
            .iter()
            .any(|item| item.elapsed_ms == Some(4321))
    );
}

#[test]
fn corrupt_reference_cannot_escape_the_transcript_directory() {
    let fixture = Fixture::new();
    let mut value = serde_json::to_value(fixture.store(1)).unwrap();
    value["version"] = 3.into();
    let session = &mut value["workspaces"][0]["sessions"][0];
    session.as_object_mut().unwrap().remove("messages");
    session["transcript"] = serde_json::json!({"reference": {
        "hash": "../state", "message_count": 1, "last_activity_at": 20, "has_user": true, "bytes": 1, "attachments": []
    }});
    fs::write(&fixture.state, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Store::load_from(&fixture.state).is_err());
}
