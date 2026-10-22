use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Provider {
    Claude,
    Codex,
}

impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    #[cfg(test)]
    pub fn executable(self) -> Option<PathBuf> {
        crate::cli::resolve(self, &crate::cli::CliSettings::default())
            .ok()
            .map(|cli| cli.path)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: u64,
    pub provider: Provider,
    pub created_at: u64,
    pub exit_code: Option<u32>,
    #[serde(default)]
    pub title: String,
    /// Hidden from the sidebar unless archived sessions are shown.
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub native_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub messages: Vec<ChatItem>,
    #[serde(default)]
    pub interrupted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ItemKind {
    User,
    Assistant,
    Tool,
    Notice,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatItem {
    pub key: String,
    pub kind: ItemKind,
    pub text: String,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub path: PathBuf,
    pub sessions: Vec<SessionRecord>,
}

impl Workspace {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
}
