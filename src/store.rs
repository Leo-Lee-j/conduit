use crate::domain::Workspace;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

mod session_archive;
pub use session_archive::{AutoArchiveDays, AutoArchiveSettings};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    System,
    Light,
    #[default]
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiIconSize {
    Compact,
    #[default]
    Standard,
    Large,
}

impl UiIconSize {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Standard, Self::Large];

    pub const fn scale(self) -> f32 {
        match self {
            Self::Compact => 0.8,
            Self::Standard => 1.0,
            Self::Large => 1.2,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Compact => "80%",
            Self::Standard => "100%",
            Self::Large => "120%",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamingMode {
    /// Regenerate the title from the latest exchange after every turn.
    Latest,
    /// Generate the title once from the first user message.
    #[default]
    FirstMessage,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiSettings {
    /// Installed data plugins stay enabled unless explicitly disabled.
    #[serde(default)]
    pub plugins: conduit::plugin_packages::Preferences,
    /// Optional, host-managed programmer encouragement panel and API preferences.
    #[serde(default)]
    pub encourager: EncouragerSettings,
    /// Missing fields in existing installations retain their original language.
    #[serde(default = "conduit::i18n::Locale::legacy")]
    pub language: conduit::i18n::Locale,
    /// Deliver toast notifications through the operating system when available.
    #[serde(default)]
    pub system_notifications: bool,
    #[serde(default)]
    pub terminal_profile: crate::terminal::ShellProfile,
    #[serde(default = "default_terminal_height")]
    pub terminal_height: f32,
    /// Session ids ordered by most recently opened.
    #[serde(default)]
    pub recent_sessions: Vec<u64>,
    /// Optional packed RGBA display colors keyed by stable workspace path.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub workspace_colors: HashMap<PathBuf, u32>,
    /// Pinned workspace paths; their relative order follows the saved manual order.
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub pinned_workspaces: HashSet<PathBuf>,
    /// User bindings by stable action ID; an empty list explicitly unbinds an action.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub shortcuts: std::collections::BTreeMap<String, Vec<String>>,
    /// How many recent sessions a workspace shows before "show more".
    #[serde(default = "default_sessions_per_workspace")]
    pub sessions_per_workspace: usize,
    #[serde(default)]
    pub default_session_permission: crate::domain::PermissionLevel,
    /// Engine used for the next new session (the composer's engine picker).
    #[serde(default)]
    pub last_provider: Option<crate::domain::Provider>,
    /// Shows the in-app performance monitor and collects its samples.
    #[serde(default = "default_performance_monitor")]
    pub performance_monitor: bool,
    /// Follow the system appearance or force light/dark mode.
    #[serde(default)]
    pub appearance: AppearanceMode,
    /// Shared visual scale for functional interface icons.
    #[serde(default)]
    pub icon_size: UiIconSize,
    #[serde(default = "default_light_theme")]
    pub light_theme: String,
    #[serde(default = "default_dark_theme")]
    pub dark_theme: String,
    #[serde(default)]
    pub background: crate::wallpaper::Settings,
    /// Collapse workspaces with no recent session activity.
    #[serde(default)]
    pub auto_collapse_inactive_workspaces: bool,
    /// Number of inactive days before a workspace is automatically collapsed.
    #[serde(default = "default_auto_collapse_inactive_days")]
    pub auto_collapse_inactive_days: u32,
    /// Display running workspaces first, then the most recently active ones.
    #[serde(default = "default_auto_sort_workspaces")]
    pub auto_sort_workspaces: bool,
    /// Fill the available chat area instead of a centered 840px column.
    #[serde(default = "default_full_width_content")]
    pub full_width_content: bool,
    /// Open new read-only files beside Main. Missing preferences keep full-page navigation.
    #[serde(default)]
    pub files_open_beside: bool,
    /// Preferred Main share of the two-column content area, saved after a drag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_split_ratio: Option<f32>,
    /// Collapse directory chains that contain only another directory in the file tree.
    #[serde(default = "default_compact_middle_packages")]
    pub compact_middle_packages: bool,
    /// Keep tabs from the same workspace adjacent in the document tab bar.
    #[serde(default = "default_group_tabs")]
    pub group_tabs: bool,
    /// Initial visibility of CLI thinking; individual blocks can override it.
    #[serde(default = "default_collapse_reasoning")]
    pub collapse_reasoning: bool,
    /// Initial visibility of completed question records; individual records can override it.
    #[serde(default)]
    pub auto_collapse_question_history: bool,
    /// What automatic titling reacts to.
    #[serde(default)]
    pub naming_mode: NamingMode,
    #[serde(default = "default_auto_rename")]
    pub auto_rename: bool,
    /// Remove stale, unused sessions that have no conversation history.
    #[serde(default)]
    pub auto_cleanup_empty_sessions: bool,
    /// Automatic archival policy and durable daily-check/restoration state.
    #[serde(default)]
    pub auto_archive: AutoArchiveSettings,
    /// CLI engine used to auto-name sessions; `None` follows the session's
    /// own engine.
    #[serde(default)]
    pub naming_provider: Option<crate::domain::Provider>,
    /// Model override for session naming; `None` follows the session model
    /// (or the CLI default when a naming engine is pinned).
    #[serde(default)]
    pub naming_model: Option<String>,
    /// Independent naming route; None follows the session only when no engine is pinned.
    #[serde(default)]
    pub naming_channel: Option<crate::providers::ModelChannel>,
    /// Maximum number of session tabs kept open; opening more closes the oldest.
    #[serde(default = "default_max_tabs")]
    pub max_tabs: usize,
    /// Application chrome font; `None` uses the bundled theme's default.
    #[serde(default)]
    pub ui_font_family: Option<String>,
    /// Base interface font size; chat and code keep their own pixel sizes.
    #[serde(default = "default_ui_font_size")]
    pub ui_font_size: f32,
    /// Chat body font; `None` uses the default independently of the UI font.
    #[serde(default)]
    pub chat_font_family: Option<String>,
    /// Chat body font size in pixels.
    #[serde(default = "default_chat_font_size")]
    pub chat_font_size: f32,
    /// Chat body line height multiplier.
    #[serde(default = "default_chat_line_height")]
    pub chat_line_height: f32,
    /// Font for fenced code blocks and tool output; `None` follows the
    /// platform monospace font.
    #[serde(default)]
    pub code_font_family: Option<String>,
    /// Code block font size in pixels.
    #[serde(default = "default_code_font_size")]
    pub code_font_size: f32,
    /// Shows a copy button in the corner of each fenced code block.
    #[serde(default = "default_code_copy_button")]
    pub code_copy_button: bool,
    /// Colors fenced code blocks by language.
    #[serde(default = "default_code_syntax_highlight")]
    pub code_syntax_highlight: bool,
    /// Numbers the lines down the left gutter of each fenced code block.
    #[serde(default)]
    pub code_line_numbers: bool,
}

fn default_terminal_height() -> f32 {
    280.
}

fn default_light_theme() -> String {
    "Conduit Light".into()
}

fn default_dark_theme() -> String {
    "Conduit Dark".into()
}

pub const DEFAULT_UI_FONT_SIZE: f32 = 16.0;
pub const MIN_UI_FONT_SIZE: f32 = 12.0;
pub const MAX_UI_FONT_SIZE: f32 = 20.0;

pub const MIN_CHAT_FONT_SIZE: f32 = 12.0;
pub const MAX_CHAT_FONT_SIZE: f32 = 20.0;
pub const MIN_CHAT_LINE_HEIGHT: f32 = 1.2;
pub const MAX_CHAT_LINE_HEIGHT: f32 = 2.0;
pub const MIN_CODE_FONT_SIZE: f32 = 10.0;
pub const MAX_CODE_FONT_SIZE: f32 = 18.0;

fn default_sessions_per_workspace() -> usize {
    5
}

fn default_auto_sort_workspaces() -> bool {
    true
}

fn default_auto_collapse_inactive_days() -> u32 {
    1
}
fn default_full_width_content() -> bool {
    true
}

fn default_compact_middle_packages() -> bool {
    true
}

fn default_group_tabs() -> bool {
    true
}

fn default_collapse_reasoning() -> bool {
    false
}

fn default_ui_font_size() -> f32 {
    DEFAULT_UI_FONT_SIZE
}

fn default_chat_font_size() -> f32 {
    15.0
}

fn default_chat_line_height() -> f32 {
    1.5
}

fn default_code_font_size() -> f32 {
    13.0
}

fn default_code_copy_button() -> bool {
    true
}

fn default_code_syntax_highlight() -> bool {
    true
}

fn default_auto_rename() -> bool {
    true
}

fn default_performance_monitor() -> bool {
    true
}

/// Upper bound of the appearance setting for open session tabs.
pub const MAX_TAB_LIMIT: usize = 20;

fn default_max_tabs() -> usize {
    8
}

impl UiSettings {
    /// Ignore invalid preferences without losing the rest of the saved settings.
    pub fn content_split_ratio(&self) -> f32 {
        self.content_split_ratio
            .filter(|ratio| ratio.is_finite() && *ratio > 0. && *ratio < 1.)
            .unwrap_or(0.5)
    }

    fn legacy() -> Self {
        Self {
            language: conduit::i18n::Locale::legacy(),
            ..Self::default()
        }
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            plugins: Default::default(),
            encourager: Default::default(),
            language: Default::default(),
            system_notifications: false,
            terminal_profile: Default::default(),
            terminal_height: default_terminal_height(),
            recent_sessions: Vec::new(),
            workspace_colors: HashMap::new(),
            pinned_workspaces: HashSet::new(),
            shortcuts: Default::default(),
            sessions_per_workspace: default_sessions_per_workspace(),
            default_session_permission: Default::default(),
            last_provider: None,
            performance_monitor: default_performance_monitor(),
            appearance: AppearanceMode::default(),
            icon_size: UiIconSize::default(),
            light_theme: default_light_theme(),
            dark_theme: default_dark_theme(),
            background: Default::default(),
            auto_collapse_inactive_workspaces: false,
            auto_collapse_inactive_days: default_auto_collapse_inactive_days(),
            auto_sort_workspaces: true,
            full_width_content: default_full_width_content(),
            files_open_beside: false,
            content_split_ratio: None,
            compact_middle_packages: default_compact_middle_packages(),
            group_tabs: default_group_tabs(),
            collapse_reasoning: default_collapse_reasoning(),
            auto_collapse_question_history: false,
            max_tabs: default_max_tabs(),
            naming_mode: NamingMode::default(),
            auto_rename: true,
            auto_cleanup_empty_sessions: false,
            auto_archive: AutoArchiveSettings::default(),
            naming_provider: None,
            naming_model: None,
            naming_channel: None,
            ui_font_family: None,
            ui_font_size: default_ui_font_size(),
            chat_font_family: None,
            chat_font_size: default_chat_font_size(),
            chat_line_height: default_chat_line_height(),
            code_font_family: None,
            code_font_size: default_code_font_size(),
            code_copy_button: default_code_copy_button(),
            code_syntax_highlight: default_code_syntax_highlight(),
            code_line_numbers: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EncouragerSettings {
    pub enabled: bool,
    /// Show the optional floating pet while the encourager plugin is enabled.
    pub pet_enabled: bool,
    /// Last saved floating-pet position in screen coordinates.
    pub pet_position: Option<EncouragerPetPosition>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub persona: EncouragerPersona,
    pub verbosity: EncouragerVerbosity,
    pub include_context: bool,
}
impl Default for EncouragerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            pet_enabled: false,
            pet_position: None,
            provider_id: None,
            model: None,
            persona: EncouragerPersona::GentleMentor,
            verbosity: EncouragerVerbosity::Normal,
            include_context: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncouragerPetPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncouragerPersona {
    #[default]
    GentleMentor,
    HypeCoach,
    DryCoworker,
    Cat,
}
impl EncouragerPersona {
    pub const ALL: [Self; 4] = [
        Self::GentleMentor,
        Self::HypeCoach,
        Self::DryCoworker,
        Self::Cat,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::GentleMentor => "encourager.persona.gentle_mentor",
            Self::HypeCoach => "encourager.persona.hype_coach",
            Self::DryCoworker => "encourager.persona.dry_coworker",
            Self::Cat => "encourager.persona.cat",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncouragerVerbosity {
    Short,
    #[default]
    Normal,
    Chatty,
}
impl EncouragerVerbosity {
    pub const ALL: [Self; 3] = [Self::Short, Self::Normal, Self::Chatty];
    pub fn label(self) -> &'static str {
        match self {
            Self::Short => "encourager.verbosity.short",
            Self::Normal => "encourager.verbosity.normal",
            Self::Chatty => "encourager.verbosity.chatty",
        }
    }
    pub fn max_chars(self) -> usize {
        match self {
            Self::Short => 40,
            Self::Normal => 80,
            Self::Chatty => 120,
        }
    }
}

pub const DEFAULT_COMMIT_PROMPT: &str = "根据 Git 已暂存变更生成一条简洁准确的中文提交说明。第一行为 conventional commit 风格的标题；必要时空一行后用简短条目补充原因。不要虚构测试结果。";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagIncrement {
    #[default]
    Patch,
    Minor,
    Major,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitSettings {
    #[serde(default)]
    pub auto_fetch_incoming: bool,
    #[serde(default = "default_tag_step")]
    pub tag_step: usize,
    #[serde(default)]
    pub tag_increment: TagIncrement,
    #[serde(default = "default_commit_prompt")]
    pub commit_message_prompt: String,
    /// CLI engine used to generate commit messages; `None` follows the
    /// currently selected session.
    #[serde(default)]
    pub generation_provider: Option<crate::domain::Provider>,
    /// Model override for commit message generation; `None` uses the CLI
    /// default (or the session model when following the session).
    #[serde(default)]
    pub generation_model: Option<String>,
    /// Independent commit-generation route, without changing global CLI settings.
    #[serde(default)]
    pub generation_channel: Option<crate::providers::ModelChannel>,
    #[serde(default)]
    pub favorite_branches: std::collections::HashMap<PathBuf, Vec<String>>,
}

fn default_tag_step() -> usize {
    1
}

fn default_commit_prompt() -> String {
    DEFAULT_COMMIT_PROMPT.into()
}

impl Default for GitSettings {
    fn default() -> Self {
        Self {
            auto_fetch_incoming: false,
            tag_increment: TagIncrement::default(),
            tag_step: default_tag_step(),
            commit_message_prompt: conduit::i18n::text_for(
                &conduit::i18n::Locale::default(),
                "store.generate_a_concise_accurate_english_commit_message",
            ),
            generation_provider: None,
            generation_model: None,
            generation_channel: None,
            favorite_branches: Default::default(),
        }
    }
}

impl GitSettings {
    fn legacy() -> Self {
        Self {
            commit_message_prompt: default_commit_prompt(),
            ..Self::default()
        }
    }
    pub fn generation_prompt(&self, files: &[&str], diff: &str) -> String {
        let instruction = self.commit_message_prompt.trim();
        let instruction = if instruction.is_empty() {
            conduit::i18n::text_static("store.generate_a_concise_accurate_english_commit_message")
        } else {
            instruction
        };
        conduit::i18n::format(
            "store.return_only_a_ready_to_use_commit",
            &[
                ("instruction", instruction.to_string()),
                ("arg0", files.join("\n").to_string()),
                ("diff", diff.to_string()),
            ],
        )
    }
}

/// Unsent follow-ups are persisted separately from the sent conversation history.
#[derive(Clone, Serialize, Deserialize)]
pub struct QueuedMessage {
    pub text: String,
    pub references: Vec<crate::file_ref::FileReference>,
    pub paused: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub child_bindings: std::collections::HashMap<u64, crate::subagents::ChildBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_commands: Vec<crate::subagents::PendingChildCommand>,
    pub version: u32,
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub reviews: crate::review::Reviews,
    #[serde(default)]
    pub workflow: crate::workflow::Workflow,
    #[serde(default)]
    pub usage_ledger: crate::usage_ledger::Ledger,
    #[serde(default)]
    pub scheduler: crate::automation::Scheduler,
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub queued_messages: std::collections::HashMap<u64, QueuedMessage>,
    #[serde(default)]
    pub cli: crate::cli::CliSettings,
    #[serde(default = "UiSettings::legacy")]
    pub ui: UiSettings,
    #[serde(default)]
    pub updates: conduit::update::Preferences,
    #[serde(default = "GitSettings::legacy")]
    pub git: GitSettings,
    #[serde(default, alias = "custom_providers")]
    pub providers: Vec<crate::providers::ModelProvider>,
    /// Last successful import, independent of external configuration lifetimes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ccs_catalog: Option<crate::ccs::CcsCatalog>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub cli_model_catalogs: HashMap<crate::domain::Provider, crate::models::ModelCatalog>,
    #[serde(default)]
    pub session_defaults: std::collections::HashMap<crate::domain::Provider, SessionDefaults>,
    /// Maps workspace paths to history file stamps, allowing startup imports to skip unchanged files.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub history_file_stamps: std::collections::HashMap<
        PathBuf,
        std::collections::HashMap<PathBuf, crate::domain::HistoryFileStamp>,
    >,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionDefaults {
    #[serde(default)]
    pub channel: crate::providers::ModelChannel,
    #[serde(default)]
    pub model: Option<String>,
}

impl Store {
    #[cfg(test)]
    pub fn load_ccs_catalog(&mut self) -> crate::ccs::CcsCatalog {
        if let Some(catalog) = &self.ccs_catalog {
            return catalog.clone();
        }
        let catalog = crate::ccs::load();
        self.sync_ccs_providers(&catalog);
        catalog
    }

    pub fn load_model_catalog(
        &mut self,
        engine: crate::domain::Provider,
    ) -> crate::models::ModelCatalog {
        if let Some(catalog) = self.cli_model_catalogs.get(&engine) {
            return catalog.clone();
        }
        self.cache_model_catalog(engine, crate::models::load(engine))
    }

    pub fn cache_model_catalog(
        &mut self,
        engine: crate::domain::Provider,
        catalog: crate::models::ModelCatalog,
    ) -> crate::models::ModelCatalog {
        if !engine.descriptor().capabilities.model_services {
            return catalog;
        }
        if catalog.error.is_some() || catalog.incomplete {
            if let Some(cached) = self.cli_model_catalogs.get(&engine) {
                let mut effective = cached.clone();
                effective.incomplete = true;
                effective.error = catalog.error;
                effective.note = catalog.note;
                return effective;
            }
        } else {
            self.cli_model_catalogs.insert(engine, catalog.clone());
        }
        catalog
    }

    /// Merge imported providers by stable origin id and migrate the earlier
    /// CCS-only session selection to the shared provider registry.
    pub fn sync_ccs_providers(&mut self, catalog: &crate::ccs::CcsCatalog) {
        self.merge_ccs_providers(catalog, true);
    }

    /// An explicit refresh adopts the current CCS catalog, including removals
    /// and hidden models, instead of carrying forward a startup discovery cache.
    pub fn refresh_ccs_providers(&mut self, catalog: &crate::ccs::CcsCatalog) {
        self.merge_ccs_providers(catalog, false);
    }

    fn merge_ccs_providers(&mut self, catalog: &crate::ccs::CcsCatalog, keep_models: bool) {
        if catalog.error.is_none() && catalog.database_path.is_file() {
            self.providers.retain(|p| match &p.source {
                crate::providers::ProviderSource::Local => true,
                crate::providers::ProviderSource::Ccs { id, .. } => catalog
                    .providers
                    .iter()
                    .any(|entry| entry.id == *id && entry.provider == p.engine),
            });
        }
        if catalog.error.is_none()
            && (catalog.database_path.is_file() || !catalog.providers.is_empty())
        {
            self.ccs_catalog = Some(catalog.clone());
        }
        for mut imported in catalog.imported_providers() {
            if let Some(provider) = self.providers.iter_mut().find(|p| p.id == imported.id) {
                if matches!(
                    provider.source,
                    crate::providers::ProviderSource::Ccs { .. }
                ) {
                    // CCS may only contain a default model or a partial catalog.
                    // Keep the durable discovery cache for the same endpoint,
                    // while allowing current CCS metadata to refresh known IDs.
                    if keep_models
                        && provider.base_url.trim_end_matches('/')
                            == imported.base_url.trim_end_matches('/')
                    {
                        for model in &provider.models {
                            if !imported.models.contains(model) {
                                imported.models.push(model.clone());
                            }
                        }
                        for metadata in &provider.model_metadata {
                            if !imported.models.contains(&metadata.id) {
                                continue;
                            }
                            if let Some(current) = imported
                                .model_metadata
                                .iter_mut()
                                .find(|model| model.id == metadata.id)
                            {
                                if current.context_window.is_none() {
                                    current.context_window = metadata.context_window;
                                }
                                if current.label == current.id {
                                    current.label = metadata.label.clone();
                                }
                                if current.note.is_empty() {
                                    current.note = metadata.note.clone();
                                }
                            } else {
                                imported.model_metadata.push(metadata.clone());
                            }
                        }
                    }
                    *provider = imported;
                }
            } else {
                self.providers.push(imported);
            }
        }
        for session in self.workspaces.iter_mut().flat_map(|w| &mut w.sessions) {
            if session.provider == crate::domain::Provider::OpenCode
                && session.channel != crate::providers::ModelChannel::Cli
            {
                // OpenCode owns its provider routing; old CCS/custom defaults
                // cannot be interpreted as OpenCode provider/model IDs.
                session.channel = crate::providers::ModelChannel::Cli;
                session.model = None;
                session.reasoning_effort = Default::default();
                session.opencode = Default::default();
            }
            if let crate::providers::ModelChannel::Ccs(id) = &session.channel {
                session.channel = crate::providers::ModelChannel::Provider(
                    crate::providers::ModelProvider::ccs_id(session.provider, id),
                );
            }
        }
    }

    pub fn path() -> PathBuf {
        let directory = std::env::var_os("CONDUIT_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                // UI tests invoke the same persistence paths as user actions.
                // Never let their synthetic sessions replace real app state.
                #[cfg(test)]
                {
                    std::env::temp_dir().join(format!("conduit-tests-{}", std::process::id()))
                }
                // Shared with crash.log. Keeps cargo run separate from the
                // installed app, including both its state files and the
                // data-directory instance lock.
                #[cfg(not(test))]
                conduit::diagnostics::default_data_dir()
            });
        // Detached saves can outlive a test even with --test-threads=1. Give
        // each test thread its own files, including under an explicit data root.
        // Persistence captures this path before handing it to its worker.
        #[cfg(test)]
        let directory = directory.join(format!(
            "test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        directory.join("state.json")
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        Self::load_migrating_from(&path)
    }
    pub(crate) fn load_migrating_from(path: &Path) -> Result<Self> {
        let state = Self::load_from(path)?;
        if state.version < 3 && path.exists() {
            // Keep the pre-migration manifest recoverable until the new format is durable.
            let backup = path.with_extension("v2.json");
            if !backup.exists() {
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&backup)?;
                use std::io::Write;
                output.write_all(&fs::read(path)?)?;
                output.sync_all()?;
            }
            state.save_to(path)?;
            return Self::load_from(path);
        }
        Ok(state)
    }
    pub(crate) fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                child_bindings: Default::default(),
                child_commands: Default::default(),
                version: 2,
                workspaces: vec![],
                scheduler: Default::default(),
                usage_ledger: Default::default(),
                workflow: Default::default(),
                reviews: Default::default(),
                queued_messages: Default::default(),
                cli: Default::default(),
                ui: Default::default(),
                updates: Default::default(),
                git: Default::default(),
                providers: vec![],
                ccs_catalog: None,
                cli_model_catalogs: HashMap::new(),
                session_defaults: Default::default(),
                history_file_stamps: Default::default(),
            });
        }
        let mut state: Self = serde_json::from_slice(&fs::read(path)?).with_context(|| {
            conduit::i18n::format(
                "store.unable_to_read_original_file_will_not",
                &[("arg0", format!("{}", path.display()))],
            )
        })?;
        if !matches!(state.version, 1..=4) {
            bail!(conduit::i18n::format(
                "store.unsupported_data_version",
                &[("arg0", format!("{}", state.version))]
            ));
        }
        let legacy = state.version == 1;
        let mut paths = HashSet::new();
        let mut usage_path_aliases = HashMap::new();
        let mut sessions = HashSet::new();
        for workspace in &mut state.workspaces {
            if !workspace.path.is_absolute() {
                bail!(conduit::i18n::text("store.workspace_path_must_be_absolute"));
            }
            if let Ok(path) = workspace.path.canonicalize() {
                if workspace.path != path {
                    usage_path_aliases.insert(workspace.path.clone(), path.clone());
                }
                // Older manifests may already have a verbatim Windows workspace path
                // while their usage ledger still contains the ordinary path.
                #[cfg(windows)]
                usage_path_aliases.insert(
                    crate::workspace_files::display_path(&path).into(),
                    path.clone(),
                );
                if workspace.path != path
                    && let Some(stamps) = state.history_file_stamps.remove(&workspace.path)
                {
                    state
                        .history_file_stamps
                        .entry(path.clone())
                        .or_insert(stamps);
                }
                workspace.path = path;
            }
            if !paths.insert(workspace.path.clone()) {
                bail!(conduit::i18n::text(
                    "store.duplicate_workspace_directories_fix_state_json"
                ));
            }
            for session in &mut workspace.sessions {
                crate::subagents::disconnect(&mut session.subagents, None, false);
                if session.id == 0 || session.id == u64::MAX || !sessions.insert(session.id) {
                    bail!(conduit::i18n::text(
                        "store.invalid_or_duplicate_session_ids_fix_state"
                    ));
                }
                if state.version >= 3 {
                    if !session.messages.is_empty() {
                        bail!(conduit::i18n::text(
                            "store.session_contains_both_messages_and_a_reference"
                        ));
                    }
                    session
                        .transcript
                        .bind(&crate::transcript::directory(path))?;
                    continue;
                }
                if legacy {
                    session.title = conduit::i18n::format(
                        "store.legacy_terminal_session",
                        &[("arg0", format!("{}", session.id))],
                    );
                    session.messages.push(crate::domain::ChatItem::from(
                        crate::domain::ChatItemData {
                            reasoning_summary: None,
                            elapsed_ms: None,
                            key: "legacy-migration".into(),
                            created_at: 0,
                            kind: crate::domain::ItemKind::Notice,
                            text: conduit::i18n::text_static(
                                "store.migrated_from_the_terminal_prototype_only_metadata",
                            )
                            .into(),
                            input: String::new(),
                            detail: String::new(),
                            status: Default::default(),
                            approval: None,
                            model: None,
                        },
                    ));
                }
                restore_session(session);
            }
        }
        // Pins use the same canonical identity as workspaces, including Windows
        // display-path aliases. Ignore records for workspaces no longer present.
        state.ui.pinned_workspaces = state
            .ui
            .pinned_workspaces
            .into_iter()
            .map(|path| usage_path_aliases.get(&path).cloned().unwrap_or(path))
            .filter(|path| paths.contains(path))
            .collect();
        for workspace in state
            .usage_ledger
            .observations
            .iter_mut()
            .map(|turn| &mut turn.workspace)
            .chain(
                state
                    .usage_ledger
                    .budgets
                    .iter_mut()
                    .map(|budget| &mut budget.workspace),
            )
        {
            if let Some(path) = usage_path_aliases.get(workspace.as_path()) {
                workspace.clone_from(path);
            }
        }
        state.version = state.version.max(2);
        Ok(state)
    }
    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        fs::create_dir_all(path.parent().unwrap())?;
        crate::transcript::register_manifest(path);
        let mut manifest = self.clone();
        // v4 roots may reference pages/chunks; older binaries must refuse them
        // rather than run v3's single-blob garbage collector on their children.
        manifest.version = 4;
        let directory = crate::transcript::directory(path);
        for workspace in &mut manifest.workspaces {
            for session in &mut workspace.sessions {
                let reference = crate::transcript::save(session, &directory)?;
                session.messages.clear();
                session.transcript.reference = reference;
            }
        }
        let temp = path.with_extension("json.tmp");
        let mut file = fs::File::create(&temp)?;
        use std::io::Write;
        {
            let mut output = std::io::BufWriter::new(&mut file);
            serde_json::to_writer(&mut output, &manifest)?;
            output.flush()?;
        }
        file.sync_all()?;
        drop(file);
        replace_file(&temp, path)?;
        // The manifest still owns all current references until collection finishes.
        crate::transcript::collect_periodically(&directory);
        Ok(())
    }
}

pub(crate) fn replace_file(temp: &Path, path: &Path) -> Result<()> {
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
    {
        fs::rename(temp, path)?;
        fs::File::open(path.parent().unwrap())?.sync_all()?;
    }
    Ok(())
}

pub(crate) fn restore_session(session: &mut crate::domain::SessionRecord) {
    crate::subagents::disconnect(&mut session.subagents, None, false);
    if session.interrupted {
        for item in &mut session.messages {
            if let Some(approval) = item.approval.as_mut()
                && approval.status == crate::message_status::ApprovalStatus::Waiting
            {
                approval.status = "已中断".into();
            }
            if item.kind == crate::domain::ItemKind::Tool
                && matches!(
                    item.status,
                    crate::message_status::ToolStatus::Running
                        | crate::message_status::ToolStatus::Output
                )
            {
                item.status = "已中断".into();
            }
        }
        session
            .messages
            .push(crate::domain::ChatItem::from(crate::domain::ChatItemData {
                reasoning_summary: None,
                elapsed_ms: None,
                key: format!("restart-{}", session.messages.len()),
                created_at: 0,
                kind: crate::domain::ItemKind::Notice,
                text: conduit::i18n::text_static("store.the_previous_run_was_interrupted_and_was")
                    .into(),
                input: String::new(),
                detail: String::new(),
                status: Default::default(),
                approval: None,
                model: None,
            }));
        session.interrupted = false;
    }
    session.migrate_approval_notices();
    dedupe_restored_messages(session);
}

/// Older builds keyed Claude stream deltas by the raw stream block index while
/// the assistant snapshot keyed by the visible block index. A hidden thinking
/// block shifted one side by one, and the same answer was stored twice. Repair
/// that on load: drop adjacent assistant blocks with identical text and failure
/// notices already carried by the preceding assistant text.
fn dedupe_restored_messages(session: &mut crate::domain::SessionRecord) {
    let mut kept: Vec<crate::domain::ChatItem> = Vec::with_capacity(session.messages.len());
    for mut item in session.messages.drain(..) {
        if item.kind == crate::domain::ItemKind::Assistant
            && crate::failure::is_engine_error_text(&item.text)
        {
            item.kind = crate::domain::ItemKind::Error;
            if item.detail.is_empty() {
                item.detail = item.text.clone();
            }
        }
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
                    matches!(
                        previous.kind,
                        crate::domain::ItemKind::Assistant | crate::domain::ItemKind::Error
                    ) && same_failure_text(&previous.text, error)
                })
            };
        if !duplicate_assistant && !duplicate_failure {
            kept.push(item);
        }
    }
    session.messages = kept;
    crate::failure::coalesce_restored_failures(session);
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
    fn remove_transcripts(path: &std::path::Path) {
        let directory = crate::transcript::directory(path);
        if let Ok(entries) = std::fs::read_dir(&directory) {
            for entry in entries {
                std::fs::remove_file(entry.unwrap().path()).unwrap();
            }
            std::fs::remove_dir(directory).unwrap();
        }
    }
    use super::*;
    use crate::domain::{Provider, SessionRecord};

    #[test]
    fn opencode_ccs_routes_migrate_to_native_without_changing_context() {
        let _locale = crate::test_support::chinese();
        use crate::providers::ModelChannel;
        let mut session: SessionRecord = serde_json::from_value(serde_json::json!({
            "id": 7, "provider": "OpenCode", "created_at": 0, "exit_code": null,
            "native_id": "native-context", "channel": {"ccs": "old-default"},
            "model": "ccs-model", "reasoning_effort": "high"
        }))
        .unwrap();
        let catalog = crate::ccs::CcsCatalog {
            providers: vec![crate::ccs::CcsProvider {
                provider: Provider::OpenCode,
                name: "CCS default".into(),
                id: "old-default".into(),
                current: true,
                models: vec![],
                base_url: String::new(),
            }],
            ..Default::default()
        };
        let mut store = Store::default();
        store.workspaces.push(crate::domain::Workspace {
            path: PathBuf::from("C:\\work"),
            sessions: vec![session.clone()],
        });
        store.sync_ccs_providers(&catalog);
        assert!(
            store.providers.is_empty(),
            "OpenCode never imports CCS provider routes"
        );
        let restored = &store.workspaces[0].sessions[0];
        assert_eq!(restored.channel, ModelChannel::Cli);
        assert_eq!(restored.model, None);
        assert_eq!(restored.reasoning_effort, Default::default());
        assert_eq!(restored.native_id, session.native_id);
        session.channel = ModelChannel::Cli;
        session.model = Some("native/model".into());
        store.workspaces[0].sessions = vec![session];
        store.sync_ccs_providers(&catalog);
        assert_eq!(
            store.workspaces[0].sessions[0].model.as_deref(),
            Some("native/model"),
            "explicit native selections survive sync"
        );
    }

    #[test]
    fn ccs_sync_merges_into_provider_registry_and_migrates_session_selection() {
        let _locale = crate::test_support::chinese();
        use crate::providers::{ModelChannel, ModelProvider, ProviderSource};
        let entry = |engine, id: &str, name: &str, model: &str| crate::ccs::CcsProvider {
            provider: engine,
            id: id.into(),
            name: name.into(),
            current: false,
            base_url: "https://proxy.invalid/v1".into(),
            models: vec![crate::models::ModelOption {
                id: model.into(),
                label: model.into(),
                note: String::new(),
                context_window: Some(200_000),
            }],
        };
        let mut catalog = crate::ccs::CcsCatalog {
            providers: vec![
                entry(Provider::Claude, "same-id", "Claude proxy", "claude-model"),
                entry(Provider::Codex, "same-id", "Codex proxy", "codex-model"),
            ],
            ..Default::default()
        };
        let mut store = Store::default();
        store.providers.push(ModelProvider {
            id: "local".into(),
            name: "Local".into(),
            engine: Provider::Codex,
            base_url: "https://local.invalid/v1".into(),
            api_key_env: "MY_KEY".into(),
            protected_api_key: None,
            models: vec!["local-model".into()],
            source: ProviderSource::Local,
            model_metadata: vec![],
            claude_models: None,
        });
        let mut session: SessionRecord = serde_json::from_value(serde_json::json!({"id":1,"provider":"Codex","created_at":0,"exit_code":null,"channel":{"ccs":"same-id"}})).unwrap();
        store.workspaces.push(crate::domain::Workspace {
            path: PathBuf::from("C:\\work"),
            sessions: vec![session.clone()],
        });
        store.sync_ccs_providers(&catalog);
        store.sync_ccs_providers(&catalog);
        assert_eq!(store.providers.len(), 3);
        assert_eq!(store.providers[0].name, "Local");
        assert_ne!(store.providers[1].id, store.providers[2].id);
        assert_eq!(store.providers[2].name, "Codex proxy");
        assert_eq!(
            store.providers[2].catalog()[0].context_window,
            Some(200_000)
        );
        assert_eq!(
            store.workspaces[0].sessions[0].channel,
            ModelChannel::Provider(ModelProvider::ccs_id(Provider::Codex, "same-id"))
        );
        catalog.providers[1].name = "Updated proxy".into();
        store.sync_ccs_providers(&catalog);
        assert_eq!(store.providers.len(), 3);
        assert_eq!(store.providers[2].name, "Updated proxy");
        let mut legacy = serde_json::to_value(&store).unwrap();
        let providers = legacy.as_object_mut().unwrap().remove("providers").unwrap();
        legacy["custom_providers"] = providers;
        let restored: Store = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.providers.len(), 3);
        session.channel = serde_json::from_value(serde_json::json!({"custom":"local"})).unwrap();
        assert_eq!(session.channel, ModelChannel::Provider("local".into()));
    }
    #[test]
    fn cli_catalog_snapshot_survives_missing_external_files_and_failed_refresh() {
        use crate::models;
        let _locale = crate::test_support::chinese();
        let directory = std::env::temp_dir().join(format!(
            "conduit-cli-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let codex = directory.join(".codex");
        fs::create_dir_all(&codex).unwrap();
        fs::write(
            codex.join("config.toml"),
            "model = \"snapshot-model\"\nmodel_catalog_json = \"ccs-models.json\"\n",
        )
        .unwrap();
        fs::write(codex.join("ccs-models.json"), serde_json::json!({"models": [
            {"slug": "snapshot-model", "display_name": "Snapshot Model", "context_window": 256000},
            {"slug": "secondary-model", "display_name": "Secondary Model", "context_window": 1000000},
            {"slug": "hidden-model", "visibility": "hide"}
        ]}).to_string()).unwrap();
        let mut store = Store::default();
        let expected = store.cache_model_catalog(
            Provider::Codex,
            models::load_from(Provider::Codex, &directory),
        );
        assert_eq!(expected.models.len(), 2);
        fs::remove_file(codex.join("ccs-models.json")).unwrap();
        let state_path = directory.join("state.json");
        store.save_to(&state_path).unwrap();
        let mut restarted = Store::load_from(&state_path).unwrap();
        let cached = restarted.load_model_catalog(Provider::Codex);
        assert_eq!(cached.models, expected.models);
        assert_eq!(cached.current, expected.current);
        // A readable config pointing at a vanished generated catalog is still a failed refresh.
        let partial = models::load_from(Provider::Codex, &directory);
        assert!(partial.note.is_some());
        let effective = restarted.cache_model_catalog(Provider::Codex, partial);
        assert!(effective.note.is_some());
        assert_eq!(effective.models, expected.models);
        for invalid_catalog in ["not json", "{}"] {
            fs::write(codex.join("ccs-models.json"), invalid_catalog).unwrap();
            let invalid = models::load_from(Provider::Codex, &directory);
            assert!(invalid.incomplete);
            assert_eq!(
                restarted
                    .cache_model_catalog(Provider::Codex, invalid)
                    .models,
                expected.models
            );
        }
        fs::remove_file(codex.join("ccs-models.json")).unwrap();
        fs::write(codex.join("config.toml"), "invalid toml").unwrap();
        let effective = restarted.cache_model_catalog(
            Provider::Codex,
            models::load_from(Provider::Codex, &directory),
        );
        assert!(effective.error.is_some());
        assert_eq!(effective.models, expected.models);
        fs::remove_file(codex.join("config.toml")).unwrap();
        let effective = restarted.cache_model_catalog(
            Provider::Codex,
            models::load_from(Provider::Codex, &directory),
        );
        assert_eq!(effective.models, expected.models);
        assert!(
            restarted
                .load_model_catalog(Provider::Codex)
                .error
                .is_none()
        );
        // A successful explicit reread replaces the snapshot, including genuine removals.
        fs::write(codex.join("config.toml"), "model = \"replacement-model\"\n").unwrap();
        let updated = restarted.cache_model_catalog(
            Provider::Codex,
            models::load_from(Provider::Codex, &directory),
        );
        assert_eq!(updated.models.len(), 1);
        assert_eq!(updated.current.as_deref(), Some("replacement-model"));
        restarted.save_to(&state_path).unwrap();
        assert_eq!(
            Store::load_from(&state_path)
                .unwrap()
                .load_model_catalog(Provider::Codex)
                .models,
            updated.models
        );
        fs::write(
            codex.join("config.toml"),
            "model_catalog_json = \"ccs-models.json\"\n",
        )
        .unwrap();
        fs::write(codex.join("ccs-models.json"), r#"{"models":[]}"#).unwrap();
        let empty = models::load_from(Provider::Codex, &directory);
        assert!(empty.models.is_empty());
        assert!(!empty.incomplete);
        restarted.cache_model_catalog(Provider::Codex, empty);
        assert!(
            restarted
                .load_model_catalog(Provider::Codex)
                .models
                .is_empty()
        );
    }

    #[test]
    fn ccs_sync_preserves_saved_model_catalog_after_restart_and_refresh() {
        let _locale = crate::test_support::chinese();
        let mut catalog = crate::ccs::CcsCatalog {
            providers: vec![crate::ccs::CcsProvider {
                provider: Provider::Codex,
                id: "cached-provider".into(),
                name: "Proxy".into(),
                current: true,
                base_url: "https://proxy.invalid/v1".into(),
                models: vec![crate::models::ModelOption {
                    id: "default-model".into(),
                    label: "Default model".into(),
                    note: String::new(),
                    context_window: Some(128_000),
                }],
            }],
            ..Default::default()
        };
        let mut store = Store::default();
        store.sync_ccs_providers(&catalog);
        let provider = &mut store.providers[0];
        provider.models.push("discovered-model".into());
        provider.model_metadata.push(crate::models::ModelOption {
            id: "discovered-model".into(),
            label: "Discovered model".into(),
            note: "Cached discovery".into(),
            context_window: Some(1_000_000),
        });
        // The manifest contains the refreshed catalog, but CCS may only return
        // its default model when the app next starts or reloads configuration.
        let path = Store::path();
        store.save_to(&path).unwrap();
        let mut restored = Store::load_from(&path).unwrap();
        catalog.providers[0].name = "Renamed proxy".into();
        catalog.providers[0].models[0].context_window = Some(256_000);
        restored.sync_ccs_providers(&catalog);
        restored.sync_ccs_providers(&catalog);
        let provider = &restored.providers[0];
        assert_eq!(provider.name, "Renamed proxy");
        assert_eq!(provider.models, ["default-model", "discovered-model"]);
        let models = provider.catalog();
        assert_eq!(models[0].context_window, Some(256_000));
        assert_eq!(models[1].label, "Discovered model");
        assert_eq!(models[1].note, "Cached discovery");
        assert_eq!(models[1].context_window, Some(1_000_000));
        // An intentional refresh must still honor CCS removals/hidden models.
        let mut explicitly_refreshed = restored.clone();
        explicitly_refreshed.refresh_ccs_providers(&catalog);
        assert_eq!(explicitly_refreshed.providers[0].models, ["default-model"]);
        // A different endpoint must not inherit the previous service's cache.
        catalog.providers[0].base_url = "https://replacement.invalid/v1".into();
        restored.sync_ccs_providers(&catalog);
        assert_eq!(restored.providers[0].models, ["default-model"]);
    }

    #[test]
    fn legacy_migration_preserves_identity_without_fabricating_context() {
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
            crate::transcript::load(
                Store::load_from(&path).unwrap().workspaces[0].sessions[0].clone()
            )
            .unwrap()
            .messages
            .len(),
            3
        );
        remove_transcripts(&path);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn atomic_replacement_and_corruption_preservation() {
        let _locale = crate::test_support::chinese();
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
            child_bindings: Default::default(),
            child_commands: Default::default(),
            version: 2,
            scheduler: Default::default(),
            usage_ledger: Default::default(),
            workflow: Default::default(),
            reviews: Default::default(),
            queued_messages: Default::default(),
            cli: Default::default(),
            ui: Default::default(),
            updates: Default::default(),
            git: Default::default(),
            providers: vec![],
            ccs_catalog: None,
            cli_model_catalogs: HashMap::new(),
            session_defaults: Default::default(),
            history_file_stamps: Default::default(),
            workspaces: vec![Workspace {
                path: directory.clone(),
                sessions: vec![],
            }],
        };
        store.save_to(&path).unwrap();
        store.workspaces[0].sessions.push(SessionRecord {
            task_list: None,
            subagents: Default::default(),
            is_subagent: false,
            branching: Default::default(),
            transcript: Default::default(),
            id: 1,
            provider: Provider::Codex,
            created_at: 1,
            exit_code: Some(7),
            title: "Test".into(),
            archived: false,
            pinned: true,
            native_id: None,
            model: None,
            messages: vec![],
            interrupted: false,
            channel: Default::default(),
            reasoning_effort: Default::default(),
            opencode: Default::default(),
            permission: crate::domain::PermissionLevel::default(),
            usage: crate::domain::Usage {
                context_window: 100_000,
                context_tokens: Some(36_000),
                ..Default::default()
            },
        });
        store.usage_ledger.record(crate::usage_ledger::Observation {
            key: "roundtrip".into(),
            session: 1,
            workspace: directory.clone(),
            provider: Provider::Codex,
            service: "cli".into(),
            service_name: None,
            model: None,
            finished_at: 1,
            input: Some(1_000),
            cached: None,
            cache_write: None,
            output: Some(500),
            elapsed_ms: 12_000,
            incomplete: false,
            api_first_response_ms: None,
            api_request_outcome: None,
        });
        store
            .cli
            .set(Provider::Claude, Some(directory.join("custom-claude.exe")));
        store.save_to(&path).unwrap();
        let restored = Store::load_from(&path).unwrap();
        assert_eq!(restored.cli, store.cli);
        assert_eq!(restored.workspaces[0].sessions[0].exit_code, Some(7));
        assert!(restored.workspaces[0].sessions[0].pinned);
        assert_eq!(
            restored.workspaces[0].sessions[0].usage.context_tokens,
            Some(36_000)
        );
        let turn = &restored.usage_ledger.observations[0];
        assert_eq!(turn.workspace, restored.workspaces[0].path);
        assert_eq!(turn.session, restored.workspaces[0].sessions[0].id);
        assert_eq!(turn.output, Some(500));
        fs::write(&path, b"invalid json").unwrap();
        assert!(Store::load_from(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid json");
        // Only this test's exact files are removed; do not recursively remove a computed directory.
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn duplicate_session_ids_are_rejected() {
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
    #[test]
    fn restored_approval_notices_move_to_the_preceding_tool_and_roundtrip() {
        let _locale = crate::test_support::chinese();
        let directory =
            std::env::temp_dir().join(format!("conduit-approval-migration-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("state.json");
        let value = serde_json::json!({"version":2,"workspaces":[{"path":directory,"sessions":[
            {"id":1,"provider":"Claude","created_at":0,"exit_code":null,"interrupted":true,"messages":[
                {"key":"turn-0-a","kind":"Tool","text":"WebSearch","status":"完成"},
                {"key":"turn-0-approval-1","kind":"Notice","text":"已提交允许本次操作：请求使用 WebSearch"},
                {"key":"turn-0-b","kind":"Tool","text":"WebSearch","status":"失败"},
                {"key":"turn-0-approval-3","kind":"Notice","text":"已提交拒绝本次操作：请求使用 WebSearch"},
                {"key":"turn-1-c","kind":"Tool","text":"Bash","status":"运行中","approval":{"request":"ticket","status":"等待中"}}
            ]}
        ]}]});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let restored = Store::load_from(&path).unwrap();
        let messages = &restored.workspaces[0].sessions[0].messages;
        assert_eq!(messages.len(), 4); // Three tools and the existing restart notice.
        for (item, state) in messages.iter().zip(["已允许", "已拒绝", "已中断"]) {
            assert_eq!(item.kind, crate::domain::ItemKind::Tool);
            assert_eq!(item.approval.as_ref().unwrap().status, state);
        }
        restored.save_to(&path).unwrap();
        let reloaded = Store::load_from(&path).unwrap();
        assert_eq!(
            serde_json::to_value(&restored.workspaces[0].sessions[0].messages).unwrap(),
            serde_json::to_value(
                crate::transcript::load(reloaded.workspaces[0].sessions[0].clone())
                    .unwrap()
                    .messages
            )
            .unwrap()
        );
        remove_transcripts(&path);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[test]
    fn ui_settings_default_to_five_sessions_and_roundtrip() {
        let _locale = crate::test_support::chinese();
        let value = serde_json::json!({"version":2,"workspaces":[]});
        let path = std::env::temp_dir().join(format!(
            "conduit-ui-settings-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut store = Store::load_from(&path).unwrap();
        assert!(!store.ui.encourager.enabled);
        assert!(!store.ui.encourager.pet_enabled);
        assert_eq!(store.ui.encourager.pet_position, None);
        assert!(!store.ui.encourager.include_context);
        assert_eq!(store.ui.sessions_per_workspace, 5);
        assert_eq!(store.ui.last_provider, None);
        assert!(store.ui.performance_monitor);
        assert_eq!(store.ui.appearance, AppearanceMode::Dark);
        assert_eq!(store.ui.light_theme, "Conduit Light");
        assert_eq!(store.ui.dark_theme, "Conduit Dark");
        assert!(!store.ui.auto_collapse_inactive_workspaces);
        assert_eq!(store.ui.auto_collapse_inactive_days, 1);
        assert_eq!(UiSettings::default().auto_collapse_inactive_days, 1);
        assert_eq!(
            serde_json::from_value::<UiSettings>(serde_json::json!({}))
                .unwrap()
                .auto_collapse_inactive_days,
            1
        );
        assert!(store.ui.auto_sort_workspaces);
        assert!(
            serde_json::from_value::<UiSettings>(serde_json::json!({}))
                .unwrap()
                .auto_sort_workspaces
        );
        assert!(
            !serde_json::from_value::<UiSettings>(serde_json::json!({}))
                .unwrap()
                .auto_collapse_inactive_workspaces
        );
        assert_eq!(store.ui.max_tabs, 8);
        assert!(store.ui.full_width_content);
        assert!(!store.ui.files_open_beside);
        assert_eq!(store.ui.content_split_ratio(), 0.5);
        assert!(store.ui.compact_middle_packages);
        assert!(store.ui.group_tabs);
        assert!(!store.ui.collapse_reasoning);
        assert!(!store.ui.auto_collapse_question_history);
        assert!(
            !serde_json::from_value::<UiSettings>(serde_json::json!({}))
                .unwrap()
                .auto_collapse_question_history
        );
        assert!(
            !serde_json::from_value::<UiSettings>(serde_json::json!({}))
                .unwrap()
                .collapse_reasoning
        );
        assert_eq!(store.ui.naming_mode, NamingMode::FirstMessage);
        assert!(!store.ui.auto_cleanup_empty_sessions);
        assert_eq!(store.ui.ui_font_family, None);
        assert_eq!(store.ui.ui_font_size, DEFAULT_UI_FONT_SIZE);
        assert_eq!(store.ui.chat_font_family, None);
        assert_eq!(store.ui.chat_font_size, 15.0);
        assert_eq!(store.ui.chat_line_height, 1.5);
        assert_eq!(store.ui.code_font_family, None);
        assert_eq!(store.ui.code_font_size, 13.0);
        assert!(store.ui.code_copy_button);
        assert!(store.ui.code_syntax_highlight);
        assert!(!store.ui.code_line_numbers);
        assert_eq!(store.git.commit_message_prompt, DEFAULT_COMMIT_PROMPT);
        store.ui.sessions_per_workspace = 8;
        store.ui.encourager.enabled = true;
        store.ui.encourager.pet_enabled = true;
        store.ui.encourager.pet_position = Some(EncouragerPetPosition { x: 1280, y: 720 });
        store.ui.encourager.provider_id = Some("encourager-test".into());
        store.ui.encourager.model = Some("encourager-model".into());
        store.ui.encourager.persona = EncouragerPersona::Cat;
        store.ui.encourager.verbosity = EncouragerVerbosity::Chatty;
        store.ui.encourager.include_context = true;
        store.ui.last_provider = Some(crate::domain::Provider::Codex);
        store.ui.performance_monitor = false;
        store.ui.appearance = AppearanceMode::System;
        store.ui.light_theme = "Catppuccin Latte".into();
        store.ui.dark_theme = "Tokyo Night".into();
        store.ui.auto_collapse_inactive_workspaces = true;
        store.ui.auto_collapse_inactive_days = 7;
        store.ui.auto_sort_workspaces = false;
        store.ui.max_tabs = 12;
        store.ui.full_width_content = false;
        store.ui.files_open_beside = true;
        store.ui.content_split_ratio = Some(0.65);
        store.ui.compact_middle_packages = false;
        store.ui.group_tabs = false;
        store.ui.collapse_reasoning = true;
        store.ui.auto_collapse_question_history = true;
        store.ui.naming_mode = NamingMode::Latest;
        store.ui.auto_cleanup_empty_sessions = true;
        store.ui.ui_font_family = Some("Microsoft YaHei".into());
        store.ui.ui_font_size = 18.0;
        store.ui.chat_font_family = Some("Segoe UI".into());
        store.ui.chat_font_size = 17.0;
        store.ui.chat_line_height = 1.7;
        store.ui.code_font_family = Some("Cascadia Mono".into());
        store.ui.code_font_size = 14.0;
        store.ui.code_copy_button = false;
        store.ui.code_syntax_highlight = false;
        store.ui.code_line_numbers = true;
        store.git.commit_message_prompt =
            "Write an English subject followed by a body.\nKeep it under 72 columns.".into();
        store.save_to(&path).unwrap();
        let reloaded = Store::load_from(&path).unwrap();
        assert!(reloaded.ui.auto_cleanup_empty_sessions);
        assert!(reloaded.ui.encourager.enabled);
        assert!(reloaded.ui.encourager.pet_enabled);
        assert_eq!(
            reloaded.ui.encourager.pet_position,
            Some(EncouragerPetPosition { x: 1280, y: 720 })
        );
        assert_eq!(
            reloaded.ui.encourager.provider_id.as_deref(),
            Some("encourager-test")
        );
        assert_eq!(
            reloaded.ui.encourager.model.as_deref(),
            Some("encourager-model")
        );
        assert_eq!(reloaded.ui.encourager.persona, EncouragerPersona::Cat);
        assert_eq!(
            reloaded.ui.encourager.verbosity,
            EncouragerVerbosity::Chatty
        );
        assert!(reloaded.ui.encourager.include_context);
        assert_eq!(reloaded.ui.sessions_per_workspace, 8);
        assert_eq!(
            reloaded.git.commit_message_prompt,
            store.git.commit_message_prompt
        );
        assert_eq!(
            reloaded.ui.last_provider,
            Some(crate::domain::Provider::Codex)
        );
        assert!(!reloaded.ui.performance_monitor);
        assert_eq!(reloaded.ui.appearance, AppearanceMode::System);
        assert_eq!(reloaded.ui.light_theme, "Catppuccin Latte");
        assert_eq!(reloaded.ui.dark_theme, "Tokyo Night");
        assert!(reloaded.ui.auto_collapse_inactive_workspaces);
        assert_eq!(reloaded.ui.auto_collapse_inactive_days, 7);
        assert!(!reloaded.ui.auto_sort_workspaces);
        assert_eq!(reloaded.ui.max_tabs, 12);
        assert!(!reloaded.ui.full_width_content);
        assert!(reloaded.ui.files_open_beside);
        assert_eq!(reloaded.ui.content_split_ratio, Some(0.65));
        assert!(!reloaded.ui.compact_middle_packages);
        assert!(!reloaded.ui.group_tabs);
        assert!(reloaded.ui.collapse_reasoning);
        assert!(reloaded.ui.auto_collapse_question_history);
        assert_eq!(reloaded.ui.naming_mode, NamingMode::Latest);
        assert_eq!(
            reloaded.ui.ui_font_family.as_deref(),
            Some("Microsoft YaHei")
        );
        assert_eq!(reloaded.ui.ui_font_size, 18.0);
        assert_eq!(reloaded.ui.chat_font_family.as_deref(), Some("Segoe UI"));
        assert_eq!(reloaded.ui.chat_font_size, 17.0);
        assert_eq!(reloaded.ui.chat_line_height, 1.7);
        assert_eq!(
            reloaded.ui.code_font_family.as_deref(),
            Some("Cascadia Mono")
        );
        assert_eq!(reloaded.ui.code_font_size, 14.0);
        assert!(!reloaded.ui.code_copy_button);
        assert!(!reloaded.ui.code_syntax_highlight);
        assert!(reloaded.ui.code_line_numbers);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn background_generation_routes_and_session_speed_roundtrip() {
        let _locale = crate::test_support::chinese();
        use crate::{
            domain::{Provider, Usage},
            providers::ModelChannel,
        };
        let legacy: Store =
            serde_json::from_str(r#"{"version":2,"workspaces":[],"ui":{},"git":{}}"#).unwrap();
        assert_eq!(legacy.ui.naming_channel, None);
        assert_eq!(legacy.git.generation_channel, None);
        let mut store = Store::default();
        store.ui.naming_provider = Some(Provider::Claude);
        store.ui.naming_channel = Some(ModelChannel::Provider("title-route".into()));
        store.ui.naming_model = Some("title-model".into());
        store.git.generation_provider = Some(Provider::OpenCode);
        store.git.generation_channel = Some(ModelChannel::Cli);
        store.git.generation_model = Some("native/commit-model".into());
        let restored: Store =
            serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(restored.ui.naming_channel, store.ui.naming_channel);
        assert_eq!(restored.ui.naming_model, store.ui.naming_model);
        assert_eq!(
            restored.git.generation_channel,
            store.git.generation_channel
        );
        assert_eq!(restored.git.generation_model, store.git.generation_model);
        let mut session: crate::domain::SessionRecord =
            serde_json::from_str(r#"{"id":1,"provider":"Claude","created_at":0,"exit_code":null}"#)
                .unwrap();
        session.usage = Usage {
            session_output_tokens: 500,
            session_elapsed_ms: 5_000,
            ..Default::default()
        };
        let restored: crate::domain::SessionRecord =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(restored.usage.average_output_speed(), Some(100));
    }

    #[test]
    fn incoming_auto_fetch_is_opt_in_and_persists() {
        let mut settings: GitSettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.auto_fetch_incoming);
        settings.auto_fetch_incoming = true;
        let encoded = serde_json::to_string(&settings).unwrap();
        let saved: GitSettings = serde_json::from_str(&encoded).unwrap();
        assert!(saved.auto_fetch_incoming);
    }

    #[test]
    fn commit_prompt_uses_saved_preferences_and_appends_staged_context() {
        let _locale = crate::test_support::chinese();
        let mut settings = GitSettings {
            auto_fetch_incoming: false,
            tag_increment: TagIncrement::Patch,
            tag_step: 1,
            commit_message_prompt: "Use English and include a concise body.".into(),
            generation_provider: None,
            generation_model: None,
            generation_channel: None,
            favorite_branches: Default::default(),
        };
        let prompt = settings.generation_prompt(&["src/中文.rs"], "+a staged change");
        assert!(prompt.starts_with(&settings.commit_message_prompt));
        assert!(!prompt.contains(DEFAULT_COMMIT_PROMPT));
        assert!(prompt.contains("src/中文.rs\n<staged-diff>\n+a staged change\n</staged-diff>"));
        settings.commit_message_prompt = " \n ".into();
        assert!(
            settings
                .generation_prompt(&[], "")
                .starts_with(DEFAULT_COMMIT_PROMPT)
        );
        let restored: GitSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(restored.commit_message_prompt, DEFAULT_COMMIT_PROMPT);
        assert_eq!(restored.tag_increment, TagIncrement::Patch);
        assert_eq!(restored.tag_step, 1);
        settings.tag_step = 10;
        let roundtrip: GitSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(roundtrip.tag_step, 10);
    }
    #[test]
    fn content_split_ratio_defaults_and_rejects_invalid_preferences() {
        let old: UiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.content_split_ratio, None);
        assert_eq!(old.content_split_ratio(), 0.5);
        for value in [
            serde_json::Value::Null,
            (-0.5).into(),
            0.into(),
            1.into(),
            2.into(),
        ] {
            let settings: UiSettings = serde_json::from_value(serde_json::json!({
                "content_split_ratio": value,
                "files_open_beside": true,
            }))
            .unwrap();
            assert_eq!(settings.content_split_ratio(), 0.5);
            assert!(settings.files_open_beside);
        }
        for ratio in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let settings = UiSettings {
                content_split_ratio: Some(ratio),
                ..Default::default()
            };
            assert_eq!(settings.content_split_ratio(), 0.5);
        }
    }

    #[test]
    fn workspace_colors_default_empty_and_roundtrip_by_path() {
        let old: UiSettings = serde_json::from_str("{}").unwrap();
        assert!(old.workspace_colors.is_empty());

        let path = PathBuf::from("/workspace/project");
        let settings = UiSettings {
            workspace_colors: HashMap::from([(path.clone(), 0x2a7fffff)]),
            ..Default::default()
        };
        let restored: UiSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.workspace_colors.get(&path), Some(&0x2a7fffff));
    }

    #[test]
    fn pinned_workspaces_default_empty_and_roundtrip_by_path() {
        let old: UiSettings = serde_json::from_str("{}").unwrap();
        assert!(old.pinned_workspaces.is_empty());
        assert!(
            serde_json::to_value(&old)
                .unwrap()
                .get("pinned_workspaces")
                .is_none()
        );
        let pins = HashSet::from([PathBuf::from("/a/project"), PathBuf::from("/b/project")]);
        let settings = UiSettings {
            pinned_workspaces: pins.clone(),
            ..Default::default()
        };
        let restored: UiSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored.pinned_workspaces, pins);
    }

    #[test]
    fn pinned_workspaces_follow_canonical_paths_and_drop_removed_records_on_reload() {
        let directory =
            std::env::temp_dir().join(format!("conduit-workspace-pins-{}", std::process::id()));
        fs::create_dir_all(directory.join("child")).unwrap();
        let path = directory.join("state.json");
        let alias = directory.join("child").join("..");
        let missing = directory.join("removed");
        let value = serde_json::json!({
            "version": 2,
            "workspaces": [{"path": alias, "sessions": []}],
            "ui": {"pinned_workspaces": [alias, missing]}
        });
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let restored = Store::load_from(&path).unwrap();
        let canonical = directory.canonicalize().unwrap();
        assert_eq!(restored.workspaces[0].path, canonical);
        assert_eq!(restored.ui.pinned_workspaces, HashSet::from([canonical]));
        restored.save_to(&path).unwrap();
        let reloaded = Store::load_from(&path).unwrap();
        assert_eq!(reloaded.ui.pinned_workspaces, restored.ui.pinned_workspaces);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory.join("child")).unwrap();
        // Empty workspaces create no transcript blobs.
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn icon_size_defaults_for_existing_settings_and_roundtrips() {
        let old: UiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.icon_size, UiIconSize::Standard);

        for size in UiIconSize::ALL {
            let settings = UiSettings {
                icon_size: size,
                ..Default::default()
            };
            let encoded = serde_json::to_string(&settings).unwrap();
            let restored: UiSettings = serde_json::from_str(&encoded).unwrap();
            assert_eq!(restored.icon_size, size);
        }
    }

    #[test]
    fn default_session_permission_is_backward_compatible_and_persisted() {
        use crate::domain::PermissionLevel;
        let old: UiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            old.default_session_permission,
            PermissionLevel::WorkspaceWrite
        );
        for permission in [
            PermissionLevel::ReadOnly,
            PermissionLevel::WorkspaceWrite,
            PermissionLevel::FullAccess,
        ] {
            let settings = UiSettings {
                default_session_permission: permission,
                ..Default::default()
            };
            let restored: UiSettings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored.default_session_permission, permission);
        }
    }
    #[test]
    fn model_preferences_restore_old_defaults_and_roundtrip_new_routes() {
        let _locale = crate::test_support::chinese();
        use crate::providers::{ModelChannel, ModelProvider, ReasoningEffort};
        let ui: UiSettings = serde_json::from_str("{}").unwrap();
        assert!(ui.auto_rename);
        let mut session: crate::domain::SessionRecord = serde_json::from_value(
            serde_json::json!({"id":1,"provider":"Codex","created_at":0,"exit_code":null}),
        )
        .unwrap();
        let mut old = serde_json::to_value(&session).unwrap();
        old.as_object_mut().unwrap().remove("channel");
        old.as_object_mut().unwrap().remove("reasoning_effort");
        old.as_object_mut().unwrap().remove("opencode");
        let restored: crate::domain::SessionRecord = serde_json::from_value(old).unwrap();
        assert_eq!(restored.channel, ModelChannel::Cli);
        assert_eq!(restored.reasoning_effort, ReasoningEffort::Default);
        assert!(restored.opencode.variant.is_none());
        session.channel = ModelChannel::Provider("custom".into());
        session.reasoning_effort = ReasoningEffort::High;
        let mut store = Store::default();
        store.ui.auto_rename = false;
        store.providers.push(ModelProvider {
            id: "custom".into(),
            name: "proxy".into(),
            engine: crate::domain::Provider::Codex,
            base_url: "https://proxy.invalid/v1".into(),
            api_key_env: "MY_PROXY_KEY".into(),
            protected_api_key: None,
            models: vec!["custom-model".into()],
            source: Default::default(),
            model_metadata: vec![],
            claude_models: None,
        });
        store.workspaces.push(crate::domain::Workspace {
            path: PathBuf::from("C:\\work"),
            sessions: vec![session],
        });
        let restored: Store =
            serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert!(!restored.ui.auto_rename);
        assert_eq!(restored.providers[0].api_key_env, "MY_PROXY_KEY");
        assert_eq!(
            restored.workspaces[0].sessions[0].channel,
            ModelChannel::Provider("custom".into())
        );
        assert_eq!(
            restored.workspaces[0].sessions[0].reasoning_effort,
            ReasoningEffort::High
        );
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;
    #[test]
    fn fresh_and_legacy_language_preferences_round_trip_without_touching_fonts() {
        assert_eq!(Store::default().ui.language.as_str(), "en");
        assert!(
            GitSettings::default()
                .commit_message_prompt
                .contains("English")
        );
        let legacy: Store =
            serde_json::from_value(serde_json::json!({"version":2,"workspaces":[]})).unwrap();
        assert_eq!(legacy.ui.language.as_str(), "zh-CN");
        assert_eq!(legacy.git.commit_message_prompt, DEFAULT_COMMIT_PROMPT);
        let old: UiSettings = serde_json::from_value(
            serde_json::json!({"ui_font_family":"Custom CJK","chat_font_size":18}),
        )
        .unwrap();
        assert_eq!(old.language.as_str(), "zh-CN");
        assert!(!old.system_notifications);
        let mut saved = old;
        saved.system_notifications = true;
        saved.language = conduit::i18n::Locale::default();
        let restored: UiSettings =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.language.as_str(), "en");
        assert!(restored.system_notifications);
        assert_eq!(restored.ui_font_family.as_deref(), Some("Custom CJK"));
        assert_eq!(restored.chat_font_size, 18.);
        let unknown: UiSettings =
            serde_json::from_value(serde_json::json!({"language":"missing-language"})).unwrap();
        assert_eq!(unknown.language.as_str(), "en");
    }
}
