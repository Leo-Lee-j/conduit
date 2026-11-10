//! Application state and initialization. Views and workflows live in app/.

use crate::notifications::{
    Level as NoticeLevel, Notice, Presentation as NoticePresentation, Source as NoticeSource,
    Target as NoticeTarget,
};
use crate::{
    approval::ApprovalQueue,
    ccs::{self, CcsCatalog},
    chat_runtime::ChatRuntime,
    cli::{self, VersionProbe},
    domain::{ChatItem, ItemKind, PermissionLevel, Provider, SessionRecord, Usage, Workspace},
    file_ref::{self, FileReference, RejectedFile, display_path},
    git::{self, BranchProbe, GitProbe},
    models::{self, ModelCatalog},
    performance::PerformanceMonitor,
    protocol::Event,
    providers::{ModelChannel, ModelProvider, ProviderSource, ReasoningEffort, RuntimeRoute},
    store::{
        AppearanceMode, DEFAULT_UI_FONT_SIZE, MAX_CHAT_FONT_SIZE, MAX_CHAT_LINE_HEIGHT,
        MAX_CODE_FONT_SIZE, MAX_TAB_LIMIT, MAX_UI_FONT_SIZE, MIN_CHAT_FONT_SIZE,
        MIN_CHAT_LINE_HEIGHT, MIN_CODE_FONT_SIZE, MIN_UI_FONT_SIZE, NamingMode, Store,
    },
};
use chrono::{DateTime, Local};
use gpui_kit::assets::IconName as FullIconName;
use gpui_kit::{
    base::TextSelection,
    component::{
        Size,
        button::*,
        clipboard::Clipboard,
        diff::{
            Diff, DiffChangeIndicator, DiffFile, DiffHoverHighlight, DiffHunkSeparator, DiffMode,
            DiffState,
        },
        highlighter::HighlightTheme,
        input::{Input, InputEvent, InputState, MoveDown, MoveUp, Textarea, TextareaState},
        menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
        popover::Popover,
        scroll::ScrollableElement,
        searchable_list::{SearchableListItem, SearchableVec},
        select::{Select, SelectEvent, SelectState},
        slider::{Slider, SliderEvent, SliderState},
        spinner::Spinner,
        tab::{Tab, TabBar},
        text::{TextView, TextViewStyle},
        *,
    },
    prelude::FluentBuilder,
    *,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod background;
mod chat;
mod code_highlight;
mod commands;
mod components;
mod composer;
mod dock;
mod events;
mod file_links;
mod file_picker;
mod file_preview;
mod files;
mod generation;
mod git_branches;
mod git_compare;
mod git_conflicts;
mod git_diff;
mod git_log;
mod git_state;
mod git_ui;
mod localization;
mod mentions;
mod mermaid;
mod model_picker;
mod naming;
mod notifications;
mod numbered_code;
mod opencode_models;
mod outline;
mod performance_view;
mod plugin_ui;
mod queue;
mod runtime;
mod search;
mod search_worker;
mod session_controller;
mod sessions;
mod settings;
mod shell;
mod shortcuts;
mod sidebar;
mod state;
mod stream_events;
mod terminal;
mod theme;
mod tool_analysis;
mod tool_details;
mod toolbar;
mod tools;
mod transcripts;
mod typography;
mod update;
mod user_input;

use components::*;
use composer::*;
use events::*;
use files::DocumentTab;
use git_diff::*;
use outline::*;
#[cfg(test)]
use search::{highlight_ranges, search_snippet};
use sessions::*;
use settings::*;
use state::*;
use theme::*;
pub(crate) use theme::{install_theme_packs, set_palette};
use tools::*;
use typography::*;

/// Keep document and right-sidebar tab strips aligned in both layouts.
const TAB_BAR_HEIGHT: f32 = 36.;

actions!(
    conduit,
    [
        SendPrompt,
        StopReply,
        Newline,
        PreviousPrompt,
        NextPrompt,
        NextDiff,
        PreviousDiff,
        SubmitBranchForm,
        CancelBranchForm,
        PreviousFileMention,
        NextFileMention,
        AcceptFileMention,
        DismissFileMention,
        PreviousChatCommand,
        NextChatCommand,
        AcceptChatCommand,
        DismissChatCommand,
        CloseDocument,
        RecentSessions,
        OpenSessionSearch,
        OpenSettingsShortcut,
        FocusSettingsSearch,
        LeaveSettings,
        OpenFileSearchShortcut,
        ToggleGitNavigation,
        ToggleTerminal,
        NewTerminal,
        LeaveTerminal,
        NextTerminal,
        PreviousTerminal
    ]
);

pub struct ConduitApp {
    store: Store,
    background: background::BackgroundState,
    git: git_ui::GitUi,
    branch_manager: Option<Entity<git_log::GitLogView>>,
    persistence: crate::persistence::Persistence,
    attachments_cleanup_pending: bool,
    session_control: session_controller::SessionController,
    transcripts: transcripts::TranscriptUi,
    terminals: terminal::TerminalUi,
    updater: update::UpdateUi,
    notifications: notifications::NotificationUi,
    connection_settings: settings::ConnectionsUi,
    settings_ui: settings::SettingsUi,
    extensions_ui: settings::ExtensionsUi,
    writable: bool,
    selected_workspace: Option<usize>,
    selected_session: Option<u64>,
    /// Open chat and read-only file documents, ordered left to right.
    /// Selecting a tab keeps its position; only new tabs append at the end.
    open_tabs: Vec<DocumentTab>,
    active_file: Option<u64>,
    active_diff: Option<u64>,
    files: files::FileBrowser,
    plugins: plugin_ui::PluginUi,
    /// The open tab under the pointer, so the strip's right-click menu knows
    /// which tab it was opened on.
    tab_menu_target: Option<DocumentTab>,
    input: Entity<TextareaState>,
    drafts: HashMap<u64, String>,
    workspace_drafts: HashMap<PathBuf, String>,
    /// While the composer shows text recalled with Up/Down, its rank among
    /// the selected conversation's sent prompts counting from the newest
    /// (0). None means the composer holds typed text, not a recalled prompt.
    prompt_history: Option<usize>,
    composer_files: HashMap<ComposerTarget, ComposerFiles>,
    file_mentions: mentions::FileMentions,
    chat_commands: commands::ChatCommands,
    queued_messages: HashMap<u64, queue::QueuedMessage>,
    probes: HashMap<Provider, VersionProbe>,
    diagnostics: HashMap<Provider, String>,
    catalogs: HashMap<Provider, ModelCatalog>,
    opencode_catalogs: HashMap<PathBuf, ModelCatalog>,
    opencode_model_probes: HashMap<PathBuf, models::OpenCodeModelsProbe>,
    ccs: CcsCatalog,
    message: Option<String>,
    git_prompt_input: Entity<TextareaState>,
    /// Font pickers rebuilt from installed families when the app starts.
    ui_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    chat_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    code_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    /// The theme's original UI family, also the independent chat default.
    default_ui_font: SharedString,
    /// The theme's platform monospace family, restored when no code font is set.
    default_mono_font: SharedString,
    next_id: u64,
    closing: bool,
    tool_details: tool_details::ToolDetailState,
    /// Measured width of the left workspace sidebar, used to cap the right one at
    /// half of the remaining content area.
    sidebar_width: f32,
    /// Workspaces whose session list was expanded past the configured limit.
    expanded_workspaces: HashSet<usize>,
    /// Workspace session lists explicitly collapsed in the sidebar.
    collapsed_workspaces: HashSet<PathBuf>,
    /// Workspace paths whose CLI history is being imported in the background.
    history_loading: HashSet<PathBuf>,
    /// Startup page state; `Some` from before the first frame until the
    /// initial history scan merges (or the poll timeout gives up on it).
    startup: Option<StartupState>,
    /// Sidebar session search filter.
    search: Entity<InputState>,
    search_query: String,
    search_ui: search_worker::SearchUi,
    double_shift: file_picker::DoubleShift,
    show_archived: bool,
    /// Engine used by the sidebar + button and the search dialog's empty state.
    new_session_provider: Provider,
    pending_model_selection: Option<ComposerSelection>,
    pending_permission: PermissionLevel,
    rename_target: Option<u64>,
    rename_input: Option<Entity<InputState>>,
    delete_target: Option<u64>,
    remove_workspace_target: Option<usize>,
    _subscriptions: Vec<Subscription>,
    dirty: bool,
    last_save: Instant,
    scroll: gpui_kit::ListState,
    timeline: Entity<shell::ChatTimeline>,
    sidebar_view: Entity<sidebar::SidebarView>,
    sidebar: sidebar::SidebarState,
    file_view: Entity<files::FileDocumentView>,
    /// Focus target for the conversation, so a finished selection drag leaves the
    /// composer and Ctrl+C reaches the window root's copy handler.
    chat_focus: FocusHandle,
    timeline_session: Option<u64>,
    chat_rows: chat::rows::Rows,
    outline: MessageOutlineCache,
    /// Whether the message list is scrolled to (or near) its end. Drives the
    /// floating "jump to latest" arrow so it only shows once the user scrolls up.
    at_bottom: bool,
    reading_positions: HashMap<u64, (String, Pixels, bool)>,
    restore_reading_position: bool,
    /// Scoped by session and tool key, independent of the right detail dock.
    expanded_tools: HashSet<(u64, String)>,
    /// Groups manually pinned open after the assistant starts replying.
    tool_group_expansion: HashMap<(u64, String), bool>,
    /// Explicit choices survive streaming and changes to the default visibility.
    reasoning_expansion: HashMap<(u64, String), bool>,
    expanded_errors: HashSet<(u64, String)>,
    /// Last elapsed-label repaint: every second for turns/recent ages, otherwise every minute.
    last_live_tick: Instant,
    tool_scrolls: HashMap<(u64, String), ScrollHandle>,
    /// Parsed diff viewers for expanded edit cards, keyed like `tool_scrolls`.
    tool_diffs: DiffCache,
    tool_analysis: tool_analysis::Cache,
    message_focus: Option<(u64, String, Instant)>,
    pending_message_jump: Option<usize>,
    performance: Entity<PerformanceMonitor>,
    performance_open: bool,
    /// Message action currently showing its "copied" check mark, keyed by id.
    copied_action: Option<SharedString>,
    /// Bumped on every copy so stale timers cannot clear a newer feedback.
    copied_action_seq: u64,
}

impl EventEmitter<crate::providers::ProviderCatalogChanged> for ConduitApp {}

impl ConduitApp {
    #[cfg(any(test, feature = "ui-tests"))]
    #[allow(dead_code)] // exercised by the feature-gated UI tests; main loads the store once.
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        start_in_settings: bool,
        start_in_git: bool,
    ) -> Self {
        let loaded = Store::load().map_err(|error| format!("{error:#}"));
        Self::with_store(window, cx, loaded, start_in_settings, start_in_git)
    }

    pub(crate) fn with_store(
        window: &mut Window,
        cx: &mut Context<Self>,
        loaded: std::result::Result<Store, String>,
        start_in_settings: bool,
        start_in_git: bool,
    ) -> Self {
        file_preview::init(cx);
        cx.bind_keys([
            KeyBinding::new("enter", SubmitBranchForm, Some("GitBranchForm > Input")),
            KeyBinding::new("enter", SubmitBranchForm, Some("GitBranchForm")),
            KeyBinding::new("escape", CancelBranchForm, Some("GitBranchForm > Input")),
            KeyBinding::new("escape", CancelBranchForm, Some("GitBranchForm")),
            // Up/Down recall sent prompts when the composer is empty, like a
            // shell. The composer-depth binding wins over the textarea's own
            // "Input" keys and forwards the caret movement whenever there is
            // nothing to recall.
            KeyBinding::new("up", PreviousPrompt, Some("ConduitComposer > Input")),
            KeyBinding::new("down", NextPrompt, Some("ConduitComposer > Input")),
            KeyBinding::new("up", PreviousPrompt, Some("ConduitComposer")),
            KeyBinding::new("down", NextPrompt, Some("ConduitComposer")),
            KeyBinding::new("up", PreviousFileMention, Some("FileMention > Input")),
            KeyBinding::new("down", NextFileMention, Some("FileMention > Input")),
            KeyBinding::new("tab", AcceptFileMention, Some("FileMention > Input")),
            KeyBinding::new("escape", DismissFileMention, Some("FileMention > Input")),
            KeyBinding::new("up", PreviousChatCommand, Some("ChatCommand > Input")),
            KeyBinding::new("down", NextChatCommand, Some("ChatCommand > Input")),
            KeyBinding::new("tab", AcceptChatCommand, Some("ChatCommand > Input")),
            KeyBinding::new("enter", SendPrompt, Some("ChatCommand > Input")),
            KeyBinding::new("escape", DismissChatCommand, Some("ChatCommand > Input")),
        ]);
        terminal::init(cx);
        let (mut store, message, writable) = match loaded {
            Ok(store) => (store, None, true),
            Err(error) => (Store::default(), Some(error), false),
        };
        shortcuts::sanitize(&mut store.ui.shortcuts);
        store.ui.background.sanitize();
        theme::set_background_visible(false);
        shortcuts::install(&store.ui.shortcuts, cx);
        install_theme_packs(cx);
        let initial_mode = match store.ui.appearance {
            AppearanceMode::System => gpui_kit::component::ThemeMode::from(window.appearance()),
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
        };
        set_palette(initial_mode);
        gpui_kit::component::Theme::change(initial_mode, Some(window), cx);
        // GPUI panics while laying out a family the machine does not have, so a
        // saved family that has since been uninstalled falls back to the default.
        let mut installed_fonts = cx.text_system().all_font_names();
        installed_fonts.sort_by_key(|name| name.to_lowercase());
        installed_fonts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
        for family in [
            &mut store.ui.ui_font_family,
            &mut store.ui.chat_font_family,
            &mut store.ui.code_font_family,
        ] {
            if family
                .as_ref()
                .is_some_and(|name| !installed_fonts.iter().any(|installed| installed == name))
            {
                *family = None;
            }
        }
        let default_ui_font = gpui_kit::component::Theme::global(cx).font_family.clone();
        store.ui.ui_font_size = store
            .ui
            .ui_font_size
            .clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE);
        install_ui_font(
            cx,
            store.ui.ui_font_family.as_deref(),
            &default_ui_font,
            store.ui.ui_font_size,
        );
        let default_mono_font = gpui_kit::component::Theme::global(cx)
            .mono_font_family
            .clone();
        install_code_font(
            cx,
            store.ui.code_font_family.as_deref(),
            &default_mono_font,
            store.ui.code_font_size,
        );
        let font_choices = |default_label: &'static str| {
            std::iter::once(FontChoice::default_entry(default_label))
                .chain(installed_fonts.iter().map(|name| FontChoice {
                    label: SharedString::from(name.clone()),
                    value: Some(SharedString::from(name.clone())),
                }))
                .collect::<Vec<_>>()
        };
        let font_index = |family: &Option<String>| {
            family
                .as_ref()
                .and_then(|family| installed_fonts.iter().position(|name| name == family))
                .map_or(0, |index| index + 1)
        };
        let ui_font_index = font_index(&store.ui.ui_font_family);
        let chat_font_index = font_index(&store.ui.chat_font_family);
        let code_font_index = font_index(&store.ui.code_font_family);
        let ui_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(font_choices(conduit::i18n::text_static(
                    "app.default_system_interface",
                ))),
                Some(IndexPath::new(ui_font_index)),
                window,
                cx,
            )
            .searchable(true)
        });
        let chat_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(font_choices(conduit::i18n::text_static(
                    "app.default_system_body",
                ))),
                Some(IndexPath::new(chat_font_index)),
                window,
                cx,
            )
            .searchable(true)
        });
        let code_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(font_choices(conduit::i18n::text_static(
                    "app.default_system_monospace",
                ))),
                Some(IndexPath::new(code_font_index)),
                window,
                cx,
            )
            .searchable(true)
        });
        let ui_font_subscription =
            cx.subscribe(&ui_font_select, |this, _, event: &SelectEvent<_>, cx| {
                if !this.writable || this.closing {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                this.store.ui.ui_font_family = value
                    .as_ref()
                    .and_then(|family| family.as_ref())
                    .map(|family| family.to_string());
                this.apply_ui_font(cx);
                this.persist();
                cx.notify();
            });
        let chat_font_subscription =
            cx.subscribe(&chat_font_select, |this, _, event: &SelectEvent<_>, cx| {
                if !this.writable || this.closing {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                this.store.ui.chat_font_family = value
                    .as_ref()
                    .and_then(|family| family.as_ref())
                    .map(|family| family.to_string());
                this.persist();
                cx.notify();
            });
        let code_font_subscription =
            cx.subscribe(&code_font_select, |this, _, event: &SelectEvent<_>, cx| {
                if !this.writable || this.closing {
                    return;
                }
                let SelectEvent::Confirm(value) = event;
                this.store.ui.code_font_family = value
                    .as_ref()
                    .and_then(|family| family.as_ref())
                    .map(|family| family.to_string());
                this.apply_code_font(cx);
                this.persist();
                cx.notify();
            });
        let next_id = store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .map(|s| s.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        for workspace in &mut store.workspaces {
            workspace
                .sessions
                .sort_by_key(|session| std::cmp::Reverse((session.last_activity_at(), session.id)));
        }
        let selected_workspace = (!store.workspaces.is_empty()).then_some(0);
        let selected_session = store
            .workspaces
            .first()
            .and_then(|w| w.sessions.iter().find(|session| !session.archived))
            .map(|s| s.id);
        let new_session_provider = store
            .ui
            .last_provider
            .or_else(|| {
                selected_session.and_then(|id| {
                    store
                        .workspaces
                        .iter()
                        .flat_map(|w| &w.sessions)
                        .find(|s| s.id == id)
                        .map(|s| s.provider)
                })
            })
            .unwrap_or(Provider::Claude);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(conduit::i18n::text_static(
                    "localization.describe_a_task_type_to_reference_files",
                ))
                .auto_grow(3, 8)
        });
        let input_subscription = cx.subscribe(&input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        // Selection changes also matter: a query belongs to the caret, not to
        // the last @ somewhere in the draft. Ignore unchanged blink notifications.
        let mention_subscription = cx.observe(&input, |this, _, cx| {
            this.refresh_chat_commands(cx);
            this.refresh_file_mentions(cx);
        });
        let shift_subscription = cx.observe_window_activation(window, |this, _, _| {
            this.double_shift = file_picker::DoubleShift::default();
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("localization.search_sessions"))
        });
        let search_subscription = cx.subscribe(&search, |this, search, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = search.read(cx).value().to_string();
                this.refresh_search(cx, true);
                cx.notify();
            }
        });
        let settings_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("localization.search_settings"))
        });
        let model_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(conduit::i18n::text_static(
                "localization.search_model_names_ids_or_notes",
            ))
        });
        let model_search_subscription =
            cx.subscribe(&model_search, |app, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    app.connection_settings.model_limit = 50;
                    cx.notify();
                }
            });
        let settings_search_subscription =
            cx.subscribe(&settings_search, |_, _, _: &InputEvent, cx| cx.notify());
        let skill_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(conduit::i18n::text_static(
                "localization.search_names_or_descriptions",
            ))
        });
        let mcp_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("localization.search_mcp_names"))
        });
        let skill_search_subscription =
            cx.subscribe(&skill_search, |app, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    app.extensions_ui.lists[0].reset(0);
                    cx.notify();
                }
            });
        let mcp_search_subscription =
            cx.subscribe(&mcp_search, |app, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    app.extensions_ui.lists[1].reset(0);
                    cx.notify();
                }
            });
        let git_prompt_input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder(conduit::i18n::text_static(
                    "localization.enter_the_commit_message_generation_prompt",
                ))
                .auto_grow(8, 8);
            state.set_value(store.git.commit_message_prompt.clone(), window, cx);
            state
        });
        let git_prompt_subscription =
            cx.subscribe(&git_prompt_input, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && this.writable && !this.closing {
                    let value = input.read(cx).value().to_string();
                    if this.store.git.commit_message_prompt != value {
                        this.store.git.commit_message_prompt = value;
                        this.dirty = true;
                    }
                    cx.notify();
                }
            });
        if selected_session.is_some() {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }
        if start_in_settings {
            cx.defer_in(window, |this, window, cx| {
                this.open_settings(window, cx);
            });
        }
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |this, cx| this.request_close(window, cx))
                .unwrap_or(true)
        });
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let appearance_subscription = cx.observe_window_appearance(window, |this, window, cx| {
            if this.store.ui.appearance == AppearanceMode::System {
                this.apply_appearance(window, cx);
            }
        });
        let ccs = ccs::load();
        store.sync_ccs_providers(&ccs);
        let scroll = gpui_kit::ListState::new(0, ListAlignment::Bottom, px(240.));
        let app = cx.entity().downgrade();
        let timeline = cx.new(|_| shell::ChatTimeline::new(app.clone(), scroll.clone()));
        let sidebar_view = cx.new(|cx| sidebar::SidebarView::new(app.clone(), window, cx));
        let file_view = cx.new(|cx| files::FileDocumentView::new(app, cx));
        let mut app = Self {
            background: background::BackgroundState::new(cx),
            persistence: crate::persistence::Persistence::new(Store::path()),
            attachments_cleanup_pending: false,
            session_control: session_controller::SessionController::default(),
            transcripts: Default::default(),
            terminals: Default::default(),
            store,
            git: git_ui::GitUi::default(),
            branch_manager: None,
            updater: update::UpdateUi::default(),
            notifications: notifications::NotificationUi::new(writable, cx),
            connection_settings: settings::ConnectionsUi::new(new_session_provider, model_search),
            settings_ui: settings::SettingsUi {
                open: false,
                focus_before_open: None,
                focus: cx.focus_handle(),
                page: 0,
                group: None,
                search: settings_search,
                naming_expanded: false,
                git_expanded: false,
                prompt_expanded: false,
            },
            extensions_ui: settings::ExtensionsUi::new(
                skill_search,
                mcp_search,
                new_session_provider,
            ),
            writable,
            selected_workspace,
            selected_session,
            open_tabs: selected_session
                .map(DocumentTab::Chat)
                .into_iter()
                .collect(),
            active_file: None,
            active_diff: None,
            files: files::FileBrowser::default(),
            plugins: plugin_ui::PluginUi::new(),
            tab_menu_target: None,
            input,
            drafts: HashMap::new(),
            workspace_drafts: HashMap::new(),
            prompt_history: None,
            composer_files: HashMap::new(),
            file_mentions: mentions::FileMentions::default(),
            chat_commands: commands::ChatCommands::default(),
            queued_messages: HashMap::new(),
            probes: HashMap::new(),
            diagnostics: HashMap::new(),
            catalogs: Provider::ALL
                .into_iter()
                .map(|provider| (provider, models::load(provider)))
                .collect(),
            opencode_catalogs: HashMap::new(),
            opencode_model_probes: HashMap::new(),
            ccs,
            message,
            git_prompt_input,
            ui_font_select,
            chat_font_select,
            code_font_select,
            default_ui_font,
            default_mono_font,
            next_id,
            closing: false,
            tool_details: tool_details::ToolDetailState::default(),
            sidebar_width: shell::NAVIGATION_DEFAULT_WIDTH,
            expanded_workspaces: HashSet::new(),
            collapsed_workspaces: HashSet::new(),
            history_loading: HashSet::new(),
            startup: None,
            search,
            search_query: String::new(),
            search_ui: search_worker::SearchUi::default(),
            double_shift: file_picker::DoubleShift::default(),
            show_archived: false,
            new_session_provider,
            pending_model_selection: None,
            pending_permission: PermissionLevel::default(),
            rename_target: None,
            rename_input: None,
            delete_target: None,
            remove_workspace_target: None,
            _subscriptions: vec![
                settings_search_subscription,
                skill_search_subscription,
                model_search_subscription,
                mcp_search_subscription,
                input_subscription,
                mention_subscription,
                search_subscription,
                shift_subscription,
                git_prompt_subscription,
                appearance_subscription,
                ui_font_subscription,
                chat_font_subscription,
                code_font_subscription,
            ],
            dirty: false,
            last_save: Instant::now(),
            scroll,
            timeline,
            sidebar_view,
            sidebar: sidebar::SidebarState::default(),
            file_view,
            chat_focus: cx.focus_handle(),
            timeline_session: None,
            chat_rows: Default::default(),
            outline: MessageOutlineCache::default(),
            at_bottom: true,
            reading_positions: HashMap::new(),
            restore_reading_position: false,
            expanded_tools: HashSet::new(),
            tool_group_expansion: HashMap::new(),
            reasoning_expansion: HashMap::new(),
            expanded_errors: HashSet::new(),
            last_live_tick: Instant::now(),
            tool_scrolls: HashMap::new(),
            tool_diffs: DiffCache::default(),
            tool_analysis: Default::default(),
            message_focus: None,
            pending_message_jump: None,
            performance: cx.new(|_| PerformanceMonitor::new()),
            performance_open: false,
            copied_action: None,
            copied_action_seq: 0,
        };
        app.queued_messages = std::mem::take(&mut app.store.queued_messages);
        let session_ids = app
            .store
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.sessions.iter().map(|session| session.id))
            .collect::<HashSet<_>>();
        app.queued_messages.retain(|id, _| session_ids.contains(id));
        for message in app.queued_messages.values_mut() {
            message.paused = Some(
                conduit::i18n::text_static("app.application_reopened_review_and_send_manually")
                    .into(),
            );
        }
        app.refresh_installed_plugins(cx);
        app.sync_plugin_context();
        app.restore_update_drafts(window, cx);
        app.show_installed_changelog(window, cx);
        if start_in_git {
            app.show_git_navigation(cx);
        }
        app.cleanup_attachments();
        app.ensure_selected_transcript(cx);
        app.restore_background(window, cx);
        app
    }

    /// Arms the startup page before the window's first frame. The initial
    /// history scan releases it from `poll` once merged (or after its
    /// timeout). No-op without workspaces: there is nothing to import, so
    /// the app opens directly.
    pub(crate) fn begin_startup_loading(&mut self) {
        if self.store.workspaces.is_empty() {
            return;
        }
        self.startup = Some(StartupState {
            started: Instant::now(),
            imports_done: false,
        });
    }

    pub(super) fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.confirm_terminal_exit(window, cx) {
            return false;
        }
        self.extensions_ui.probe = None;
        self.plugins.stop_instances();
        #[cfg(conduit_shell_desktop)]
        gpui_shell::ShellRoot::update(window, cx, |root, window, cx| {
            root.clear_stopped_overlays(window, cx);
        });
        if self.updater.exiting {
            return true;
        }
        if self.updater.preparing() {
            return false;
        }
        self.closing = true;
        self.terminals.stop_all();
        self.session_control.cancel_imports();
        self.history_loading.clear();
        if self.dirty {
            self.persist();
        }
        self.git.probe = None;
        self.git.branch_probe = None;
        if !self.session_control.runs.is_empty()
            || !self.probes.is_empty()
            || !self.opencode_model_probes.is_empty()
            || self.git.operation.is_some()
            || self.git.generation.is_some()
            || !self.session_control.namings.is_empty()
        {
            self.closing = true;
            if let Some(operation) = &self.git.operation {
                operation.probe.cancel();
            }
            if let Some(generation) = &self.git.generation {
                generation.runtime.cancel();
            }
            for naming in self.session_control.namings.values_mut() {
                naming.runtime.cancel();
            }
            for run in self.session_control.runs.values_mut() {
                run.cancel();
            }
            for probe in self.probes.values() {
                probe.cancel();
            }
            for probe in self.opencode_model_probes.values() {
                probe.cancel();
            }
            cx.notify();
            false
        } else {
            cx.notify();
            false
        }
    }

    fn persist(&mut self) -> bool {
        if !self.writable {
            return false;
        }
        self.last_save = Instant::now();
        self.session_control.global_revision = self.session_control.global_revision.wrapping_add(1);
        self.store.queued_messages = self.queued_messages.clone();
        if !self.persistence.submit(&self.store) {
            self.dirty = true;
            self.message = self
                .persistence
                .error
                .as_ref()
                .map(|e| conduit::i18n::format("app.save_failed", &[("e", e.to_string())]));
            return false;
        }
        self.dirty = false;
        true
    }
    fn persist_confirmed(&mut self) -> Result<crate::persistence::SaveReceipt, String> {
        if !self.writable {
            return Err(conduit::i18n::text_static("app.unable_to_save_messages_right_now").into());
        }
        self.last_save = Instant::now();
        self.session_control.global_revision = self.session_control.global_revision.wrapping_add(1);
        self.store.queued_messages = self.queued_messages.clone();
        let receipt = self.persistence.submit_confirmed(&self.store)?;
        self.dirty = false;
        Ok(receipt)
    }
    fn poll_persistence(&mut self, cx: &mut Context<Self>) {
        if self.persistence.poll() {
            self.trim_transcripts();
            if let Some(error) = &self.persistence.error {
                self.dirty = true;
                self.closing = false;
                // Pending sends report their own failure against the originating
                // conversation, preserving another conversation's banner.
                if self
                    .session_control
                    .runs
                    .values()
                    .all(|run| run.pending_save.is_none())
                    && self.session_control.queue_saves.is_empty()
                {
                    self.message = Some(conduit::i18n::format(
                        "notifications.save_failed",
                        &[("error", error.to_string())],
                    ));
                }
            }
            cx.notify();
        }
        if self.attachments_cleanup_pending
            && !self.persistence.busy()
            && self.persistence.error.is_none()
            && !self.dirty
            && self
                .session_control
                .runs
                .values()
                .all(|run| run.pending_save.is_none())
            && self.session_control.queue_saves.is_empty()
        {
            self.attachments_cleanup_pending = false;
            self.cleanup_attachments();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests;
