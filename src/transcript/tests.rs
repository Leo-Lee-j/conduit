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
        assert_eq!(state.version, 3);
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
