//! Application state and initialization. Views and workflows live in app/.

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
        AppearanceMode, MAX_CHAT_FONT_SIZE, MAX_CHAT_LINE_HEIGHT, MAX_CODE_FONT_SIZE,
        MAX_TAB_LIMIT, MIN_CHAT_FONT_SIZE, MIN_CHAT_LINE_HEIGHT, MIN_CODE_FONT_SIZE, NamingMode,
        Store,
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
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
        popover::Popover,
        scroll::ScrollableElement,
        searchable_list::{SearchableListItem, SearchableVec},
        select::{Select, SelectEvent, SelectState},
        slider::{Slider, SliderEvent, SliderState},
        spinner::Spinner,
        switch::Switch,
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

mod chat;
mod components;
mod composer;
mod dock;
mod events;
mod git_branches;
mod git_compare;
mod git_diff;
mod git_ui;
mod mentions;
mod model_picker;
mod naming;
mod opencode_models;
mod outline;
mod performance_view;
mod runtime;
mod search;
mod sessions;
mod settings;
mod shell;
mod sidebar;
mod state;
mod theme;
mod toolbar;
mod tools;
mod typography;

use components::*;
use composer::*;
use events::*;
use git_diff::*;
use outline::*;
#[cfg(test)]
use search::{highlight_ranges, search_snippet};
use sessions::*;
use settings::*;
use state::*;
pub(crate) use theme::set_palette;
use theme::*;
use tools::*;
use typography::*;

actions!(
    conduit,
    [
        SendPrompt,
        Newline,
        NextDiff,
        PreviousDiff,
        SubmitBranchForm,
        CancelBranchForm,
        PreviousFileMention,
        NextFileMention,
        AcceptFileMention,
        DismissFileMention
    ]
);

