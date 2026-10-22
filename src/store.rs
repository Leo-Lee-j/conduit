use crate::domain::Workspace;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub cli: crate::cli::CliSettings,
}

impl Store {
    pub fn path() -> PathBuf {
        std::env::var_os("CONDUIT_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into()))
                    .join("Conduit")
            })
            .join("state.json")
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        Self::load_from(&path)
    }
    fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                version: 2,
                workspaces: vec![],
                cli: Default::default(),
            });
        }
        let mut state: Self = serde_json::from_slice(&fs::read(path)?)
            .with_context(|| format!("无法读取 {}；原文件不会被覆盖", path.display()))?;
        if state.version != 1 && state.version != 2 {
            bail!("不支持的数据版本 {}", state.version);
        }
        let legacy = state.version == 1;
        let mut paths = HashSet::new();
        let mut sessions = HashSet::new();
        for workspace in &mut state.workspaces {
            if !workspace.path.is_absolute() {
                bail!("工作区必须是绝对路径");
            }
            if let Ok(path) = workspace.path.canonicalize() {
                workspace.path = path;
            }
            if !paths.insert(workspace.path.clone()) {
                bail!("工作区目录重复，请修复 state.json");
            }
            for session in &mut workspace.sessions {
                if session.id == 0 || session.id == u64::MAX || !sessions.insert(session.id) {
                    bail!("会话 ID 无效或重复，请修复 state.json");
                }
                if legacy {
                    session.title = format!("旧终端会话 #{}", session.id);
                    session.messages.push(crate::domain::ChatItem {
                        key: "legacy-migration".into(),
                        kind: crate::domain::ItemKind::Notice,
                        text: "从终端原型迁移，仅保留元数据；发送消息将建立新的 CLI 对话。".into(),
                        input: String::new(),
                        detail: String::new(),
                        status: String::new(),
                    });
                }
                if session.interrupted {
                    for item in &mut session.messages {
                        if item.kind == crate::domain::ItemKind::Tool
                            && matches!(item.status.as_str(), "运行中" | "输出中")
                        {
                            item.status = "已中断".into();
                        }
                    }
                    session.messages.push(crate::domain::ChatItem {
                        key: format!("restart-{}", session.messages.len()),
                        kind: crate::domain::ItemKind::Notice,
                        text: "上次执行已中断，未自动重试；可以检查现有结果后继续。".into(),
                        input: String::new(),
                        detail: String::new(),
                        status: String::new(),
                    });
                    session.interrupted = false;
                }
                dedupe_restored_messages(session);
            }
        }
        state.version = 2;
        Ok(state)
    }
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        self.save_to(&path)
    }
    fn save_to(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path.parent().unwrap())?;
        let temp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        let mut file = fs::File::create(&temp)?;
        use std::io::Write;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(not(windows))]
        fs::rename(temp, path)?;
        Ok(())
    }
}

/// Older builds keyed Claude stream deltas by the raw stream block index while
/// the assistant snapshot keyed by the visible block index. A hidden thinking
/// block shifted one side by one, and the same answer was stored twice. Repair
/// that on load: drop adjacent assistant blocks with identical text and failure
/// notices already carried by the preceding assistant text.
fn dedupe_restored_messages(session: &mut crate::domain::SessionRecord) {
    let mut kept: Vec<crate::domain::ChatItem> = Vec::with_capacity(session.messages.len());
    for item in session.messages.drain(..) {
        let duplicate_assistant = item.kind == crate::domain::ItemKind::Assistant
            && !item.text.trim().is_empty()
            && kept.last().is_some_and(|previous| {
                previous.kind == crate::domain::ItemKind::Assistant
                    && previous.text.trim() == item.text.trim()
            });
        let duplicate_failure = item.kind == crate::domain::ItemKind::Notice
            && item.text.starts_with("执行失败：")
            && {
                let error = item.text.trim_start_matches("执行失败：").trim();
                kept.iter().rev().take(3).any(|previous| {
                    previous.kind == crate::domain::ItemKind::Assistant
                        && same_failure_text(&previous.text, error)
                })
            };
        if !duplicate_assistant && !duplicate_failure {
            kept.push(item);
        }
    }
    session.messages = kept;
}

