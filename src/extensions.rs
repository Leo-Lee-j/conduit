//! Bounded, read-only discovery. A definition is not proof of runtime availability.
pub mod connection;
pub mod edit;
pub mod management;
pub mod mcp;

use crate::domain::Provider;
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

const FILE_LIMIT: u64 = 2 * 1024 * 1024;
const ENTRY_LIMIT: usize = 2000;
const WALK_LIMIT: usize = 10000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Skill,
    Mcp,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub scope: String,
    pub kind: Kind,
    pub disabled: bool,
    pub transport: String,
    /// Only an origin or executable filename; args, headers and env are never retained.
    pub target: String,
}

impl Entry {
    pub fn managed(&self) -> bool {
        self.scope.starts_with("插件")
    }
    pub fn id(&self) -> String {
        format!("{:?}:{}:{}", self.kind, self.path.display(), self.name)
    }
    pub fn status(&self) -> &'static str {
        if self.disabled {
            conduit::i18n::text_static("extensions.configuration_disabled")
        } else {
            match self.kind {
                Kind::Skill => conduit::i18n::text_static("extensions.discovered"),
                Kind::Mcp => {
                    conduit::i18n::text_static("extensions.configured_connection_not_tested")
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub errors: Vec<String>,
    pub scanned: Vec<PathBuf>,
    pub sources: Vec<PathBuf>,
    pub updated: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Context {
    pub engine: Provider,
    pub workspace: Option<PathBuf>,
    pub cli_path: Option<PathBuf>,
}

pub struct Probe {
    pub context: Context,
    pub events: Receiver<Catalog>,
    cancelled: Arc<AtomicBool>,
}

impl Probe {
    /// A completed discovery event for deterministic workspace lifecycle tests.
    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn ready(context: Context, catalog: Catalog) -> Self {
        let (sender, events) = mpsc::channel();
        sender.send(catalog).unwrap();
        Self {
            context,
            events,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn start(context: Context) -> Self {
        let (sender, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let query = context.clone();
        thread::spawn(move || {
            let mut scan = Scan::new(&cancel);
            scan.discover(&query, &Locations::environment());
            scan.catalog.entries.sort_by_key(|entry| {
                (
                    entry.kind == Kind::Mcp,
                    entry.name.to_lowercase(),
                    entry.path.clone(),
                )
            });
            scan.catalog.updated = Some(SystemTime::now());
            if !cancel.load(Ordering::Acquire) {
                let _ = sender.send(scan.catalog);
            }
        });
        Self {
            context,
            events,
            cancelled,
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.cancel();
    }
}

struct Locations {
    home: PathBuf,
    codex: PathBuf,
    claude: PathBuf,
    opencode: PathBuf,
    opencode_extra: Option<PathBuf>,
    opencode_dir: Option<PathBuf>,
}
impl Locations {
    fn environment() -> Self {
        let home = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default();
        Self {
            codex: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            claude: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude")),
            opencode: std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode"),
            opencode_extra: std::env::var_os("OPENCODE_CONFIG").map(PathBuf::from),
            opencode_dir: std::env::var_os("OPENCODE_CONFIG_DIR").map(PathBuf::from),
            home,
        }
    }
}

struct Scan<'a> {
    cancel: &'a AtomicBool,
    started: Instant,
    seen: HashSet<PathBuf>,
    visited: usize,
    catalog: Catalog,
    limited: bool,
}
impl<'a> Scan<'a> {
    fn new(cancel: &'a AtomicBool) -> Self {
        Self {
            cancel,
            started: Instant::now(),
            seen: HashSet::new(),
            visited: 0,
            catalog: Catalog::default(),
            limited: false,
        }
    }
    fn active(&mut self) -> bool {
        if self.cancel.load(Ordering::Acquire) {
            return false;
        }
        if self.started.elapsed() > Duration::from_secs(10)
            || self.catalog.entries.len() >= ENTRY_LIMIT
            || self.visited >= WALK_LIMIT
        {
            if !self.limited {
                self.catalog.errors.push(
                    conduit::i18n::text_static("extensions.scan_reached_its_time_or_count_limit")
                        .into(),
                );
                self.limited = true;
            }
            return false;
        }
        true
    }
    fn error(&mut self, path: &Path, message: &str) {
        // Do not include parser errors or CLI output: they can contain credentials.
        self.catalog
            .errors
            .push(format!("{}：{message}", path.display()));
    }
    fn read(&mut self, path: &Path) -> Option<String> {
        if !self.active() {
            return None;
        }
        let canonical = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => {
                self.error(path, conduit::i18n::text_static("extensions.inaccessible"));
                return None;
            }
        };
        if !self.seen.insert(canonical) {
            return None;
        }
        self.visited += 1;
        self.catalog.scanned.push(path.to_path_buf());
        let result = fs::File::open(path).and_then(|file| {
            let mut bytes = Vec::new();
            file.take(FILE_LIMIT + 1).read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        match result {
            Ok(bytes) if bytes.len() as u64 > FILE_LIMIT => {
                self.error(
                    path,
                    conduit::i18n::text_static("extensions.file_exceeds_2_mib_skipped"),
                );
                None
            }
            Ok(bytes) => String::from_utf8(bytes)
                .map_err(|_| {
                    self.error(
                        path,
                        conduit::i18n::text_static("extensions.edit.file_is_not_utf_8"),
                    )
                })
                .ok(),
            Err(_) => {
                self.error(
                    path,
                    conduit::i18n::text_static("settings.models.read_failed"),
                );
                None
            }
        }
    }
    fn json(&mut self, path: &Path) -> Option<Value> {
        let text = self.read(path)?;
        match jsonc(&text) {
            Some(value) => Some(value),
            None => {
                self.error(
                    path,
                    conduit::i18n::text_static("extensions.unable_to_parse_json_configuration"),
                );
                None
            }
        }
    }
    fn config(&mut self, path: &Path, scope: &str, key: &str) {
        self.catalog.sources.push(path.to_path_buf());
        let value = if path.extension().is_some_and(|ext| ext == "toml") {
            let Some(text) = self.read(path) else {
                return;
            };
            match toml::from_str::<toml::Value>(&text)
                .ok()
                .and_then(|v| serde_json::to_value(v).ok())
            {
                Some(v) => v,
                None => {
                    self.error(
                        path,
                        conduit::i18n::text_static("extensions.unable_to_parse_toml_configuration"),
                    );
                    return;
                }
            }
        } else {
            let Some(value) = self.json(path) else {
                return;
            };
            value
        };
        self.servers(&value[key], path, scope, &[]);
        if key == "mcp_servers" {
            self.codex_plugins(&value, path);
        }
        if let Some(paths) = value.pointer("/skills/paths").and_then(Value::as_array) {
            for entry in paths {
                if let Some(folder) = entry.as_str() {
                    let folder = expand_path(folder, path.parent().unwrap_or(Path::new(".")));
                    self.skills(&folder, scope, 0);
                }
            }
        }
        if let Some(config) = value.pointer("/skills/config").and_then(Value::as_array) {
            for item in config {
                if item["enabled"] == false
                    && let Some(skill) = item["path"].as_str()
                {
                    let skill = expand_path(skill, path.parent().unwrap_or(Path::new(".")));
                    let canonical = fs::canonicalize(skill).ok();
                    for entry in &mut self.catalog.entries {
                        if entry.kind == Kind::Skill
                            && canonical.is_some()
                            && fs::canonicalize(&entry.path).ok() == canonical
                        {
                            entry.disabled = true;
                        }
                    }
                }
            }
        }
    }
    fn servers(&mut self, value: &Value, path: &Path, scope: &str, disabled: &[String]) {
        let Some(servers) = value.as_object() else {
            if !value.is_null() {
                self.error(
                    path,
                    conduit::i18n::text_static("extensions.unsupported_mcp_list_format"),
                );
            }
            return;
        };
        for (name, server) in servers {
            if !self.active() {
                break;
            }
            if !server.is_object() {
                self.error(
                    path,
                    conduit::i18n::text_static("extensions.mcp_entry_is_not_an_object"),
                );
                continue;
            }
            let url = server["url"].as_str();
            let transport = server["type"]
                .as_str()
                .unwrap_or(if url.is_some() { "http" } else { "stdio" })
                .to_owned();
            let command = server["command"]
                .as_str()
                .or_else(|| server["command"][0].as_str());
            let target = url
                .map(origin)
                .or_else(|| command.map(executable_name))
                .unwrap_or_else(|| {
                    conduit::i18n::text_static("extensions.no_endpoint_or_command_provided").into()
                });
            self.catalog.entries.push(Entry {
                name: clipped(name, 160),
                description: String::new(),
                path: path.into(),
                scope: scope.into(),
                kind: Kind::Mcp,
                disabled: server["enabled"] == false
                    || server["disabled"] == true
                    || disabled.iter().any(|s| s == name),
                transport: clipped(&transport, 32),
                target,
            });
        }
    }
    fn skills(&mut self, folder: &Path, scope: &str, depth: usize) {
        if depth == 0 {
            self.catalog.sources.push(folder.to_path_buf());
        }
        if depth > 5 || !self.active() {
            return;
        }
        let canonical = match fs::canonicalize(folder) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => {
                self.error(
                    folder,
                    conduit::i18n::text_static("extensions.unable_to_access_skill_directory"),
                );
                return;
            }
        };
        if !self.seen.insert(canonical) {
            return;
        }
        let entries = match fs::read_dir(folder) {
            Ok(entries) => entries,
            Err(_) => {
                self.error(
                    folder,
                    conduit::i18n::text_static("extensions.unable_to_read_skill_directory"),
                );
                return;
            }
        };
        let active = folder.join("SKILL.md");
        let disabled = folder.join(management::DISABLED_SKILL);
        let definition = if active.exists() {
            active
        } else if disabled.exists() {
            disabled
        } else {
            active
        };
        let is_disabled = definition
            .file_name()
            .is_some_and(|name| name == management::DISABLED_SKILL);
        if let Some(text) = self.read(&definition) {
            let (name, description) = skill_metadata(&text);
            match name.filter(|name| !name.trim().is_empty()) {
                Some(name) => self.catalog.entries.push(Entry {
                    name: clipped(&name, 160),
                    description: clipped(&description, 1200),
                    path: definition,
                    scope: scope.into(),
                    kind: Kind::Skill,
                    disabled: is_disabled,
                    transport: String::new(),
                    target: String::new(),
                }),
                None => self.error(
                    &definition,
                    conduit::i18n::text_static(
                        "extensions.skill_is_missing_a_valid_frontmatter_name",
                    ),
                ),
            }
            return;
        }
        for entry in entries {
            if !self.active() {
                break;
            }
            self.visited += 1;
            match entry {
                Ok(entry) if entry.path().is_dir() => self.skills(&entry.path(), scope, depth + 1),
                Ok(_) => {}
                Err(_) => self.error(
                    folder,
                    conduit::i18n::text_static(
                        "extensions.some_directory_entries_could_not_be_read",
                    ),
                ),
            }
        }
    }
    fn discover(&mut self, context: &Context, locations: &Locations) {
        if locations.home.as_os_str().is_empty() {
            self.catalog.errors.push(
                conduit::i18n::text_static(
                    "extensions.unable_to_locate_user_directory_check_userprofile",
                )
                .into(),
            );
            return;
        }
        let projects = context
            .workspace
            .as_deref()
            .map(project_directories)
            .unwrap_or_default();
        match context.engine {
            Provider::Qoder => {
                self.catalog
                    .errors
                    .push(conduit::i18n::text("qoder.extensions_native"));
            }
            Provider::Claude => {
                self.skills(&locations.claude.join("skills"), "全局", 0);
                for project in &projects {
                    self.skills(&project.join(".claude/skills"), "项目", 0);
                }
                let global = locations.home.join(".claude.json");
                self.catalog.sources.push(global.clone());
                let mut project_disabled = Vec::new();
                if let Some(value) = self.json(&global) {
                    self.servers(&value["mcpServers"], &global, "全局", &[]);
                    if let Some(workspace) = &context.workspace
                        && let Some(projects) = value["projects"].as_object()
                    {
                        for (folder, settings) in projects {
                            if same_path(Path::new(folder), workspace) {
                                let disabled = strings(&settings["disabledMcpServers"]);
                                project_disabled.extend(disabled.clone());
                                self.servers(
                                    &settings["mcpServers"],
                                    &global,
                                    "项目（本机）",
                                    &disabled,
                                );
                                for entry in &mut self.catalog.entries {
                                    if entry.kind == Kind::Mcp && disabled.contains(&entry.name) {
                                        entry.disabled = true;
                                    }
                                }
                            }
                        }
                    }
                }
                for project in &projects {
                    self.config(&project.join(".mcp.json"), "项目", "mcpServers");
                }
                self.claude_plugins(locations);
                for entry in &mut self.catalog.entries {
                    if entry.kind == Kind::Mcp && project_disabled.contains(&entry.name) {
                        entry.disabled = true;
                    }
                }
            }
            Provider::Codex => {
                self.skills(&locations.home.join(".agents/skills"), "全局", 0);
                self.skills(&locations.codex.join("skills"), "全局（Codex）", 0);
                for project in &projects {
                    self.skills(&project.join(".agents/skills"), "项目", 0);
                }
                self.config(&locations.codex.join("config.toml"), "全局", "mcp_servers");
                for project in projects.iter().rev() {
                    self.config(&project.join(".codex/config.toml"), "项目", "mcp_servers");
                }
            }
            Provider::OpenCode => {
                for folder in [
                    &locations.opencode,
                    &locations.home.join(".claude"),
                    &locations.home.join(".agents"),
                ] {
                    self.skills(&folder.join("skills"), "全局", 0);
                    self.skills(&folder.join("skill"), "全局", 0);
                }
                for project in &projects {
                    for folder in [
                        ".opencode/skills",
                        ".opencode/skill",
                        ".claude/skills",
                        ".agents/skills",
                    ] {
                        self.skills(&project.join(folder), "项目", 0);
                    }
                }
                for filename in ["opencode.json", "opencode.jsonc"] {
                    self.config(&locations.opencode.join(filename), "全局", "mcp");
                }
                for project in projects.iter().rev() {
                    for filename in [
                        "opencode.json",
                        "opencode.jsonc",
                        ".opencode/opencode.json",
                        ".opencode/opencode.jsonc",
                    ] {
                        self.config(&project.join(filename), "项目", "mcp");
                    }
                }
                if let Some(path) = &locations.opencode_extra {
                    self.config(path, "环境指定", "mcp");
                }
                if let Some(folder) = &locations.opencode_dir {
                    self.skills(&folder.join("skills"), "环境指定", 0);
                    self.skills(&folder.join("skill"), "环境指定", 0);
                    for filename in ["opencode.json", "opencode.jsonc"] {
                        self.config(&folder.join(filename), "环境指定", "mcp");
                    }
                }
            }
        }
    }
    fn claude_plugins(&mut self, locations: &Locations) {
        let settings = self
            .json(&locations.claude.join("settings.json"))
            .unwrap_or(Value::Null);
        let path = locations.claude.join("plugins/installed_plugins.json");
        if let Some(value) = self.json(&path)
            && let Some(plugins) = value["plugins"].as_object()
        {
            for (name, installs) in plugins {
                if settings["enabledPlugins"][name] != true {
                    continue;
                }
                for install in installs.as_array().into_iter().flatten() {
                    // Only global installations; project plugin enablement needs native resolution.
                    if install["scope"] != "user" {
                        continue;
                    }
                    if let Some(folder) = install["installPath"].as_str() {
                        let folder = PathBuf::from(folder);
                        self.skills(&folder.join("skills"), "插件", 0);
                        self.config(&folder.join(".mcp.json"), "插件", "mcpServers");
                    }
                }
            }
        }
    }
    fn codex_plugins(&mut self, config: &Value, path: &Path) {
        let Some(plugins) = config["plugins"].as_object() else {
            return;
        };
        let cache = path
            .parent()
            .unwrap_or(Path::new("."))
            .join("plugins/cache");
        for (id, options) in plugins {
            if options["enabled"] != true {
                continue;
            }
            let Some((name, marketplace)) = id.split_once('@') else {
                continue;
            };
            // Do not treat cache contents as installed/effective plugin versions.
            if !safe_component(name) || !safe_component(marketplace) {
                continue;
            }
            let folder = cache.join(marketplace).join(name);
            self.catalog.sources.push(folder.clone());
            let Ok(versions) = fs::read_dir(&folder) else {
                continue;
            };
            for version in versions.flatten() {
                if !self.active() {
                    return;
                }
                self.visited += 1;
                if !version.path().is_dir() {
                    continue;
                }
                let folder = version.path();
                self.skills(&folder.join("skills"), "插件缓存", 0);
                self.config(&folder.join(".mcp.json"), "插件缓存", "mcpServers");
                for manifest in [".codex-plugin/plugin.json", ".claude-plugin/plugin.json"] {
                    if let Some(value) = self.json(&folder.join(manifest)) {
                        if let Some(skills) = value["skills"].as_str() {
                            self.skills(&folder.join(skills), "插件缓存", 0);
                        }
                        if value["mcpServers"].is_object() {
                            self.servers(
                                &value["mcpServers"],
                                &folder.join(manifest),
                                "插件缓存",
                                &[],
                            );
                        }
                        if let Some(file) = value["mcpServers"].as_str() {
                            self.config(&folder.join(file), "插件缓存", "mcpServers");
                        }
                    }
                }
            }
        }
    }
}

fn safe_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', ':'])
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}
fn project_directories(workspace: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for folder in workspace.ancestors() {
        directories.push(folder.to_path_buf());
        if folder.join(".git").exists() {
            break;
        }
    }
    directories
}
fn same_path(a: &Path, b: &Path) -> bool {
    fs::canonicalize(a)
        .ok()
        .zip(fs::canonicalize(b).ok())
        .is_some_and(|(a, b)| a == b)
        || a.to_string_lossy()
            .replace('\\', "/")
            .eq_ignore_ascii_case(&b.to_string_lossy().replace('\\', "/"))
}
fn expand_path(value: &str, base: &Path) -> PathBuf {
    if let Some(relative) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(relative);
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}
fn clipped(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(limit)
        .collect()
}
fn executable_name(value: &str) -> String {
    value
        .rsplit(['/', '\\'])
        .next()
        .filter(|v| !v.contains([' ', '=', '?', '&']))
        .map(|v| clipped(v, 160))
        .unwrap_or_else(|| conduit::i18n::text_static("extensions.command_hidden").into())
}
fn origin(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https" | "ws" | "wss"))
        .map(|u| u.origin().ascii_serialization())
        .filter(|s| s != "null")
        .unwrap_or_else(|| conduit::i18n::text_static("extensions.endpoint_hidden").into())
}

fn skill_metadata(text: &str) -> (Option<String>, String) {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, String::new());
    }
    let mut name = None;
    let mut description = String::new();
    let mut multiline = false;
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        if multiline && line.starts_with([' ', '\t']) {
            description.push(' ');
            description.push_str(line.trim());
            continue;
        }
        multiline = false;
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(unquote(value.trim()));
        }
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            multiline = matches!(value, ">" | "|" | ">-" | "|-");
            if !multiline {
                description = unquote(value);
            }
        }
    }
    if closed {
        (name, description)
    } else {
        (None, String::new())
    }
}
fn unquote(value: &str) -> String {
    if value.starts_with('"') {
        serde_json::from_str::<String>(value).unwrap_or_else(|_| value.trim_matches('"').to_owned())
    } else {
        value.trim_matches('\'').to_owned()
    }
}

/// Strip JSONC comments/trailing commas while leaving strings untouched.
fn jsonc(text: &str) -> Option<Value> {
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let mut output = String::new();
    let mut quoted = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if quoted {
            output.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
        } else if c == '"' {
            quoted = true;
            output.push(c);
        } else if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for c in chars.by_ref() {
                if c == '\n' {
                    output.push('\n');
                    break;
                }
            }
        } else if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = ' ';
            let mut closed = false;
            for c in chars.by_ref() {
                if previous == '*' && c == '/' {
                    closed = true;
                    break;
                }
                previous = c;
            }
            if !closed {
                return None;
            }
            output.push(' ');
        } else {
            output.push(c);
        }
    }
    let chars: Vec<char> = output.chars().collect();
    let mut cleaned = String::new();
    quoted = false;
    escaped = false;
    for (i, c) in chars.iter().copied().enumerate() {
        if !quoted
            && c == ','
            && chars[i + 1..]
                .iter()
                .find(|c| !c.is_whitespace())
                .is_some_and(|c| matches!(c, '}' | ']'))
        {
            continue;
        }
        cleaned.push(c);
        if escaped {
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        }
    }
    serde_json::from_str(&cleaned).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jsonc_preserves_urls_strings_and_unicode() {
        let _locale = crate::test_support::chinese();
        let v =
            jsonc("{ // comment\n\"mcp\":{\"中文\":{\"url\":\"https://host/a//b?q=secret\",},},}")
                .unwrap();
        assert_eq!(v["mcp"]["中文"]["url"], "https://host/a//b?q=secret");
        assert!(jsonc("{/* never closed").is_none());
    }
    #[test]
    fn frontmatter_handles_multiline_and_quoted_fields() {
        let _locale = crate::test_support::chinese();
        assert_eq!(
            skill_metadata("---\nname: '测试'\ndescription: >-\n  first\n  second\n---\nbody"),
            (Some("测试".into()), " first second".into())
        );
        assert!(skill_metadata("---\nname: broken").0.is_none());
    }
    #[test]
    fn mcp_metadata_never_retains_secrets() {
        let _locale = crate::test_support::chinese();
        let cancel = AtomicBool::new(false);
        let mut scan = Scan::new(&cancel);
        scan.servers(&serde_json::json!({"test":{"url":"https://user:password@host/path-secret?token=secret#secret","headers":{"Authorization":"secret"},"env":{"KEY":"secret"},"args":["secret"]}}), Path::new("config.json"), "全局", &[]);
        let entry = &scan.catalog.entries[0];
        assert_eq!(entry.target, "https://host");
        assert!(!format!("{entry:?}").contains("secret"));
        assert_eq!(executable_name("npx --token=secret"), "命令已隐藏");
    }
    #[test]
    fn discovery_keeps_scopes_partial_errors_and_disabled_entries() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join(format!(
            "conduit-extensions-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("user");
        let workspace = root.join("project");
        fs::create_dir_all(home.join(".agents/skills/global")).unwrap();
        fs::create_dir_all(workspace.join(".agents/skills/local")).unwrap();
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::create_dir_all(workspace.join(".codex")).unwrap();
        fs::create_dir_all(workspace.join(".git")).unwrap();
        fs::write(
            home.join(".agents/skills/global/SKILL.md"),
            "---\nname: same\ndescription: global\n---",
        )
        .unwrap();
        fs::write(
            workspace.join(".agents/skills/local/SKILL.md"),
            "---\nname: same\ndescription: local\n---",
        )
        .unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[mcp_servers.demo]\ncommand='tool'\nenabled=false\n",
        )
        .unwrap();
        fs::write(workspace.join(".codex/config.toml"), "invalid = [").unwrap();
        let locations = Locations {
            codex: home.join(".codex"),
            claude: home.join(".claude"),
            opencode: home.join(".config/opencode"),
            home,
            opencode_extra: None,
            opencode_dir: None,
        };
        let cancel = AtomicBool::new(false);
        let mut scan = Scan::new(&cancel);
        scan.discover(
            &Context {
                engine: Provider::Codex,
                workspace: Some(workspace.clone()),
                cli_path: None,
            },
            &locations,
        );
        assert_eq!(scan.catalog.entries.len(), 3);
        assert_eq!(
            scan.catalog
                .entries
                .iter()
                .filter(|e| e.name == "same")
                .count(),
            2
        );
        assert!(
            scan.catalog
                .entries
                .iter()
                .any(|e| e.kind == Kind::Mcp && e.disabled)
        );
        assert_eq!(scan.catalog.errors.len(), 1);
        let mut global = Scan::new(&cancel);
        global.discover(
            &Context {
                engine: Provider::Codex,
                workspace: None,
                cli_path: None,
            },
            &locations,
        );
        assert_eq!(global.catalog.entries.len(), 2);
        let resolved = fs::canonicalize(&root).unwrap();
        assert!(resolved.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        fs::remove_dir_all(resolved).unwrap();
    }
    #[test]
    fn claude_and_opencode_scans_keep_disabled_sources_and_jsonc() {
        let _locale = crate::test_support::chinese();
        let root = std::env::temp_dir().join(format!(
            "conduit-extensions-engines-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = root.join("home");
        let project = root.join("project");
        fs::create_dir_all(home.join(".claude/skills/demo")).unwrap();
        fs::create_dir_all(home.join(".config/opencode")).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::write(
            home.join(".claude/skills/demo/SKILL.md"),
            "---\nname: example\ndescription: '中文简介'\n---",
        )
        .unwrap();
        let projects =
            serde_json::json!({project.to_string_lossy().as_ref():{"disabledMcpServers":["demo"]}});
        fs::write(home.join(".claude.json"),serde_json::to_vec(&serde_json::json!({"mcpServers":{"global":{"command":"tool"}},"projects":projects})).unwrap()).unwrap();
        fs::write(project.join(".mcp.json"),r#"{"mcpServers":{"demo":{"type":"http","url":"https://example.org/private?key=secret"}}}"#).unwrap();
        fs::write(home.join(".config/opencode/opencode.jsonc"),r#"{ /* comment */ "mcp": { "native": {"type":"local","command":["tool","secret"],"enabled":false,},},}"#).unwrap();
        let locations = Locations {
            codex: home.join(".codex"),
            claude: home.join(".claude"),
            opencode: home.join(".config/opencode"),
            home,
            opencode_extra: None,
            opencode_dir: None,
        };
        let cancel = AtomicBool::new(false);
        let mut claude = Scan::new(&cancel);
        claude.discover(
            &Context {
                engine: Provider::Claude,
                workspace: Some(project.clone()),
                cli_path: None,
            },
            &locations,
        );
        assert_eq!(claude.catalog.entries.len(), 3);
        assert!(
            claude
                .catalog
                .entries
                .iter()
                .any(|e| e.name == "demo" && e.disabled && e.target == "https://example.org")
        );
        let mut opencode = Scan::new(&cancel);
        opencode.discover(
            &Context {
                engine: Provider::OpenCode,
                workspace: Some(project),
                cli_path: None,
            },
            &locations,
        );
        assert!(
            opencode
                .catalog
                .entries
                .iter()
                .any(|e| e.kind == Kind::Skill && e.name == "example")
        );
        assert!(
            opencode
                .catalog
                .entries
                .iter()
                .any(|e| e.name == "native" && e.disabled && e.target == "tool")
        );
        assert!(!format!("{:?}", opencode.catalog.entries).contains("secret"));
        let cancelled = AtomicBool::new(true);
        let mut scan = Scan::new(&cancelled);
        scan.discover(
            &Context {
                engine: Provider::Claude,
                workspace: None,
                cli_path: None,
            },
            &locations,
        );
        assert!(scan.catalog.entries.is_empty());
        let resolved = fs::canonicalize(&root).unwrap();
        assert!(resolved.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        fs::remove_dir_all(resolved).unwrap();
    }
}