pub struct ConduitApp {
    store: Store,
    writable: bool,
    selected_workspace: Option<usize>,
    selected_session: Option<u64>,
    /// Open session tabs, ordered left to right by when they were opened.
    /// Selecting a tab keeps its position; only new tabs append at the end.
    open_tabs: Vec<u64>,
    input: Entity<TextareaState>,
    drafts: HashMap<u64, String>,
    workspace_drafts: HashMap<PathBuf, String>,
    composer_files: HashMap<ComposerTarget, ComposerFiles>,
    file_mentions: mentions::FileMentions,
    runs: HashMap<u64, ActiveRun>,
    /// Successful background replies waiting for the user to open the session.
    unread_completed: HashSet<u64>,
    probes: HashMap<Provider, VersionProbe>,
    diagnostics: HashMap<Provider, String>,
    catalogs: HashMap<Provider, ModelCatalog>,
    opencode_catalogs: HashMap<PathBuf, ModelCatalog>,
    opencode_model_probes: HashMap<PathBuf, models::OpenCodeModelsProbe>,
    ccs: CcsCatalog,
    message: Option<String>,
    settings_tab: SettingsTab,
    git_prompt_input: Entity<TextareaState>,
    /// Font pickers for chat body text and code; rebuilt from the installed
    /// families when the app starts.
    chat_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    code_font_select: Entity<SelectState<SearchableVec<FontChoice>>>,
    /// The theme's platform monospace family, restored when no code font is set.
    default_mono_font: SharedString,
    next_id: u64,
    closing: bool,
    details: bool,
    detail_view: DetailView,
    detail_text: Option<String>,
    /// Width of the right sidebar.
    detail_width: f32,
    /// Drag origin `(pointer x, width at drag start)` for the right sidebar edge.
    detail_drag: Option<(f32, f32)>,
    /// Measured width of the left workspace sidebar, used to cap the right one at
    /// half of the remaining content area.
    sidebar_width: f32,
    /// Read-only Git snapshot for the selected workspace and its in-flight probe.
    git_probe: Option<GitProbe>,
    git_snapshot: Option<git::Snapshot>,
    git_error: Option<String>,
    git_workspace: Option<PathBuf>,
    branch_probe: Option<BranchProbe>,
    git_repository: Option<git::Repository>,
    last_branch_check: Instant,
    commit_drafts: HashMap<PathBuf, CommitDraft>,
    commit_generation: Option<CommitGeneration>,
    /// In-flight session naming requests, keyed by session id.
    session_namings: HashMap<u64, SessionNaming>,
    /// Last automatic naming failure per session; cleared on the next attempt
    /// and shown in the conversation toolbar until then.
    naming_errors: HashMap<u64, String>,
    git_operation: Option<GitOperation>,
    git_feedback: HashMap<PathBuf, GitFeedback>,
    /// Git sidebar groups the user collapsed; empty means all expanded.
    collapsed_git_groups: HashSet<git::ChangeKind>,
    commit_and_push: bool,
    /// Workspaces whose session list was expanded past the configured limit.
    expanded_workspaces: HashSet<usize>,
    /// Workspace paths whose CLI history is being imported in the background.
    history_loading: HashSet<PathBuf>,
    /// Sidebar session search filter.
    search: Entity<InputState>,
    search_query: String,
    show_archived: bool,
    /// Engine used by the sidebar + button and the search dialog's empty state.
    new_session_provider: Provider,
    pending_model_selection: Option<ComposerSelection>,
    rename_target: Option<u64>,
    rename_input: Option<Entity<InputState>>,
    delete_target: Option<u64>,
    remove_workspace_target: Option<usize>,
    _subscriptions: Vec<Subscription>,
    dirty: bool,
    last_save: Instant,
    scroll: gpui_kit::ListState,
    /// Focus target for the conversation, so a finished selection drag leaves the
    /// composer and Ctrl+C reaches the window root's copy handler.
    chat_focus: FocusHandle,
    timeline_session: Option<u64>,
    outline: MessageOutlineCache,
    /// Whether the message list is scrolled to (or near) its end. Drives the
    /// floating "jump to latest" arrow so it only shows once the user scrolls up.
    at_bottom: bool,
    /// Scoped by session and tool key, independent of the right detail dock.
    expanded_tools: HashSet<(u64, String)>,
    /// Groups manually pinned open after the assistant starts replying.
    expanded_tool_groups: HashSet<(u64, String)>,
    expanded_reasoning: HashSet<(u64, String)>,
    expanded_errors: HashSet<(u64, String)>,
    /// Live plan reported by the current turn, keyed by session.
    plans: HashMap<u64, Vec<PlanStep>>,
    /// Last elapsed-label repaint: every second during turns, every minute idle.
    last_live_tick: Instant,
    tool_scrolls: HashMap<(u64, String), ScrollHandle>,
    message_focus: Option<(u64, String, Instant)>,
    pending_message_jump: bool,
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
        cx.bind_keys([
            KeyBinding::new("enter", SubmitBranchForm, Some("GitBranchForm > Input")),
            KeyBinding::new("enter", SubmitBranchForm, Some("GitBranchForm")),
            KeyBinding::new("escape", CancelBranchForm, Some("GitBranchForm > Input")),
            KeyBinding::new("escape", CancelBranchForm, Some("GitBranchForm")),
            // Enter sends from the composer; Ctrl/Shift+Enter insert a newline.
            // Bind at the input depth (wins over the textarea's own Enter
            // handling) and at composer depth for when focus is nearby.
            KeyBinding::new("enter", SendPrompt, Some("ConduitComposer > Input")),
            KeyBinding::new("enter", SendPrompt, Some("ConduitComposer")),
            KeyBinding::new("ctrl-enter", Newline, Some("ConduitComposer > Input")),
            KeyBinding::new("ctrl-enter", Newline, Some("ConduitComposer")),
            KeyBinding::new("shift-enter", Newline, Some("ConduitComposer > Input")),
            KeyBinding::new("shift-enter", Newline, Some("ConduitComposer")),
            KeyBinding::new("up", PreviousFileMention, Some("FileMention > Input")),
            KeyBinding::new("down", NextFileMention, Some("FileMention > Input")),
            KeyBinding::new("tab", AcceptFileMention, Some("FileMention > Input")),
            KeyBinding::new("escape", DismissFileMention, Some("FileMention > Input")),
            // JetBrains' diff navigation shortcuts, scoped to the diff window.
            KeyBinding::new("f7", NextDiff, Some("GitDiffDialog")),
            KeyBinding::new("shift-f7", PreviousDiff, Some("GitDiffDialog")),
            KeyBinding::new("f7", NextDiff, Some("Diff")),
            KeyBinding::new("shift-f7", PreviousDiff, Some("Diff")),
        ]);
        let (mut store, message, writable) = match loaded {
            Ok(store) => (store, None, true),
            Err(error) => (Store::default(), Some(error), false),
        };
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
        let chat_font_index = font_index(&store.ui.chat_font_family);
        let code_font_index = font_index(&store.ui.code_font_family);
        let chat_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(font_choices("默认（跟随主题）")),
                Some(IndexPath::new(chat_font_index)),
                window,
                cx,
            )
            .searchable(true)
        });
        let code_font_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(font_choices("默认（系统等宽）")),
                Some(IndexPath::new(code_font_index)),
                window,
                cx,
            )
            .searchable(true)
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
                .sort_by_key(|session| std::cmp::Reverse(session.created_at));
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
                .placeholder("描述任务，输入 @ 引用文件…")
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
            this.refresh_file_mentions(cx);
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索会话"));
        let search_subscription = cx.subscribe(&search, |this, search, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = search.read(cx).value().to_string();
                cx.notify();
            }
        });
        let git_prompt_input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder("填写 Commit Message 生成提示词…")
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
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                this.git_probe = None;
                this.branch_probe = None;
                if !this.runs.is_empty()
                    || !this.probes.is_empty()
                    || !this.opencode_model_probes.is_empty()
                    || this.git_operation.is_some()
                    || this.commit_generation.is_some()
                    || !this.session_namings.is_empty()
                {
                    this.closing = true;
                    if let Some(operation) = &this.git_operation {
                        operation.probe.cancel();
                    }
                    if let Some(generation) = &this.commit_generation {
                        generation.runtime.cancel();
                    }
                    for naming in this.session_namings.values_mut() {
                        naming.runtime.cancel();
                    }
                    for run in this.runs.values_mut() {
                        run.cancel();
                    }
                    for probe in this.probes.values() {
                        probe.cancel();
                    }
                    for probe in this.opencode_model_probes.values() {
                        probe.cancel();
                    }
                    cx.notify();
                    false
                } else {
                    this.persist();
                    true
                }
            })
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
        Self {
            store,
            writable,
            selected_workspace,
            selected_session,
            open_tabs: selected_session.into_iter().collect(),
            input,
            drafts: HashMap::new(),
            workspace_drafts: HashMap::new(),
            composer_files: HashMap::new(),
            file_mentions: mentions::FileMentions::default(),
            runs: HashMap::new(),
            unread_completed: HashSet::new(),
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
            settings_tab: SettingsTab::General,
            git_prompt_input,
            chat_font_select,
            code_font_select,
            default_mono_font,
            next_id,
            closing: false,
            details: start_in_git,
            detail_view: if start_in_git {
                DetailView::Git
            } else {
                DetailView::Tool
            },
            detail_text: None,
            detail_width: 340.,
            detail_drag: None,
            sidebar_width: 248.,
            git_probe: None,
            git_snapshot: None,
            git_error: None,
            git_workspace: None,
            branch_probe: None,
            git_repository: None,
            last_branch_check: Instant::now(),
            commit_drafts: HashMap::new(),
            commit_generation: None,
            session_namings: HashMap::new(),
            naming_errors: HashMap::new(),
            git_operation: None,
            git_feedback: HashMap::new(),
            collapsed_git_groups: HashSet::new(),
            commit_and_push: false,
            expanded_workspaces: HashSet::new(),
            history_loading: HashSet::new(),
            search,
            search_query: String::new(),
            show_archived: false,
            new_session_provider,
            pending_model_selection: None,
            rename_target: None,
            rename_input: None,
            delete_target: None,
            remove_workspace_target: None,
            _subscriptions: vec![
                input_subscription,
                mention_subscription,
                search_subscription,
                git_prompt_subscription,
                appearance_subscription,
                chat_font_subscription,
                code_font_subscription,
            ],
            dirty: false,
            last_save: Instant::now(),
            scroll: gpui_kit::ListState::new(0, ListAlignment::Bottom, px(240.)),
            chat_focus: cx.focus_handle(),
            timeline_session: None,
            outline: MessageOutlineCache::default(),
            at_bottom: true,
            expanded_tools: HashSet::new(),
            expanded_tool_groups: HashSet::new(),
            expanded_reasoning: HashSet::new(),
            expanded_errors: HashSet::new(),
            plans: HashMap::new(),
            last_live_tick: Instant::now(),
            tool_scrolls: HashMap::new(),
            message_focus: None,
            pending_message_jump: false,
            performance: cx.new(|_| PerformanceMonitor::new()),
            performance_open: false,
            copied_action: None,
            copied_action_seq: 0,
        }
    }

    fn persist(&mut self) -> bool {
        if !self.writable {
            return false;
        }
        self.last_save = Instant::now();
        match self.store.save() {
            Ok(()) => {
                self.dirty = false;
                self.last_save = Instant::now();
                true
            }
            Err(error) => {
                self.message = Some(format!("保存失败：{error:#}"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests;