fn same_failure_text(message: &str, error: &str) -> bool {
    let message = message.trim();
    let error = error.trim();
    if message.is_empty() || error.is_empty() {
        return false;
    }
    message == error || message.contains(error) || error.contains(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Provider, SessionRecord};
    #[test]
    fn legacy_migration_preserves_identity_without_fabricating_context() {
        let path =
            std::env::temp_dir().join(format!("conduit-migrate-{}.json", std::process::id()));
        let value = serde_json::json!({"version":1,"workspaces":[{"path":"C:\\conduit-migration", "sessions":[
            {"id":42,"provider":"Claude","created_at":9,"exit_code":0}]}]});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let restored = Store::load_from(&path).unwrap();
        assert_eq!(restored.version, 2);
        let session = &restored.workspaces[0].sessions[0];
        assert_eq!(session.id, 42);
        assert_eq!(session.provider, Provider::Claude);
        assert!(session.native_id.is_none());
        assert!(session.messages[0].text.contains("仅保留元数据"));
        assert_eq!(
            fs::read(&path).unwrap(),
            serde_json::to_vec(&value).unwrap()
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn chat_roundtrip_preserves_native_context_and_marks_interrupted_once() {
        let directory =
            std::env::temp_dir().join(format!("conduit-chat-store-{}", std::process::id()));
        let path = directory.join("state.json");
        let value = serde_json::json!({"version":2,"workspaces":[{"path":directory,"sessions":[
            {"id":8,"provider":"Codex","created_at":1,"exit_code":null,"title":"中文任务",
             "native_id":"native-thread","model":"gpt-test","interrupted":true,
             "messages":[{"key":"reply","kind":"Assistant","text":"部分回复","detail":"","status":""},
                         {"key":"tool","kind":"Tool","text":"tool","detail":"部分输出","status":"运行中"}]}]}]});
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let restored = Store::load_from(&path).unwrap();
        let session = &restored.workspaces[0].sessions[0];
        assert_eq!(session.native_id.as_deref(), Some("native-thread"));
        assert_eq!(session.model.as_deref(), Some("gpt-test"));
        assert_eq!(session.messages[0].text, "部分回复");
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.messages[1].status, "已中断");
        assert!(!session.interrupted);
        restored.save_to(&path).unwrap();
        assert_eq!(
            Store::load_from(&path).unwrap().workspaces[0].sessions[0]
                .messages
                .len(),
            3
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn atomic_replacement_and_corruption_preservation() {
        let directory = std::env::temp_dir().join(format!(
            "conduit-store-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("state.json");
        let mut store = Store {
            version: 2,
            cli: Default::default(),
            workspaces: vec![Workspace {
                path: directory.clone(),
                sessions: vec![],
            }],
        };
        store.save_to(&path).unwrap();
        store.workspaces[0].sessions.push(SessionRecord {
            id: 1,
            provider: Provider::Codex,
            created_at: 1,
            exit_code: Some(7),
            title: "Test".into(),
            archived: false,
            native_id: None,
            model: None,
            messages: vec![],
            interrupted: false,
        });
        store
            .cli
            .set(Provider::Claude, Some(directory.join("custom-claude.exe")));
        store.save_to(&path).unwrap();
        let restored = Store::load_from(&path).unwrap();
        assert_eq!(restored.cli, store.cli);
        assert_eq!(restored.workspaces[0].sessions[0].exit_code, Some(7));
        fs::write(&path, b"invalid json").unwrap();
        assert!(Store::load_from(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid json");
        // Only this test's exact files are removed; do not recursively remove a computed directory.
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn duplicate_session_ids_are_rejected() {
        let value = serde_json::json!({"version":1,"workspaces":[{"path":"C:\\conduit-test", "sessions":[
            {"id":1,"provider":"Claude","created_at":0,"exit_code":null},
            {"id":1,"provider":"Codex","created_at":0,"exit_code":null}]}]});
        let path =
            std::env::temp_dir().join(format!("conduit-duplicate-{}.json", std::process::id()));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Store::load_from(&path).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn restored_duplicate_assistant_blocks_and_failure_notices_are_cleaned() {
        let directory = std::env::temp_dir().join(format!("conduit-dedupe-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("state.json");
        let value = serde_json::json!({"version":2,"workspaces":[{"path":directory,"sessions":[
        {"id":1,"provider":"Claude","created_at":0,"exit_code":null,"title":"t","messages":[
            {"key":"turn-0-a","kind":"Assistant","text":"Hello"},
            {"key":"turn-0-b","kind":"Assistant","text":"Hello"},
            {"key":"turn-0-c","kind":"Assistant","text":"boom"},
            {"key":"turn-0-notice","kind":"Notice","text":"执行失败：boom"}
        ]}]}]});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let restored = Store::load_from(&path).unwrap();
        let messages = &restored.workspaces[0].sessions[0].messages;
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "Hello");
        assert_eq!(messages[1].text, "boom");
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn archived_flag_defaults_to_false_and_roundtrips() {
        let directory =
            std::env::temp_dir().join(format!("conduit-archived-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("state.json");
        let value = serde_json::json!({"version":2,"workspaces":[{"path":directory,"sessions":[
            {"id":1,"provider":"Claude","created_at":0,"exit_code":null,"title":"t"},
            {"id":2,"provider":"Codex","created_at":0,"exit_code":null,"title":"u","archived":true}
        ]}]});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut restored = Store::load_from(&path).unwrap();
        assert!(!restored.workspaces[0].sessions[0].archived);
        assert!(restored.workspaces[0].sessions[1].archived);
        restored.workspaces[0].sessions[0].archived = true;
        restored.save_to(&path).unwrap();
        let reloaded = Store::load_from(&path).unwrap();
        assert!(reloaded.workspaces[0].sessions[0].archived);
        assert!(reloaded.workspaces[0].sessions[1].archived);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
