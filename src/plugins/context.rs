use super::{Error, Result, require};
use std::path::{Path, PathBuf};

/// Uses the path identity already chosen by Store; no filesystem access or case folding.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceKey(PathBuf);
impl WorkspaceKey {
    pub fn from_host_path(path: PathBuf) -> Result<Self> {
        require(
            path.is_absolute(),
            "workspace identity must be an absolute host path",
        )?;
        Ok(Self(path))
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionKey {
    pub workspace: WorkspaceKey,
    pub id: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Workspace,
    Session,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScopeKey {
    Global,
    Workspace(WorkspaceKey),
    Session { workspace: WorkspaceKey, id: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextInput {
    pub workspace: Option<WorkspaceKey>,
    pub session: Option<SessionKey>,
    pub document: Option<String>,
    pub session_running: bool,
    pub writable: bool,
    pub closing: bool,
}
#[derive(Clone, Debug)]
pub struct PluginContext {
    input: ContextInput,
    version: u64,
}
impl PluginContext {
    pub fn input(&self) -> &ContextInput {
        &self.input
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn writable(&self) -> bool {
        self.input.writable && !self.input.closing
    }
    pub fn scope(&self, scope: Scope) -> Option<ScopeKey> {
        match scope {
            Scope::Global => Some(ScopeKey::Global),
            Scope::Workspace => self.input.workspace.clone().map(ScopeKey::Workspace),
            Scope::Session => self
                .input
                .session
                .as_ref()
                .map(|session| ScopeKey::Session {
                    workspace: session.workspace.clone(),
                    id: session.id,
                }),
        }
    }
    pub fn matches_scope(&self, scope: &ScopeKey) -> bool {
        match scope {
            ScopeKey::Global => true,
            ScopeKey::Workspace(_) => self.scope(Scope::Workspace).as_ref() == Some(scope),
            ScopeKey::Session { .. } => self.scope(Scope::Session).as_ref() == Some(scope),
        }
    }
}
#[derive(Default)]
pub struct ContextTracker {
    current: Option<PluginContext>,
}
impl ContextTracker {
    pub fn update(&mut self, mut input: ContextInput) -> Result<&PluginContext> {
        if input
            .session
            .as_ref()
            .is_some_and(|s| Some(&s.workspace) != input.workspace.as_ref())
        {
            input.session = None;
        }
        if input.session.is_none() {
            input.session_running = false;
        }
        let changed = self.current.as_ref().is_none_or(|old| {
            old.input.workspace != input.workspace
                || old.input.session != input.session
                || old.input.document != input.document
                || old.input.writable != input.writable
                || old.input.closing != input.closing
        });
        let version = self.current.as_ref().map_or(0, |old| old.version);
        let version = if changed {
            version
                .checked_add(1)
                .ok_or_else(|| Error("context version exhausted".into()))?
        } else {
            version
        };
        self.current = Some(PluginContext { input, version });
        Ok(self.current.as_ref().unwrap())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Workspace,
    Session,
    Running,
    Document,
    Writable,
}
/// Restricted conjunctions; neither arbitrary expressions nor plugin-defined keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct When(Vec<(Key, bool)>);
impl When {
    pub fn parse(source: Option<&str>) -> Result<Self> {
        let Some(source) = source else {
            return Ok(Self::default());
        };
        require(source.len() <= 512, "condition exceeds 512 bytes")?;
        let mut terms = Vec::new();
        for term in source.split("&&") {
            let term = term.trim();
            let (term, positive) = term.strip_prefix('!').map_or((term, true), |s| (s, false));
            let key = match term {
                "workspace.open" => Key::Workspace,
                "session.selected" => Key::Session,
                "session.running" => Key::Running,
                "document.open" => Key::Document,
                "host.writable" => Key::Writable,
                _ => return Err(Error(format!("unsupported condition: {term}"))),
            };
            terms.push((key, positive));
        }
        Ok(Self(terms))
    }
    pub fn matches(&self, cx: &PluginContext) -> bool {
        self.0.iter().all(|(key, expected)| {
            let value = match key {
                Key::Workspace => cx.input.workspace.is_some(),
                Key::Session => cx.input.session.is_some(),
                Key::Running => cx.input.session_running,
                Key::Document => cx.input.document.is_some(),
                Key::Writable => cx.writable(),
            };
            value == *expected
        })
    }
}
