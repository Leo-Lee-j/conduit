use crate::{
    approval::ApprovalQueue,
    ccs::{self, CcsCatalog},
    chat_runtime::ChatRuntime,
    cli::{self, VersionProbe},
    domain::{ChatItem, ItemKind, Provider, SessionRecord, Workspace},
    git::{self, BranchProbe, GitProbe},
    models::{self, ModelCatalog},
    protocol::Event,
    store::Store,
};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        popover::Popover,
        text::TextView,
        *,
    },
    prelude::FluentBuilder,
    *,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct ActiveRun {
    runtime: ChatRuntime,
    approvals: ApprovalQueue,
    prefix: String,
    status: String,
}

struct CommitDraft {
    input: Entity<TextareaState>,
    _subscription: Subscription,
}

struct CommitGeneration {
    path: PathBuf,
    runtime: ChatRuntime,
    started: Instant,
    original: String,
    parts: Vec<(String, String)>,
    error: Option<String>,
}

struct GitOperation {
    probe: git::OperationProbe,
    draft: String,
}

struct GitFeedback {
    text: String,
    error: bool,
}

fn update_commit_text(
    parts: &mut Vec<(String, String)>,
    key: String,
    text: String,
    snapshot: bool,
) {
    let index = parts
        .iter()
        .position(|(id, _)| id == &key)
        .unwrap_or_else(|| {
            parts.push((key, String::new()));
            parts.len() - 1
        });
    if snapshot {
        parts[index].1 = text;
    } else {
        parts[index].1.push_str(&text);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Git,
    Cli,
    Provider(Provider),
    Models,
}

/// Independent tool windows sharing the application's right-hand dock.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailView {
    Tool,
    Git,
}

/// Drag payload (and invisible drag preview) for the right-sidebar edge.
#[derive(Clone)]
struct DetailResize;

impl Render for DetailResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

actions!(conduit, [SendPrompt, Newline]);

const CANVAS: u32 = 0x0e0e10;
const SIDEBAR_BG: u32 = 0x131315;
const SURFACE: u32 = 0x19191c;
const SURFACE_HOVER: u32 = 0x232327;
const SURFACE_STRONG: u32 = 0x202024;
const BORDER: u32 = 0x29292e;
const BORDER_STRONG: u32 = 0x3b3b42;
const TEXT: u32 = 0xf3f3f4;
const MUTED: u32 = 0xa1a1a8;
const FAINT: u32 = 0x717179;
const FOCUS: u32 = 0x4b4b54;

fn provider_icon(provider: Provider) -> Icon {
    let (data, color): (&[u8], u32) = match provider {
        Provider::Claude => (include_bytes!("../assets/icons/claude.svg"), 0xd97757),
        Provider::Codex => (include_bytes!("../assets/icons/openai.svg"), TEXT),
    };
    Icon::default()
        .data(data)
        .size(px(14.))
        .text_color(rgb(color))
}

fn display_path(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
    }
}

fn settings_label(text: &'static str) -> impl IntoElement {
    div()
        .w(px(84.))
        .flex_shrink_0()
        .text_size(px(11.))
        .text_color(rgb(FAINT))
        .child(text)
}

/// A read-only settings card row: fixed label on the left, value on the right.
fn settings_row(label: &'static str, value: String, divider: bool) -> AnyElement {
    let mut row = h_flex().px_4().py_3().gap_4().items_center();
    if divider {
        row = row.border_t_1().border_color(rgb(BORDER));
    }
    row.child(settings_label(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child(value),
        )
        .into_any_element()
}

/// Whether a session matches the sidebar search query (title or message text).
fn session_matches(session: &SessionRecord, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    session.title.to_lowercase().contains(query)
        || session
            .messages
            .iter()
            .any(|message| message.text.to_lowercase().contains(query))
}

struct SearchHit {
    workspace: usize,
    session: u64,
    provider: Provider,
    workspace_name: String,
    title: String,
    archived: bool,
    snippet: String,
}

/// Byte ranges of `query` occurrences in `text`, matched case-insensitively
/// while keeping ranges aligned to the original text's char boundaries.
fn highlight_ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
    let query = query.to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    // One shadow byte maps back to the original char's byte start. Lowercasing
    // can change a char's UTF-8 length (e.g. 'İ'), so the map must be per byte.
    let mut shadow = String::new();
    let mut map: Vec<usize> = Vec::new();
    for (index, ch) in text.char_indices() {
        let lowered = ch.to_lowercase().collect::<String>();
        for _ in lowered.bytes() {
            map.push(index);
        }
        shadow.push_str(&lowered);
    }
    map.push(text.len());
    let mut ranges = Vec::new();
    let mut start = 0;
    while let Some(position) = shadow[start..].find(&query) {
        let begin = start + position;
        let end = begin + query.len();
        let (from, to) = (map[begin], map[end]);
        if from < to {
            ranges.push((from, to));
        }
        start = end;
    }
    ranges
}

fn search_snippet(text: &str, query: &str) -> String {
    let ranges = highlight_ranges(text, query);
    let fallback = |text: &str| -> String {
        let mut snippet: String = text.chars().take(160).collect();
        if text.chars().count() > 160 {
            snippet.push('…');
        }
        snippet.replace(['\r', '\n'], " ")
    };
    let Some((match_start, match_end)) = ranges.first().copied() else {
        return fallback(text);
    };
    let char_position = text[..match_start].chars().count();
    let match_chars = text[match_start..match_end].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let start = char_position.saturating_sub(40);
    let end = (char_position + match_chars + 120).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect();
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet.replace(['\r', '\n'], " ")
}

fn highlighted_text(text: &str, query: &str, size: f32, color: u32) -> AnyElement {
    let ranges = highlight_ranges(text, query);
    let mut styled = StyledText::new(text.to_string());
    if !ranges.is_empty() {
        styled = styled.with_highlights(ranges.into_iter().map(|(start, end)| {
            (
                start..end,
                HighlightStyle {
                    color: Some(Hsla::from(rgb(0xf0d49b))),
                    background_color: Some(Hsla::from(rgb(0x3a3220))),
                    ..Default::default()
                },
            )
        }));
    }
    div()
        .text_size(px(size))
        .text_color(rgb(color))
        .child(styled)
        .into_any_element()
}

/// Render a Git patch as one highlighted monospace text block, so large diffs
/// stay a single text layout instead of thousands of per-line elements.
fn diff_element(diff: &str) -> AnyElement {
    let mut styled = StyledText::new(diff.to_string());
    let highlights: Vec<_> = git::diff_highlights(diff)
        .into_iter()
        .map(|(range, kind)| {
            let (color, background) = match kind {
                git::DiffLine::Added => (0x9fdcab, Some(0x18261c)),
                git::DiffLine::Removed => (0xe6a1a1, Some(0x2a1a1a)),
                git::DiffLine::Hunk => (0x8fb8e6, None),
                git::DiffLine::Header => (FAINT, None),
            };
            (
                range,
                HighlightStyle {
                    color: Some(Hsla::from(rgb(color))),
                    background_color: background.map(|background| Hsla::from(rgb(background))),
                    ..Default::default()
                },
            )
        })
        .collect();
    if !highlights.is_empty() {
        styled = styled.with_highlights(highlights);
    }
    div()
        .w_full()
        .p_2()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(0x141417))
        .font_family("Consolas")
        .text_size(px(10.))
        .line_height(relative(1.35))
        .text_color(rgb(MUTED))
        .child(styled)
        .into_any_element()
}

fn session_menu_button(id: (&'static str, u64), label: &'static str, icon: IconName) -> Button {
    Button::new(id)
        .ghost()
        .h(px(30.))
        .w_full()
        .px_2()
        .rounded(px(6.))
        .cursor_pointer()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).small().text_color(rgb(FAINT)))
                .child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(label)),
        )
}

pub struct ConduitApp {
    store: Store,
    writable: bool,
    selected_workspace: Option<usize>,
    selected_session: Option<u64>,
    input: Entity<TextareaState>,
    drafts: HashMap<u64, String>,
    runs: HashMap<u64, ActiveRun>,
    probes: HashMap<Provider, VersionProbe>,
    diagnostics: HashMap<Provider, String>,
    catalogs: HashMap<Provider, ModelCatalog>,
    ccs: CcsCatalog,
    message: Option<String>,
    settings_tab: SettingsTab,
    git_prompt_input: Entity<TextareaState>,
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
    git_operation: Option<GitOperation>,
    git_feedback: HashMap<PathBuf, GitFeedback>,
    commit_and_push: bool,
    /// Approval tickets whose raw request detail the user expanded.
    approval_details: HashSet<String>,
    /// Workspaces whose session list was expanded past the configured limit.
    expanded_workspaces: HashSet<usize>,
    /// Sidebar session search filter.
    search: Entity<InputState>,
    search_query: String,
    show_archived: bool,
    /// Engine used by the sidebar + button and the search dialog's empty state.
    new_session_provider: Provider,
    rename_target: Option<u64>,
    rename_input: Option<Entity<InputState>>,
    delete_target: Option<u64>,
    remove_workspace_target: Option<usize>,
    _subscriptions: Vec<Subscription>,
    dirty: bool,
    last_save: Instant,
    scroll: ScrollHandle,
    /// Whether the message list is scrolled to (or near) its end. Drives the
    /// floating "jump to latest" pin so it only shows once the user scrolls up.
    at_bottom: bool,
}

impl ConduitApp {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        start_in_settings: bool,
        start_in_git: bool,
    ) -> Self {
        cx.bind_keys([
            // Enter sends from the composer; Ctrl/Shift+Enter insert a newline.
            // Bind at the input depth (wins over the textarea's own Enter
            // handling) and at composer depth for when focus is nearby.
            KeyBinding::new("enter", SendPrompt, Some("ConduitComposer > Input")),
            KeyBinding::new("enter", SendPrompt, Some("ConduitComposer")),
            KeyBinding::new("ctrl-enter", Newline, Some("ConduitComposer > Input")),
            KeyBinding::new("ctrl-enter", Newline, Some("ConduitComposer")),
            KeyBinding::new("shift-enter", Newline, Some("ConduitComposer > Input")),
            KeyBinding::new("shift-enter", Newline, Some("ConduitComposer")),
        ]);
        let (store, message, writable) = match Store::load() {
            Ok(store) => (store, None, true),
            Err(error) => (Store::default(), Some(format!("{error:#}")), false),
        };
        let next_id = store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .map(|s| s.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let selected_workspace = (!store.workspaces.is_empty()).then_some(0);
        let selected_session = store
            .workspaces
            .first()
            .and_then(|w| w.sessions.iter().rev().find(|session| !session.archived))
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
                .placeholder("描述任务，或继续提问…")
                .auto_grow(3, 8)
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
                    || this.git_operation.is_some()
                    || this.commit_generation.is_some()
                {
                    this.closing = true;
                    if let Some(operation) = &this.git_operation {
                        operation.probe.cancel();
                    }
                    if let Some(generation) = &this.commit_generation {
                        generation.runtime.cancel();
                    }
                    for run in this.runs.values_mut() {
                        run.runtime.cancel();
                        run.approvals.clear();
                        run.status = "正在停止…".into();
                    }
                    for probe in this.probes.values() {
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
        Self {
            store,
            writable,
            selected_workspace,
            selected_session,
            input,
            drafts: HashMap::new(),
            runs: HashMap::new(),
            probes: HashMap::new(),
            diagnostics: HashMap::new(),
            catalogs: [Provider::Claude, Provider::Codex]
                .into_iter()
                .map(|provider| (provider, models::load(provider)))
                .collect(),
            ccs: ccs::load(),
            message,
            settings_tab: SettingsTab::General,
            git_prompt_input,
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
            git_operation: None,
            git_feedback: HashMap::new(),
            commit_and_push: false,
            approval_details: HashSet::new(),
            expanded_workspaces: HashSet::new(),
            search,
            search_query: String::new(),
            show_archived: false,
            new_session_provider,
            rename_target: None,
            rename_input: None,
            delete_target: None,
            remove_workspace_target: None,
            _subscriptions: vec![search_subscription, git_prompt_subscription],
            dirty: false,
            last_save: Instant::now(),
            scroll: ScrollHandle::new(),
            at_bottom: true,
        }
    }
    fn session(&self, id: u64) -> Option<&SessionRecord> {
        self.store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .find(|s| s.id == id)
    }
    fn session_mut(&mut self, id: u64) -> Option<&mut SessionRecord> {
        self.store
            .workspaces
            .iter_mut()
            .flat_map(|w| &mut w.sessions)
            .find(|s| s.id == id)
    }
    fn select_fallback_session(&mut self) {
        if self
            .selected_session
            .is_some_and(|id| self.session(id).is_some_and(|session| !session.archived))
        {
            return;
        }
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            if let Some(session) = workspace.sessions.iter().rev().find(|s| !s.archived) {
                self.selected_workspace = Some(index);
                self.selected_session = Some(session.id);
                return;
            }
        }
        self.selected_session = None;
        if self.store.workspaces.is_empty() {
            self.selected_workspace = None;
        } else if self
            .selected_workspace
            .is_none_or(|index| index >= self.store.workspaces.len())
        {
            self.selected_workspace = Some(0);
        }
    }
    fn toggle_archive(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(id) {
            session.archived = !session.archived;
        }
        self.select_fallback_session();
        self.persist();
        self.sync_git_workspace(cx);
        cx.notify();
    }
    fn open_rename_dialog(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(current) = self.session(id).map(|session| session.title.clone()) else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.rename_target = Some(id);
        self.rename_input = Some(input);
        cx.notify();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .w(px(420.))
                .margin_top(px(120.))
                .title("重命名会话")
                .content(move |content, _, cx| {
                    let Some(input) = view.read(cx).rename_input.clone() else {
                        return content;
                    };
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex().gap_3().child(Input::new(&input).h(px(32.))).child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("rename-cancel")
                                        .label("取消")
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .on_click(move |_, window, cx| {
                                            cancel.update(cx, |this, cx| {
                                                this.rename_target = None;
                                                this.rename_input = None;
                                                cx.notify();
                                            });
                                            window.close_dialog(cx);
                                        }),
                                )
                                .child(
                                    Button::new("rename-confirm")
                                        .label("保存")
                                        .small()
                                        .primary()
                                        .cursor_pointer()
                                        .on_click(move |_, window, cx| {
                                            let value = confirm
                                                .read(cx)
                                                .rename_input
                                                .as_ref()
                                                .map(|input| input.read(cx).value().to_string())
                                                .unwrap_or_default();
                                            confirm.update(cx, |this, cx| {
                                                this.apply_rename(value, cx)
                                            });
                                            window.close_dialog(cx);
                                        }),
                                ),
                        ),
                    )
                })
        });
    }
    fn apply_rename(&mut self, title: String, cx: &mut Context<Self>) {
        let Some(id) = self.rename_target.take() else {
            return;
        };
        self.rename_input = None;
        let title = title.trim().to_string();
        if title.is_empty() {
            self.message = Some("会话名不能为空。".into());
            cx.notify();
            return;
        }
        if let Some(session) = self.session_mut(id) {
            session.title = title.chars().take(80).collect();
        }
        self.persist();
        cx.notify();
    }
    fn open_delete_dialog(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.session(id).map(|session| {
            if session.title.is_empty() {
                "未命名会话".to_string()
            } else {
                session.title.clone()
            }
        }) else {
            return;
        };
        self.delete_target = Some(id);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let title = title.clone();
            dialog
                .w(px(420.))
                .margin_top(px(120.))
                .title("删除会话")
                .content(move |content, _, _| {
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(relative(1.5))
                                    .text_color(rgb(MUTED))
                                    .child(format!(
                                        "将永久删除「{title}」及其消息记录，无法恢复。"
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("delete-cancel")
                                            .label("取消")
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                cancel.update(cx, |this, cx| {
                                                    this.delete_target = None;
                                                    cx.notify();
                                                });
                                                window.close_dialog(cx);
                                            }),
                                    )
                                    .child(
                                        Button::new("delete-confirm")
                                            .label("删除")
                                            .small()
                                            .primary()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                let target = confirm.read(cx).delete_target;
                                                if let Some(id) = target {
                                                    confirm.update(cx, |this, cx| {
                                                        this.delete_session(id, cx)
                                                    });
                                                }
                                                window.close_dialog(cx);
                                            }),
                                    ),
                            ),
                    )
                })
        });
    }
    fn delete_session(&mut self, id: u64, cx: &mut Context<Self>) {
        for workspace in &mut self.store.workspaces {
            workspace.sessions.retain(|session| session.id != id);
        }
        self.drafts.remove(&id);
        self.detail_text = None;
        self.select_fallback_session();
        self.delete_target = None;
        self.persist();
        self.sync_git_workspace(cx);
        cx.notify();
    }
    fn new_session_here(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let provider = self.new_session_provider;
        self.new_session(index, provider, window, cx);
    }
    fn select_engine(&mut self, provider: Provider, window: &mut Window, cx: &mut Context<Self>) {
        self.new_session_provider = provider;
        self.remember_provider(provider);
        if let Some(index) = self.selected_workspace
            && index < self.store.workspaces.len()
        {
            let target = self.store.workspaces[index]
                .sessions
                .iter()
                .rev()
                .find(|session| !session.archived && session.provider == provider)
                .map(|session| session.id);
            match target {
                Some(id) => self.switch(index, Some(id), window, cx),
                None => {
                    // No session of that engine yet: clear the selection so the
                    // next prompt starts one with the chosen engine. The draft
                    // stays in the composer.
                    if let Some(current) = self.selected_session {
                        self.drafts
                            .insert(current, self.input.read(cx).value().to_string());
                    }
                    self.selected_session = None;
                    self.detail_text = None;
                    self.message = None;
                    cx.notify();
                }
            }
        } else {
            cx.notify();
        }
    }
    fn engine_picker(&self, view: Entity<Self>) -> AnyElement {
        let provider = self.new_session_provider;
        let trigger = Button::new("engine-picker-trigger")
            .ghost()
            .small()
            .h(px(22.))
            .px_1()
            .cursor_pointer()
            .tooltip("切换引擎")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(provider.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(FAINT)),
                    ),
            );
        Popover::new("engine-picker")
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(180.)).gap(px(1.));
                for option in [Provider::Claude, Provider::Codex] {
                    let app = view.clone();
                    let popover = popover.clone();
                    let active = option == provider;
                    list = list.child(
                        Button::new(if option == Provider::Claude {
                            "engine-option-claude"
                        } else {
                            "engine-option-codex"
                        })
                        .ghost()
                        .h(px(30.))
                        .w_full()
                        .px_2()
                        .cursor_pointer()
                        .accessibility_label(option.label())
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .items_center()
                                .child(provider_icon(option))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(MUTED))
                                        .child(option.label()),
                                ),
                        )
                        .when(active, |button| button.bg(rgb(SURFACE_HOVER)))
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| this.select_engine(option, window, cx));
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list
            })
            .into_any_element()
    }
    /// Top-left workspace switcher: pick an existing workspace or add a folder.
    fn workspace_picker(&self, view: Entity<Self>) -> AnyElement {
        let current = self.selected_workspace;
        let label = current
            .and_then(|index| self.store.workspaces.get(index))
            .map(|workspace| workspace.name())
            .unwrap_or_else(|| "选择工作区".into());
        let rows: Vec<(usize, String, String, Option<u64>, bool)> = self
            .store
            .workspaces
            .iter()
            .enumerate()
            .map(|(index, workspace)| {
                let session = workspace
                    .sessions
                    .iter()
                    .rev()
                    .find(|session| !session.archived)
                    .map(|session| session.id);
                (
                    index,
                    workspace.name(),
                    display_path(&workspace.path),
                    session,
                    current == Some(index),
                )
            })
            .collect();
        let trigger = Button::new("workspace-picker-trigger")
            .ghost()
            .small()
            .h(px(26.))
            .px_1()
            .cursor_pointer()
            .tooltip("切换工作区")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(Icon::new(IconName::Folder).small().text_color(rgb(FAINT)))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(if current.is_some() { TEXT } else { MUTED }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(FAINT)),
                    ),
            );
        let content_view = view.clone();
        Popover::new("workspace-picker")
            .anchor(Anchor::TopLeft)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _window, cx| {
                let app = content_view.clone();
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(320.)).gap(px(1.));
                for (index, name, path, session, active) in rows.clone() {
                    let select_app = app.clone();
                    let select_popover = popover.clone();
                    list = list.child(
                        Button::new(("workspace-pick", index as u64))
                            .ghost()
                            .h(px(36.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(SURFACE_HOVER)))
                            .on_click(move |_, window, cx| {
                                select_app
                                    .update(cx, |this, cx| this.switch(index, session, window, cx));
                                if let Some(popover) = select_popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Icon::new(IconName::Folder).small().text_color(rgb(FAINT)),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(TEXT))
                                                    .child(name),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(FAINT))
                                                    .child(path),
                                            ),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(MUTED)),
                                        )
                                    }),
                            ),
                    );
                }
                if !rows.is_empty() {
                    list = list.child(div().h(px(1.)).my(px(3.)).bg(rgb(BORDER)));
                }
                let add_app = app.clone();
                let add_popover = popover.clone();
                list = list.child(
                    Button::new("workspace-add-directory")
                        .ghost()
                        .h(px(32.))
                        .w_full()
                        .px_2()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .on_click(move |event, window, cx| {
                            add_app.update(cx, |this, cx| this.add_workspace(event, window, cx));
                            if let Some(popover) = add_popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .items_center()
                                .child(Icon::new(IconName::Plus).small().text_color(rgb(FAINT)))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(MUTED))
                                        .child("添加目录…"),
                                ),
                        ),
                );
                list
            })
            .into_any_element()
    }
    fn open_remove_workspace_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .store
            .workspaces
            .get(index)
            .map(|workspace| workspace.name())
        else {
            return;
        };
        self.remove_workspace_target = Some(index);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let name = name.clone();
            dialog
                .w(px(440.))
                .margin_top(px(120.))
                .title("移除工作区")
                .content(move |content, _, _| {
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(relative(1.5))
                                    .text_color(rgb(MUTED))
                                    .child(format!(
                                        "从 Conduit 移除「{name}」？磁盘上的文件与 CLI 自带历史不会被删除；应用内该工作区的会话记录将不再显示。"
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("remove-workspace-cancel")
                                            .label("取消")
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                cancel.update(cx, |this, cx| {
                                                    this.remove_workspace_target = None;
                                                    cx.notify();
                                                });
                                                window.close_dialog(cx);
                                            }),
                                    )
                                    .child(
                                        Button::new("remove-workspace-confirm")
                                            .label("移除")
                                            .small()
                                            .primary()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                let target =
                                                    confirm.read(cx).remove_workspace_target;
                                                if let Some(index) = target {
                                                    confirm.update(cx, |this, cx| {
                                                        this.remove_workspace(index, cx)
                                                    });
                                                }
                                                window.close_dialog(cx);
                                            }),
                                    ),
                            ),
                    )
                })
        });
    }
    fn remove_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        self.remove_workspace_target = None;
        if index >= self.store.workspaces.len() {
            return;
        }
        if self.workspace_busy(index) {
            self.message = Some("该工作区仍有任务运行，请先取消后再移除。".into());
            cx.notify();
            return;
        }
        let removed = self.store.workspaces.remove(index);
        self.expanded_workspaces.clear();
        for session in &removed.sessions {
            self.drafts.remove(&session.id);
        }
        if self.selected_workspace == Some(index) || self.selected_session.is_none() {
            self.selected_workspace = None;
            self.selected_session = None;
            self.select_fallback_session();
        } else if let Some(selected) = self.selected_workspace
            && selected > index
        {
            self.selected_workspace = Some(selected - 1);
        }
        self.detail_text = None;
        self.persist();
        self.sync_git_workspace(cx);
        cx.notify();
    }
    fn workspace_menu(&self, index: usize, app: Entity<Self>) -> AnyElement {
        let trigger = Button::new(("workspace-menu-trigger", index))
            .ghost()
            .small()
            .compact()
            .icon(IconName::Ellipsis)
            .cursor_pointer()
            .on_click(|_, _, cx| cx.stop_propagation());
        Popover::new(("workspace-menu", index))
            .anchor(Anchor::LeftCenter)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let remove_app = app.clone();
                let remove_popover = popover.clone();
                let remove = session_menu_button(
                    ("workspace-remove", index as u64),
                    "移除工作区",
                    IconName::Delete,
                )
                .on_click(move |_, window, cx| {
                    remove_app.update(cx, |this, cx| {
                        this.open_remove_workspace_dialog(index, window, cx)
                    });
                    if let Some(popover) = remove_popover.clone().upgrade() {
                        popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                    }
                });
                v_flex().w(px(170.)).gap(px(1.)).child(remove)
            })
            .into_any_element()
    }
    fn collect_search_results(&self, query: &str) -> Vec<SearchHit> {
        let mut hits = Vec::new();
        'outer: for (workspace_index, workspace) in self.store.workspaces.iter().enumerate() {
            let workspace_name = workspace.name();
            for session in workspace.sessions.iter().rev() {
                let title_matches =
                    !query.is_empty() && session.title.to_lowercase().contains(query);
                let matching_message = if query.is_empty() {
                    None
                } else {
                    session
                        .messages
                        .iter()
                        .find(|message| message.text.to_lowercase().contains(query))
                };
                if !query.is_empty() && !title_matches && matching_message.is_none() {
                    continue;
                }
                let snippet_source = matching_message
                    .map(|message| message.text.as_str())
                    .or_else(|| {
                        session
                            .messages
                            .iter()
                            .find(|message| message.kind == ItemKind::User)
                            .map(|message| message.text.as_str())
                    })
                    .or_else(|| {
                        session
                            .messages
                            .first()
                            .map(|message| message.text.as_str())
                    })
                    .unwrap_or("");
                hits.push(SearchHit {
                    workspace: workspace_index,
                    session: session.id,
                    provider: session.provider,
                    workspace_name: workspace_name.clone(),
                    title: if session.title.is_empty() {
                        "新会话".into()
                    } else {
                        session.title.clone()
                    },
                    archived: session.archived,
                    snippet: search_snippet(snippet_source, query),
                });
                if hits.len() >= 40 {
                    break 'outer;
                }
            }
        }
        hits
    }
    fn open_search_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let viewport_height = f32::from(window.viewport_size().height);
            let height = (viewport_height - 160.).clamp(320., 560.);
            let margin_top = ((viewport_height - height) * 0.5).max(24.);
            dialog
                .w(px(640.))
                .margin_top(px(margin_top))
                .title("搜索会话")
                .content(move |content, _, cx| {
                    let this = view.read(cx);
                    let input = this.search.clone();
                    let query = this.search_query.trim().to_lowercase();
                    let hits = this.collect_search_results(&query);
                    let mut body = v_flex().w_full().gap_2();
                    body = body.child(
                        Input::new(&input)
                            .h(px(34.))
                            .prefix(Icon::new(IconName::Search).small().text_color(rgb(FAINT))),
                    );
                    if hits.is_empty() {
                        body = body.child(
                            div()
                                .py_2()
                                .text_size(px(12.))
                                .text_color(rgb(FAINT))
                                .child(if query.is_empty() {
                                    "输入会话标题或消息内容开始搜索"
                                } else {
                                    "没有匹配的会话"
                                }),
                        );
                    } else {
                        let mut list = v_flex()
                            .id("search-results")
                            .w_full()
                            .max_h(px((height - 150.).max(160.)))
                            .overflow_y_scroll()
                            .gap(px(1.));
                        for hit in hits {
                            let SearchHit {
                                workspace,
                                session,
                                provider,
                                workspace_name,
                                title,
                                archived,
                                snippet,
                            } = hit;
                            let app = view.clone();
                            list = list.child(
                                div()
                                    .id(("search-hit", session))
                                    .w_full()
                                    .px_2()
                                    .py_2()
                                    .gap_1()
                                    .rounded(px(8.))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                                    .on_click(move |_, window, cx| {
                                        app.update(cx, |this, cx| {
                                            this.switch(workspace, Some(session), window, cx)
                                        });
                                        window.close_dialog(cx);
                                    })
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(provider_icon(provider))
                                            .child(
                                                div().flex_1().min_w_0().child(highlighted_text(
                                                    &title, &query, 13., TEXT,
                                                )),
                                            )
                                            .when(archived, |row| {
                                                row.child(
                                                    div()
                                                        .text_size(px(9.))
                                                        .text_color(rgb(FAINT))
                                                        .child("已归档"),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(FAINT))
                                                    .child(workspace_name),
                                            ),
                                    )
                                    .when(!snippet.is_empty(), |row| {
                                        row.child(highlighted_text(&snippet, &query, 11., MUTED))
                                    }),
                            );
                        }
                        body = body.child(list);
                    }
                    content.child(body)
                })
        });
        cx.notify();
    }
    fn session_menu(&self, id: u64, app: Entity<Self>) -> AnyElement {
        let archived = self.session(id).is_some_and(|session| session.archived);
        let trigger = Button::new(("session-menu-trigger", id))
            .ghost()
            .small()
            .compact()
            .icon(IconName::Ellipsis)
            .cursor_pointer()
            .on_click(|_, _, cx| cx.stop_propagation());
        Popover::new(("session-menu", id))
            .anchor(Anchor::LeftCenter)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let rename_app = app.clone();
                let rename_popover = popover.clone();
                let rename =
                    session_menu_button(("session-rename", id), "重命名", IconName::Replace)
                        .on_click(move |_, window, cx| {
                            rename_app
                                .update(cx, |this, cx| this.open_rename_dialog(id, window, cx));
                            if let Some(popover) = rename_popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        });
                let archive_app = app.clone();
                let archive_popover = popover.clone();
                let archive = session_menu_button(
                    ("session-archive", id),
                    if archived { "取消归档" } else { "归档" },
                    IconName::Inbox,
                )
                .on_click(move |_, window, cx| {
                    archive_app.update(cx, |this, cx| this.toggle_archive(id, cx));
                    if let Some(popover) = archive_popover.clone().upgrade() {
                        popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                    }
                });
                let delete_app = app.clone();
                let delete_popover = popover.clone();
                let delete = session_menu_button(("session-delete", id), "删除", IconName::Delete)
                    .on_click(move |_, window, cx| {
                        delete_app.update(cx, |this, cx| this.open_delete_dialog(id, window, cx));
                        if let Some(popover) = delete_popover.clone().upgrade() {
                            popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                        }
                    });
                v_flex()
                    .w(px(150.))
                    .gap(px(1.))
                    .child(rename)
                    .child(archive)
                    .child(delete)
            })
            .into_any_element()
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
    fn switch(
        &mut self,
        workspace: usize,
        session: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected_session {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
        }
        self.selected_workspace = Some(workspace);
        self.selected_session = session;
        self.detail_text = None;
        self.message = None;
        let draft = session
            .and_then(|id| self.drafts.get(&id))
            .cloned()
            .unwrap_or_default();
        self.input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        if session.is_some() {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.scroll.set_offset(point(px(0.), px(0.)));
        // The new list starts at its top; let the next poll re-evaluate whether
        // that is already the end instead of keeping the previous session's pin.
        self.at_bottom = true;
        if let Some(provider) = session
            .and_then(|id| self.session(id))
            .map(|session| session.provider)
        {
            self.new_session_provider = provider;
            self.remember_provider(provider);
        }
        // Branch metadata and diffs always belong to the selected workspace.
        self.sync_git_workspace(cx);
        cx.notify();
    }
    fn workspace_busy(&self, index: usize) -> bool {
        self.git_operation.is_some()
            || self.store.workspaces[index]
                .sessions
                .iter()
                .any(|s| self.runs.contains_key(&s.id))
    }
    fn add_workspace(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择你信任的工作区目录".into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        match path.canonicalize() {
                            Ok(path) if path.is_dir() => {
                                let index = this
                                    .store
                                    .workspaces
                                    .iter()
                                    .position(|w| w.path == path)
                                    .unwrap_or_else(|| {
                                        this.store.workspaces.push(Workspace {
                                            path,
                                            sessions: vec![],
                                        });
                                        this.store.workspaces.len() - 1
                                    });
                                let session =
                                    this.store.workspaces[index].sessions.last().map(|s| s.id);
                                this.switch(index, session, window, cx);
                                this.persist();
                            }
                            _ => this.message = Some("目录不存在或不可访问".into()),
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.message = Some(format!("打开目录失败：{other:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn new_session(
        &mut self,
        index: usize,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.create_session(index, provider, cx) {
            self.switch(index, Some(id), window, cx);
        }
    }
    /// Append a session without touching the composer draft. Used by the "+"
    /// buttons (which then switch to it) and by "type to start" (which sends
    /// the current prompt immediately).
    fn create_session(
        &mut self,
        index: usize,
        provider: Provider,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if !self.writable || self.closing || index >= self.store.workspaces.len() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.catalogs.insert(provider, models::load(provider));
        self.ccs = ccs::load();
        let model = self
            .catalogs
            .get(&provider)
            .and_then(|catalog| catalog.current.clone());
        self.store.workspaces[index].sessions.push(SessionRecord {
            id,
            provider,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            exit_code: None,
            title: "新会话".into(),
            archived: false,
            native_id: None,
            model,
            messages: vec![],
            interrupted: false,
        });
        self.selected_workspace = Some(index);
        self.selected_session = Some(id);
        self.new_session_provider = provider;
        self.remember_provider(provider);
        self.persist();
        cx.notify();
        Some(id)
    }
    fn remember_provider(&mut self, provider: Provider) {
        if self.store.ui.last_provider != Some(provider) {
            self.store.ui.last_provider = Some(provider);
            self.persist();
        }
    }
    fn reload_models(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.catalogs.insert(provider, models::load(provider));
        self.ccs = ccs::load();
        cx.notify();
    }
    fn reload_all_models(&mut self, cx: &mut Context<Self>) {
        for provider in [Provider::Claude, Provider::Codex] {
            self.catalogs.insert(provider, models::load(provider));
        }
        self.ccs = ccs::load();
        cx.notify();
    }
    fn set_session_model(&mut self, id: u64, model: Option<String>, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        if let Some(session) = self.session_mut(id) {
            session.model = model;
        }
        self.persist();
        cx.notify();
    }
    fn send_message(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.send_prompt(window, cx);
    }
    fn send_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        let Some(index) = self.selected_workspace else {
            self.message = Some("请先在左上角选择工作区。".into());
            cx.notify();
            return;
        };
        if self.workspace_busy(index) {
            self.message = Some("该工作区已有任务运行，请等待完成或取消后发送。".into());
            cx.notify();
            return;
        }
        let prompt = self.input.read(cx).value().to_string();
        if prompt.trim().is_empty() {
            return;
        }
        if prompt.len() > 256 * 1024 {
            self.message = Some("单条消息最多 256 KiB。".into());
            cx.notify();
            return;
        }
        // Resolve the target engine and validate the environment before creating
        // a session, so a failed send cannot leave an empty conversation behind.
        let provider = match self.selected_session {
            Some(id) => self.session(id).unwrap().provider,
            None => self.new_session_provider,
        };
        if let Some(id) = self.selected_session {
            let session = self.session(id).unwrap();
            if session.native_id.is_none()
                && session
                    .messages
                    .iter()
                    .any(|m| m.kind == ItemKind::Assistant)
            {
                self.message =
                    Some("该会话缺少可靠的 CLI 会话标识，请新建会话，避免丢失上下文。".into());
                cx.notify();
                return;
            }
        }
        let executable = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(error) => {
                self.message = Some(format!(
                    "{}：{error:#}。可在左下角「设置」中检查路径。",
                    provider.label()
                ));
                cx.notify();
                return;
            }
        };
        let cwd = self.store.workspaces[index].path.clone();
        if !cwd.is_dir() {
            self.message = Some("工作区目录已移动或不可访问。".into());
            cx.notify();
            return;
        }
        // Typing in a workspace without a selected session starts one with the
        // last used engine instead of asking the user to click "新建会话".
        let id = match self.selected_session {
            Some(id) => id,
            None => match self.create_session(index, provider, cx) {
                Some(id) => id,
                None => return,
            },
        };
        let native_id = self
            .session(id)
            .and_then(|session| session.native_id.clone());
        let model = self.session(id).and_then(|session| session.model.clone());
        let session = self.session_mut(id).unwrap();
        let prefix = format!("turn-{}", session.messages.len());
        if !session.messages.iter().any(|m| m.kind == ItemKind::User) {
            session.title = prompt
                .lines()
                .next()
                .unwrap_or("新会话")
                .chars()
                .take(26)
                .collect();
        }
        session.messages.push(ChatItem {
            key: format!("{prefix}-user"),
            kind: ItemKind::User,
            text: prompt.clone(),
            input: String::new(),
            detail: String::new(),
            status: String::new(),
        });
        session.interrupted = true;
        if !self.persist() {
            let session = self.session_mut(id).unwrap();
            session.messages.pop();
            session.interrupted = false;
            cx.notify();
            return;
        }
        let runtime = ChatRuntime::start(provider, executable, cwd, prompt, native_id, model);
        self.runs.insert(
            id,
            ActiveRun {
                runtime,
                approvals: ApprovalQueue::default(),
                prefix,
                status: "正在连接 CLI…".into(),
            },
        );
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        self.drafts.remove(&id);
        self.message = None;
        self.scroll_to_end();
        cx.notify();
    }
    fn scroll_to_end(&self) {
        self.scroll.scroll_to_bottom();
    }
    fn selected_workspace_path(&self) -> Option<PathBuf> {
        self.selected_workspace
            .and_then(|index| self.store.workspaces.get(index))
            .map(|workspace| workspace.path.clone())
    }
    fn ensure_commit_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.commit_drafts.contains_key(&path) {
            return;
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("填写 Commit Message…")
                .auto_grow(3, 3)
        });
        let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        self.commit_drafts.insert(
            path,
            CommitDraft {
                input,
                _subscription: subscription,
            },
        );
        cx.notify();
    }
    fn git_feedback(
        &mut self,
        path: PathBuf,
        text: impl Into<String>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.git_feedback.insert(
            path,
            GitFeedback {
                text: text.into(),
                error,
            },
        );
        cx.notify();
    }
    fn start_commit(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git_operation.is_some()
            || self.commit_generation.is_some()
        {
            return;
        }
        if !self.runs.is_empty() {
            self.git_feedback(
                path,
                "请等待 CLI 任务结束后提交，避免暂存区同时变化。",
                true,
                cx,
            );
            return;
        }
        let Some(snapshot) = &self.git_snapshot else {
            return;
        };
        if self.git_probe.is_some() {
            return;
        }
        let message = self
            .commit_drafts
            .get(&path)
            .map(|draft| draft.input.read(cx).value().to_string())
            .unwrap_or_default();
        if message.trim().is_empty() {
            self.git_feedback(path, "请填写 Commit Message，或点击 AI 生成。", true, cx);
            return;
        }
        let operation = git::Operation::Commit {
            message: message.clone(),
            target: snapshot.target.clone(),
            push: self.commit_and_push,
        };
        self.git_operation = Some(GitOperation {
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: message,
        });
        self.git_feedback(
            path,
            if self.commit_and_push {
                "正在提交并推送…"
            } else {
                "正在提交…"
            },
            false,
            cx,
        );
    }
    fn start_push(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git_operation.is_some()
            || self.commit_generation.is_some()
        {
            return;
        }
        if !self.runs.is_empty() {
            self.git_feedback(path, "请等待 CLI 任务结束后推送。", true, cx);
            return;
        }
        let Some(repository) = &self.git_repository else {
            return;
        };
        let Some(upstream) = repository.upstream.clone() else {
            self.details = true;
            self.detail_view = DetailView::Git;
            self.refresh_git(cx);
            self.git_feedback(
                path,
                "当前分支没有有效上游。请先在终端设置 upstream，再刷新并推送。",
                true,
                cx,
            );
            return;
        };
        let operation = git::Operation::Push {
            branch: repository.current.clone(),
            upstream,
        };
        self.git_operation = Some(GitOperation {
            probe: git::OperationProbe::start(path.clone(), operation),
            draft: String::new(),
        });
        self.git_feedback(path, "正在推送…", false, cx);
    }
    fn push_button(&self, view: Entity<Self>) -> Option<AnyElement> {
        let repository = self
            .git_repository
            .as_ref()
            .filter(|repository| !repository.detached && repository.ahead > 0)?;
        let busy = self.git_operation.is_some();
        let disabled = self.closing
            || !self.writable
            || busy
            || self.commit_generation.is_some()
            || !self.runs.is_empty();
        let tooltip = match &repository.upstream {
            Some(upstream) => format!("推送 {} 个提交到 {}", repository.ahead, upstream),
            None => format!("{} 个本地未发布提交；尚未设置上游", repository.ahead),
        };
        Some(
            Button::new("git-push-pending")
                .ghost()
                .small()
                .h(px(26.))
                .px_2()
                .tooltip(tooltip)
                .disabled(disabled)
                .loading(busy)
                .when(!disabled, |button| button.cursor_pointer())
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(gpui_kit::assets::IconName::ArrowUpFromLine).small())
                        .child(
                            div()
                                .min_w(px(16.))
                                .h(px(16.))
                                .px_1()
                                .rounded_full()
                                .bg(rgb(SURFACE_HOVER))
                                .text_size(px(10.))
                                .text_color(rgb(TEXT))
                                .child(repository.ahead.to_string()),
                        ),
                )
                .on_click(move |_, _, cx| view.update(cx, |this, cx| this.start_push(cx)))
                .into_any_element(),
        )
    }
    fn generate_commit_message(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_workspace_path() else {
            return;
        };
        if self.closing
            || !self.writable
            || self.git_operation.is_some()
            || self.commit_generation.is_some()
        {
            return;
        }
        let Some(snapshot) = self
            .git_snapshot
            .as_ref()
            .filter(|snapshot| !snapshot.target.staged.is_empty())
        else {
            return;
        };
        let provider = self
            .selected_session
            .and_then(|id| self.session(id))
            .map(|session| session.provider)
            .unwrap_or(self.new_session_provider);
        let model = self
            .selected_session
            .and_then(|id| self.session(id))
            .and_then(|session| session.model.clone());
        let executable = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(error) => {
                self.git_feedback(
                    path,
                    format!("无法生成 Commit Message：{error:#}"),
                    true,
                    cx,
                );
                return;
            }
        };
        let original = self
            .commit_drafts
            .get(&path)
            .map(|draft| draft.input.read(cx).value().to_string())
            .unwrap_or_default();
        let files: Vec<_> = snapshot
            .files
            .iter()
            .filter(|file| file.staged())
            .map(|file| file.path.as_str())
            .collect();
        let prompt = self
            .store
            .git
            .generation_prompt(&files, &snapshot.staged_diff);
        self.commit_generation = Some(CommitGeneration {
            path: path.clone(),
            runtime: ChatRuntime::start_text(provider, executable, prompt, model),
            started: Instant::now(),
            original,
            parts: vec![],
            error: None,
        });
        self.git_feedback(
            path,
            format!("正在使用 {} 生成 Commit Message…", provider.label()),
            false,
            cx,
        );
    }
    fn poll_git_actions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_commit_draft(window, cx);
        let completed = self.git_operation.as_ref().and_then(|operation| {
            match operation.probe.events.try_recv() {
                Ok(result) => Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(
                    "Git 操作线程已退出，请刷新仓库状态确认结果，避免重复提交。".into(),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            }
        });
        if let Some(result) = completed {
            let operation = self.git_operation.take().unwrap();
            let path = operation.probe.path.clone();
            match result {
                Ok(result) => {
                    if result.error.is_some()
                        && self.selected_workspace_path().as_ref() == Some(&path)
                    {
                        self.details = true;
                        self.detail_view = DetailView::Git;
                    }
                    if result.committed
                        && let Some(draft) = self.commit_drafts.get(&path)
                        && draft.input.read(cx).value().as_ref() == operation.draft
                    {
                        draft
                            .input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    let text = match &result.error {
                        Some(error) => format!("{}：{}", result.message, error),
                        None => result.message,
                    };
                    self.git_feedback(path.clone(), text, result.error.is_some(), cx);
                }
                Err(error) => self.git_feedback(path.clone(), error, true, cx),
            }
            if self.selected_workspace_path().as_ref() == Some(&path) {
                self.branch_probe = None;
                self.refresh_branches();
                self.refresh_git(cx);
            }
        }
        let mut generation_result = None;
        if let Some(generation) = &mut self.commit_generation {
            if generation.started.elapsed() > Duration::from_secs(120) && generation.error.is_none()
            {
                generation.error = Some("AI 生成超时，请重试。".into());
                generation.runtime.cancel();
            }
            for _ in 0..1000 {
                let event = match generation.runtime.events.try_recv() {
                    Ok(event) => event,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        generation_result = Some(Some("AI 工作线程已退出，请重试。".into()));
                        break;
                    }
                };
                match event {
                    Event::Text { key, text } => {
                        update_commit_text(&mut generation.parts, key, text, true)
                    }
                    Event::Delta { key, text } => {
                        update_commit_text(&mut generation.parts, key, text, false)
                    }
                    Event::Approval(approval) => {
                        generation.runtime.decide(approval.id, false);
                        generation.error =
                            Some("AI 请求了工具权限，已停止；请重试生成纯文本说明。".into());
                        generation.runtime.cancel();
                    }
                    Event::Tool { .. } | Event::ToolInput { .. } => {
                        generation.error = Some("AI 尝试调用工具，已停止生成。".into());
                        generation.runtime.cancel();
                    }
                    Event::Finished { error, cancelled } => {
                        generation_result = Some(
                            generation
                                .error
                                .clone()
                                .or(error)
                                .or_else(|| cancelled.then(|| "已取消生成。".into())),
                        );
                        break;
                    }
                    _ => {}
                }
                if generation
                    .parts
                    .iter()
                    .map(|(_, text)| text.len())
                    .sum::<usize>()
                    > 16 * 1024
                {
                    generation.error = Some("AI 返回的 Commit Message 过长，已停止。".into());
                    generation.runtime.cancel();
                }
            }
        }
        if let Some(error) = generation_result {
            let generation = self.commit_generation.take().unwrap();
            let text = generation
                .parts
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("\n\n");
            if let Some(error) = error {
                self.git_feedback(generation.path, error, true, cx);
            } else if text.trim().is_empty() {
                self.git_feedback(
                    generation.path,
                    "AI 没有返回 Commit Message，请重试。",
                    true,
                    cx,
                );
            } else if let Some(draft) = self.commit_drafts.get(&generation.path) {
                if draft.input.read(cx).value().as_ref() == generation.original {
                    draft
                        .input
                        .update(cx, |input, cx| input.set_value(text.trim(), window, cx));
                    self.git_feedback(
                        generation.path,
                        "Commit Message 已生成，可编辑后提交。",
                        false,
                        cx,
                    );
                } else {
                    self.git_feedback(
                        generation.path,
                        "草稿已修改，未覆盖；可重新生成。",
                        true,
                        cx,
                    );
                }
            }
        }
    }
    fn refresh_branches(&mut self) {
        if self.closing || self.branch_probe.is_some() {
            return;
        }
        self.last_branch_check = Instant::now();
        self.branch_probe = self.selected_workspace_path().map(BranchProbe::start);
    }
    fn commit_editor(&self, view: Entity<Self>, cx: &App) -> AnyElement {
        let Some(path) = self.selected_workspace_path() else {
            return div().into_any_element();
        };
        let Some(draft) = self.commit_drafts.get(&path) else {
            return div().into_any_element();
        };
        let generating = self
            .commit_generation
            .as_ref()
            .is_some_and(|generation| generation.path == path);
        let busy = self.git_operation.is_some() || self.commit_generation.is_some();
        let staged = self.git_snapshot.as_ref().is_some_and(|snapshot| {
            !snapshot.target.staged.is_empty() && snapshot.target.branch.is_some()
        });
        let disabled = self.closing
            || !self.writable
            || busy
            || !self.runs.is_empty()
            || self.git_probe.is_some()
            || !staged;
        let can_commit = !disabled && !draft.input.read(cx).value().trim().is_empty();
        let can_generate = !disabled;
        let generate_app = view.clone();
        let commit_app = view.clone();
        let cancel_app = view.clone();
        let action_label = if self.commit_and_push {
            "Commit and Push"
        } else {
            "Commit"
        };
        let dropdown = Popover::new("git-commit-mode")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(
                Button::new("git-commit-mode-trigger")
                    .ghost()
                    .small()
                    .compact()
                    .icon(IconName::ChevronDown)
                    .tooltip("选择提交操作")
                    .disabled(busy || self.closing)
                    .when(!busy && !self.closing, |button| button.cursor_pointer()),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(180.)).gap_1();
                for push in [false, true] {
                    let app = view.clone();
                    let popover = popover.clone();
                    list = list.child(
                        Button::new(if push {
                            "git-mode-commit-push"
                        } else {
                            "git-mode-commit"
                        })
                        .accessibility_label(if push { "Commit and Push" } else { "Commit" })
                        .small()
                        .ghost()
                        .w_full()
                        .child(div().w_full().text_left().child(if push {
                            "Commit and Push"
                        } else {
                            "Commit"
                        }))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.commit_and_push = push;
                                cx.notify();
                            });
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |state, cx| state.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list
            });
        let mut editor = v_flex()
            .w_full()
            .flex_shrink_0()
            .pt_3()
            .gap_2()
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .font_semibold()
                            .child("Commit Message"),
                    )
                    .child(
                        Button::new("git-generate-message")
                            .ghost()
                            .small()
                            .compact()
                            .icon(gpui_kit::assets::IconName::Sparkles)
                            .tooltip("Generate Commit Message（使用当前引擎和模型）")
                            .disabled(!can_generate)
                            .loading(generating)
                            .when(can_generate, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                generate_app.update(cx, |this, cx| this.generate_commit_message(cx))
                            }),
                    )
                    .when(generating, |row| {
                        row.child(
                            Button::new("git-cancel-generate")
                                .ghost()
                                .small()
                                .compact()
                                .icon(gpui_kit::assets::IconName::X)
                                .tooltip("取消生成")
                                .cursor_pointer()
                                .on_click(move |_, _, cx| {
                                    cancel_app.update(cx, |this, cx| {
                                        if let Some(generation) = &this.commit_generation {
                                            generation.runtime.cancel();
                                        }
                                        cx.notify();
                                    })
                                }),
                        )
                    }),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_2()
                    .rounded(px(8.))
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(
                        Textarea::new(&draft.input)
                            .appearance(false)
                            .bordered(false)
                            .h(px(78.))
                            .text_size(px(12.))
                            .aria_label("Git Commit Message")
                            .disabled(
                                self.closing
                                    || !self.writable
                                    || generating
                                    || self
                                        .git_operation
                                        .as_ref()
                                        .is_some_and(|operation| operation.probe.path == path),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new("git-commit")
                                    .label(action_label)
                                    .primary()
                                    .small()
                                    .disabled(!can_commit)
                                    .loading(
                                        self.git_operation
                                            .as_ref()
                                            .is_some_and(|operation| operation.probe.path == path),
                                    )
                                    .when(can_commit, |button| button.cursor_pointer())
                                    .on_click(move |_, _, cx| {
                                        commit_app.update(cx, |this, cx| this.start_commit(cx))
                                    }),
                            )
                            .child(dropdown),
                    ),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(FAINT))
                    .child(if !self.runs.is_empty() {
                        "CLI 任务运行中，结束后可提交"
                    } else if self
                        .git_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.target.branch.is_none())
                    {
                        "分离 HEAD：请先切回分支"
                    } else if self
                        .git_snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.target.staged.is_empty())
                    {
                        "没有已暂存变更，请先暂存后刷新"
                    } else {
                        "仅提交已暂存变更"
                    }),
            );
        if let Some(feedback) = self.git_feedback.get(&path) {
            editor = editor.child(
                div()
                    .id("git-action-feedback")
                    .max_h(px(60.))
                    .overflow_y_scroll()
                    .text_size(px(11.))
                    .text_color(rgb(if feedback.error { 0xe6a1a1 } else { MUTED }))
                    .child(feedback.text.clone()),
            );
        }
        editor.into_any_element()
    }
    fn sync_git_workspace(&mut self, cx: &mut Context<Self>) {
        let path = self.selected_workspace_path();
        if self.git_workspace == path {
            return;
        }
        self.git_workspace = path;
        self.branch_probe = None;
        self.git_repository = None;
        self.git_probe = None;
        self.git_snapshot = None;
        self.git_error = None;
        self.refresh_branches();
        if self.details && self.detail_view == DetailView::Git {
            self.refresh_git(cx);
        }
        cx.notify();
    }
    fn poll_branches(&mut self, cx: &mut Context<Self>) {
        self.sync_git_workspace(cx);
        let update = self
            .branch_probe
            .as_ref()
            .and_then(|probe| match probe.events.try_recv() {
                Ok(result) => Some((probe.path.clone(), result.ok())),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some((probe.path.clone(), None))
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        if let Some((path, repository)) = update {
            self.branch_probe = None;
            if self.selected_workspace_path().as_ref() == Some(&path)
                && self.git_repository != repository
            {
                let head_changed = self
                    .git_repository
                    .as_ref()
                    .map(|repo| (&repo.current, repo.detached))
                    != repository
                        .as_ref()
                        .map(|repo| (&repo.current, repo.detached));
                self.git_repository = repository;
                if head_changed
                    && self.details
                    && self.detail_view == DetailView::Git
                    && self.git_probe.is_none()
                {
                    self.refresh_git(cx);
                }
                cx.notify();
            }
        }
        // Pick up changes made in an external terminal, even with the dock closed.
        if self.last_branch_check.elapsed() >= Duration::from_secs(10) {
            self.refresh_branches();
        }
    }
    fn branch_picker(&self, view: Entity<Self>) -> AnyElement {
        let Some(repository) = self.git_repository.clone() else {
            return div().into_any_element();
        };
        let label = if repository.detached {
            format!("HEAD · {}", repository.current)
        } else {
            repository.current.clone()
        };
        let trigger = Button::new("git-branch-trigger")
            .ghost()
            .small()
            .h(px(26.))
            .px_2()
            .cursor_pointer()
            .tooltip(format!("当前分支：{label} · 查看分支（只读）"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(gpui_kit::assets::IconName::GitBranch).small())
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(11.))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(FAINT)),
                    ),
            );
        Popover::new(SharedString::from(format!(
            "git-branch-picker-{}",
            self.git_workspace
                .as_deref()
                .map(display_path)
                .unwrap_or_default()
        )))
        .anchor(Anchor::BottomLeft)
        .offset(px(6.))
        .trigger(trigger)
        .content(move |_, _, cx| {
            let popover = cx.entity().downgrade();
            let app = view.clone();
            let mut list = v_flex()
                .id("git-branches")
                .w(px(280.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .gap_1()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child("工作区分支 · 只读"),
                );
            if repository.detached {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(format!("分离 HEAD · {}", repository.current)),
                );
            }
            for remote in [false, true] {
                if !repository
                    .branches
                    .iter()
                    .any(|branch| branch.remote == remote)
                {
                    continue;
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child(if remote {
                            "远程分支（本地记录）"
                        } else {
                            "本地分支"
                        }),
                );
                for (index, branch) in repository
                    .branches
                    .iter()
                    .enumerate()
                    .filter(|(_, b)| b.remote == remote)
                {
                    let current =
                        !repository.detached && !remote && branch.name == repository.current;
                    list = list.child(
                        h_flex()
                            .id(("git-branch-row", index))
                            .px_2()
                            .py_1()
                            .gap_2()
                            .rounded(px(4.))
                            .when(current, |row| row.bg(rgb(SURFACE_HOVER)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(TEXT))
                                    .child(branch.name.clone()),
                            )
                            .when(current, |row| {
                                row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(MUTED))
                                        .child("当前"),
                                )
                            }),
                    );
                }
            }
            if repository.truncated {
                list = list.child(
                    div()
                        .px_2()
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child("分支过多，列表已截断"),
                );
            }
            list.child(
                h_flex()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(
                        Button::new("git-branches-refresh")
                            .label("刷新分支")
                            .small()
                            .ghost()
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                app.update(cx, |this, _| this.refresh_branches());
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                                }
                            }),
                    ),
            )
        })
        .into_any_element()
    }
    /// Start a background read-only Git snapshot for the selected workspace.
    fn refresh_git(&mut self, cx: &mut Context<Self>) {
        if self.last_branch_check.elapsed() >= Duration::from_secs(1) {
            self.refresh_branches();
        }
        self.git_probe = None;
        self.git_snapshot = None;
        let Some(path) = self.selected_workspace_path() else {
            self.git_error = Some("未选择工作区".into());
            cx.notify();
            return;
        };
        self.git_error = None;
        self.git_probe = Some(GitProbe::start(path));
        cx.notify();
    }
    /// Collect a finished Git probe; stale snapshots for another workspace are
    /// dropped so switching sessions cannot show the previous directory's diff.
    fn poll_git(&mut self, cx: &mut Context<Self>) {
        let update = self
            .git_probe
            .as_ref()
            .and_then(|probe| match probe.events.try_recv() {
                Ok(result) => Some((probe.path.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some((
                    probe.path.clone(),
                    Err("Git 读取线程已退出，请重试".to_string()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
            });
        let Some((path, result)) = update else {
            return;
        };
        self.git_probe = None;
        if self.selected_workspace_path().as_deref() != Some(path.as_path()) {
            return;
        }
        match result {
            Ok(snapshot) => {
                self.git_snapshot = Some(snapshot);
                self.git_error = None;
            }
            Err(error) => {
                self.git_snapshot = None;
                self.git_error = Some(error);
            }
        }
        cx.notify();
    }
    /// Read-only Git view for the right-side detail panel.
    fn git_panel(&self, view: Entity<Self>) -> AnyElement {
        let refresh = |label: &'static str, id: &'static str| -> AnyElement {
            let app = view.clone();
            Button::new(id)
                .label(label)
                .small()
                .ghost()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    app.update(cx, |this, cx| this.refresh_git(cx));
                })
                .into_any_element()
        };
        let mut body = v_flex()
            .id("git-panel-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap_3();
        if self.git_probe.is_some() {
            return body
                .child(
                    h_flex().child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child("正在读取工作区 Git 状态…"),
                    ),
                )
                .child(h_flex().child(refresh("重新读取", "git-refresh-loading")))
                .into_any_element();
        }
        if let Some(error) = &self.git_error {
            return body
                .child(
                    v_flex()
                        .w_full()
                        .p_3()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(rgb(0x5c332c))
                        .bg(rgb(0x2a1d1a))
                        .text_size(px(11.))
                        .line_height(relative(1.4))
                        .text_color(rgb(0xe6b4a8))
                        .child(error.clone()),
                )
                .child(h_flex().child(refresh("重新读取", "git-refresh-error")))
                .into_any_element();
        }
        let Some(snapshot) = &self.git_snapshot else {
            return body
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(FAINT))
                        .child("打开面板时读取工作区 Git 状态"),
                )
                .into_any_element();
        };
        body = body
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::GitBranch)
                            .small()
                            .text_color(rgb(FAINT)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child(snapshot.branch.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(9.))
                            .text_color(rgb(FAINT))
                            .child("变更预览"),
                    )
                    .child(refresh("刷新", "git-refresh")),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(FAINT))
                    .child(format!(
                        "暂存 {} · 未暂存 {} · 未跟踪 {}",
                        snapshot.staged_count(),
                        snapshot.unstaged_count(),
                        snapshot.untracked_count()
                    )),
            );
        if snapshot.is_clean() {
            body = body.child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("工作区没有未提交变更"),
            );
        } else {
            let mut list = v_flex().w_full().gap(px(1.));
            for file in &snapshot.files {
                let color = match file.label() {
                    "新增" => 0x8fd694,
                    "删除" => 0xe6a1a1,
                    "冲突" => 0xe3b96b,
                    "未跟踪" => FAINT,
                    _ => MUTED,
                };
                list = list.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .py(px(2.))
                        .child(
                            div()
                                .w(px(48.))
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(rgb(color))
                                .child(file.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(rgb(TEXT))
                                .child(file.path.clone()),
                        )
                        .when_some(file.original.clone(), |row, original| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(9.))
                                    .text_color(rgb(FAINT))
                                    .child(format!("← {original}")),
                            )
                        }),
                );
            }
            body = body.child(list);
        }
        for (title, diff) in [
            ("暂存变更", snapshot.staged_diff.as_str()),
            ("未暂存变更", snapshot.unstaged_diff.as_str()),
        ] {
            if diff.is_empty() {
                continue;
            }
            body = body
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .pt_1()
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_semibold()
                                .text_color(rgb(MUTED))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(rgb(FAINT))
                                .child(format!("{} 行", diff.lines().count())),
                        ),
                )
                .child(diff_element(diff));
        }
        for note in &snapshot.notes {
            body = body.child(
                div()
                    .text_size(px(10.))
                    .line_height(relative(1.4))
                    .text_color(rgb(FAINT))
                    .child(note.clone()),
            );
        }
        body.into_any_element()
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dirty && self.last_save.elapsed() >= Duration::from_secs(1) {
            self.persist();
            cx.notify();
        }
        let mut completed = vec![];
        for (provider, probe) in &self.probes {
            match probe.events.try_recv() {
                Ok(result) => completed.push((*provider, probe.path.clone(), result)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => completed.push((
                    *provider,
                    probe.path.clone(),
                    Err("检测线程已退出，请重新检测".into()),
                )),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        for (provider, path, result) in completed {
            self.probes.remove(&provider);
            let text = if cli::resolve(provider, &self.store.cli)
                .ok()
                .is_none_or(|cli| cli.path != path)
            {
                "配置已变更，请重新检测。".into()
            } else {
                match result {
                    Ok(info) => format!(
                        "版本 {} · {}",
                        info.version,
                        if info.verified_version {
                            "与本项目已验证版本一致"
                        } else {
                            "尚未经过完整回归"
                        }
                    ),
                    Err(error) if error == "检测已取消" => error,
                    Err(error) => format!("检测失败：{error}"),
                }
            };
            self.diagnostics.insert(provider, text);
            cx.notify();
        }
        self.poll_branches(cx);
        self.poll_git(cx);
        self.poll_git_actions(window, cx);
        let follow = f32::from(self.scroll.offset().y + self.scroll.max_offset().y).abs() < 60.;
        // Re-render when the scroll position crosses the "at the end" threshold
        // so the floating pin appears/disappears without polling every frame.
        if follow != self.at_bottom {
            self.at_bottom = follow;
            cx.notify();
        }
        let mut incoming = vec![];
        for (id, run) in &self.runs {
            for event in run.runtime.events.try_iter().take(1000) {
                incoming.push((*id, run.prefix.clone(), event));
            }
        }
        let mut changed = false;
        let mut terminal = false;
        let mut visible = false;
        for (id, prefix, event) in incoming {
            changed = true;
            visible |= Some(id) == self.selected_session;
            match event {
                Event::Approval(approval) => {
                    if let Some(run) = self.runs.get_mut(&id) {
                        if self.closing || run.runtime.is_cancelled() {
                            continue;
                        }
                        run.status = "等待你的审批".into();
                        run.approvals.push(approval);
                    }
                }
                Event::ApprovalCancelled(request) => {
                    self.approval_details.remove(&request);
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.approvals.remove(&request);
                        if run.approvals.is_empty() && !run.runtime.is_cancelled() {
                            run.status = "正在执行…".into();
                        }
                    }
                }
                Event::ApprovalResolved { id: request, allow } => {
                    self.approval_details.remove(&request);
                    let approval = self.runs.get_mut(&id).and_then(|run| {
                        let approval = run.approvals.remove(&request);
                        run.status = if run.approvals.is_empty() {
                            "正在执行…"
                        } else {
                            "等待你的审批"
                        }
                        .into();
                        approval
                    });
                    if let (Some(approval), Some(session)) = (approval, self.session_mut(id)) {
                        session.messages.push(ChatItem {
                            key: format!("{prefix}-approval-{}", session.messages.len()),
                            kind: ItemKind::Notice,
                            text: format!(
                                "已提交{}：{}",
                                if allow {
                                    "允许本次操作"
                                } else {
                                    "拒绝本次操作"
                                },
                                approval.title
                            ),
                            input: String::new(),
                            detail: approval.detail,
                            status: String::new(),
                        });
                    }
                }
                Event::Finished { error, cancelled } => {
                    self.runs.remove(&id);
                    terminal = true;
                    if let Some(session) = self.session_mut(id) {
                        session.interrupted = false;
                        finish_tool_cards(
                            session,
                            &prefix,
                            if cancelled {
                                "已取消"
                            } else {
                                "未返回结果"
                            },
                        );
                        // Some CLIs print the failure as assistant text and then
                        // also report it through the result event. Do not show the
                        // same failure twice: the assistant message already carries
                        // the explanation.
                        let already_shown = error.as_deref().is_some_and(|error| {
                            session
                                .messages
                                .iter()
                                .rev()
                                .filter(|item| {
                                    matches!(item.kind, ItemKind::Assistant | ItemKind::Tool)
                                })
                                .take(3)
                                .any(|item| same_failure(&item.text, error))
                        });
                        if !already_shown {
                            session.messages.push(ChatItem {
                                key: format!("{prefix}-finish"),
                                kind: ItemKind::Notice,
                                text: if cancelled {
                                    "已取消；不会自动重试。".into()
                                } else if let Some(error) = error {
                                    format!("执行失败：{error}")
                                } else {
                                    "本轮已完成".into()
                                },
                                input: String::new(),
                                detail: String::new(),
                                status: String::new(),
                            });
                        }
                    }
                }
                Event::NativeId(native_id) => {
                    if let Some(session) = self.session_mut(id) {
                        session.native_id = Some(native_id);
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.status = "正在执行…".into();
                    }
                    self.dirty = true;
                    self.persist();
                }
                other => {
                    if let Some(session) = self.session_mut(id) {
                        apply_event(session, &prefix, other);
                    }
                }
            }
        }
        if changed {
            self.dirty = true;
            if terminal || self.last_save.elapsed() > Duration::from_secs(1) {
                self.persist();
            }
            cx.notify();
            if follow && visible {
                self.scroll_to_end();
            }
            // A finished turn can change workspace files; keep an open Git
            // panel current without requiring the user to press 刷新.
            if terminal {
                self.refresh_branches();
                if self.details && self.detail_view == DetailView::Git && self.git_probe.is_none() {
                    self.refresh_git(cx);
                }
            }
        }
        if self.closing
            && self.runs.is_empty()
            && self.probes.is_empty()
            && self.git_operation.is_none()
            && self.commit_generation.is_none()
        {
            self.persist();
            window.remove_window();
            cx.quit();
        }
    }
    fn choose_cli(&mut self, provider: Provider, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("选择 {} 原生 .exe", provider.label()).into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        match cli::validate_executable(&path) {
                            Ok(path) => this.set_cli_path(provider, Some(path), cx),
                            Err(error) => {
                                this.diagnostics
                                    .insert(provider, format!("路径未保存：{error:#}"));
                            }
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.diagnostics
                        .insert(provider, format!("打开文件失败：{other:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }
    fn set_cli_path(
        &mut self,
        provider: Provider,
        path: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let previous = self.store.cli.clone();
        self.store.cli.set(provider, path);
        if self.persist() {
            if let Some(probe) = self.probes.get(&provider) {
                probe.cancel();
            }
            self.diagnostics
                .insert(provider, "配置已保存，用于后续启动；请检测版本。".into());
        } else {
            self.store.cli = previous;
            self.diagnostics.insert(
                provider,
                self.message
                    .clone()
                    .unwrap_or_else(|| "配置保存失败".into()),
            );
        }
        cx.notify();
    }
    fn probe_cli(&mut self, provider: Provider, cx: &mut Context<Self>) {
        if self.closing || self.probes.contains_key(&provider) {
            return;
        }
        match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => {
                self.probes
                    .insert(provider, VersionProbe::start(provider, cli.path));
                self.diagnostics
                    .insert(provider, "正在运行 --version，最多等待 10 秒…".into());
            }
            Err(error) => {
                self.diagnostics
                    .insert(provider, format!("无法检测：{error:#}"));
            }
        }
        cx.notify();
    }
    fn composer_model_picker(
        &self,
        view: Entity<Self>,
        session_id: u64,
        provider: Provider,
        selected_model: Option<String>,
        disabled: bool,
    ) -> AnyElement {
        let catalog = self.catalogs.get(&provider);
        let synced = self.ccs.current(provider);
        let merged = match (catalog, synced) {
            (Some(catalog), Some(synced)) if !synced.models.is_empty() => {
                catalog.with_synced(&format!("CCS · {}", synced.name), &synced.models)
            }
            (Some(catalog), _) => catalog.clone(),
            (None, Some(synced)) if !synced.models.is_empty() => ModelCatalog::default()
                .with_synced(&format!("CCS · {}", synced.name), &synced.models),
            (None, _) => ModelCatalog::default(),
        };
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| "CLI 默认".into());
        let models = merged.models;
        let mut notes = vec![];
        if let Some(note) =
            catalog.and_then(|catalog| catalog.error.clone().or_else(|| catalog.note.clone()))
        {
            notes.push(format!("配置：{note}"));
        }
        if let Some(synced) = synced {
            notes.push(format!(
                "CCS：{} · {} 个模型",
                synced.name,
                synced.models.len()
            ));
        } else if let Some(error) = &self.ccs.error {
            notes.push(format!("CCS：{error}"));
        } else if let Some(note) = &self.ccs.note {
            notes.push(format!("CCS：{note}"));
        }
        let sync_note = (!notes.is_empty()).then(|| notes.join("；"));
        let provider_label = provider.label();
        let trigger = Button::new("composer-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择模型")
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(Icon::new(IconName::Cpu).small().text_color(rgb(FAINT)))
                    .child(
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(FAINT)),
                    ),
            );
        Popover::new("composer-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let model_count = models.len();
                let mut list = v_flex().w(px(340.)).gap_1();
                list = list.child(
                    h_flex()
                        .px_2()
                        .pt_1()
                        .pb_2()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_semibold()
                                .text_color(rgb(TEXT))
                                .child("会话模型"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(FAINT))
                                .child(format!("{provider_label} · {model_count} 个")),
                        ),
                );
                let option =
                    |index: usize,
                     id: Option<String>,
                     label: String,
                     note: String,
                     selected: bool|
                     -> AnyElement {
                        let view = view.clone();
                        let popover = popover.clone();
                        Button::new(("model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(format!("模型 {label}"))
                            .when(selected, |button| button.bg(rgb(SURFACE_HOVER)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    this.set_session_model(session_id, id, cx);
                                });
                                if let Some(state) = popover.clone().upgrade() {
                                    state.update(cx, |state, cx| state.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(if selected {
                                                        TEXT
                                                    } else {
                                                        MUTED
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(10.))
                                                        .text_color(rgb(FAINT))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .child(Icon::new(IconName::Check).small().text_color(rgb(
                                        if selected { TEXT } else { SURFACE_HOVER },
                                    ))),
                            )
                            .into_any_element()
                    };
                list = list.child(option(
                    0,
                    None,
                    "CLI 默认".into(),
                    format!("跟随 {provider_label} 当前配置"),
                    selected_model.is_none(),
                ));
                for (index, model) in models.iter().enumerate() {
                    let mut note = vec![];
                    if model.label != model.id {
                        note.push(model.id.clone());
                    }
                    if !model.note.is_empty() {
                        note.push(model.note.clone());
                    }
                    list = list.child(option(
                        index + 1,
                        Some(model.id.clone()),
                        model.label.clone(),
                        note.join(" · "),
                        selected_model.as_deref() == Some(model.id.as_str()),
                    ));
                }
                if models.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_2()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child("未发现模型定义；可在左下角「设置」中重新读取。"),
                    );
                }
                if let Some(note) = &sync_note {
                    list = list.child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(px(10.))
                            .line_height(relative(1.4))
                            .text_color(rgb(FAINT))
                            .child(note.clone()),
                    );
                }
                let refresh_view = view.clone();
                let refresh_popover = popover.clone();
                list.child(
                    h_flex()
                        .px_2()
                        .pt_2()
                        .gap_2()
                        .items_center()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(10.))
                                .text_color(rgb(FAINT))
                                .child("仅影响本会话，下一条消息生效"),
                        )
                        .child(
                            h_flex()
                                .id("model-refresh")
                                .cursor_pointer()
                                .rounded(px(4.))
                                .px_1()
                                .text_size(px(10.))
                                .text_color(rgb(MUTED))
                                .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                                .on_click(move |_, window, cx| {
                                    refresh_view.update(cx, |this, cx| {
                                        this.reload_models(provider, cx);
                                    });
                                    if let Some(state) = refresh_popover.clone().upgrade() {
                                        state.update(cx, |state, cx| state.dismiss(window, cx));
                                    }
                                })
                                .child("重新读取配置"),
                        ),
                )
            })
            .into_any_element()
    }
    fn reload_all_models_async(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Reading the CLI config files and the cc-switch SQLite database must
        // not block the first frame of the settings dialog (or the UI thread at
        // all); publish the result when the background read finishes.
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    (
                        models::load(Provider::Claude),
                        models::load(Provider::Codex),
                        ccs::load(),
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.catalogs.insert(Provider::Claude, loaded.0);
                this.catalogs.insert(Provider::Codex, loaded.1);
                this.ccs = loaded.2;
                cx.notify();
            });
        })
        .detach();
    }
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_all_models_async(window, cx);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            // Center the dialog vertically. The content follows the viewport
            // height (see settings_modal_content), so the same formula keeps
            // top and bottom margins equal and still fits the minimum window.
            let viewport_height = f32::from(window.viewport_size().height);
            let modal_height = (viewport_height - 96.).clamp(360., 560.);
            let margin_top = ((viewport_height - modal_height) * 0.5).max(24.);
            dialog
                .w(px(840.))
                .margin_top(px(margin_top))
                .p_0()
                .gap(px(0.))
                .rounded(px(16.))
                .bg(rgb(SURFACE))
                .border_color(rgb(BORDER))
                .close_button(true)
                .overlay_closable(true)
                .content(move |content, window, cx| {
                    let body = view.read(cx).settings_modal_content(window, view.clone());
                    content.child(body)
                })
        });
        cx.notify();
    }
    fn settings_provider_content(&self, provider: Provider, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let label = |text: &'static str| {
            div()
                .w(px(84.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(rgb(FAINT))
                .child(text)
        };
        let index: usize = match provider {
            Provider::Claude => 0,
            Provider::Codex => 1,
        };
        let configured = self
            .store
            .cli
            .path(provider)
            .map(display_path)
            .unwrap_or_else(|| "自动查找（环境变量 / PATH）".into());
        let effective = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => format!("{}：{}", cli.source.label(), display_path(&cli.path)),
            Err(error) => format!("尚不可用：{error:#}"),
        };
        let probing = self.probes.contains_key(&provider);
        let diagnostic = self
            .diagnostics
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| "尚未检测版本。".into());
        let choose_view = view.clone();
        let clear_view = view.clone();
        let mut actions = h_flex()
            .px_4()
            .py_2()
            .gap_2()
            .justify_end()
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                Button::new(("settings-clear", index))
                    .label("清除配置")
                    .small()
                    .ghost()
                    .disabled(disabled || self.store.cli.path(provider).is_none())
                    .when(
                        !disabled && self.store.cli.path(provider).is_some(),
                        |button| button.cursor_pointer(),
                    )
                    .on_click(move |_, _, cx| {
                        clear_view.update(cx, |this, cx| {
                            this.set_cli_path(provider, None, cx);
                        });
                    }),
            );
        if probing {
            let cancel_view = view.clone();
            actions = actions.child(
                Button::new(("settings-cancel-probe", index))
                    .label("取消检测")
                    .small()
                    .ghost()
                    .disabled(self.closing)
                    .when(!self.closing, |button| button.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        cancel_view.update(cx, |this, cx| {
                            if let Some(probe) = this.probes.get(&provider) {
                                probe.cancel();
                            }
                            this.diagnostics.insert(provider, "正在停止检测…".into());
                            cx.notify();
                        });
                    }),
            );
        } else {
            let probe_view = view.clone();
            actions = actions.child(
                Button::new(("settings-probe", index))
                    .label("检测版本")
                    .small()
                    .outline()
                    .disabled(self.closing)
                    .when(!self.closing, |button| button.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        probe_view.update(cx, |this, cx| {
                            this.probe_cli(provider, cx);
                        });
                    }),
            );
        }
        let card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE_STRONG))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("可执行文件"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(format!("已验证版本 {}", cli::tested_version(provider))),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_center()
                    .child(label("可执行文件"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(configured),
                    )
                    .child(
                        Button::new(("settings-choose", index))
                            .label("更改")
                            .small()
                            .outline()
                            .disabled(disabled)
                            .when(!disabled, |button| button.cursor_pointer())
                            .on_click(move |_, window, cx| {
                                choose_view.update(cx, |this, cx| {
                                    this.choose_cli(provider, window, cx);
                                });
                            }),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(label("本次启动"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(effective),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_start()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(label("状态"))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(FAINT))
                            .child(diagnostic),
                    ),
            )
            .child(actions);
        let catalog = self.catalogs.get(&provider);
        let current_model = catalog
            .and_then(|catalog| catalog.current.clone())
            .unwrap_or_else(|| "CLI 默认".into());
        let source = catalog
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let catalog_path = catalog
            .and_then(|catalog| catalog.catalog_path.as_deref())
            .map(display_path);
        let endpoint = catalog
            .and_then(|catalog| catalog.endpoint.clone())
            .unwrap_or_else(|| "CLI 内置".into());
        let detail = catalog.and_then(|catalog| catalog.detail.clone());
        let model_count = catalog.map(|catalog| catalog.models.len()).unwrap_or(0);
        let mut model_rows = v_flex().w_full();
        if let Some(catalog) = catalog {
            for model in &catalog.models {
                let is_current = catalog.current.as_deref() == Some(model.id.as_str());
                let show_id = model.label != model.id;
                let mut value = v_flex().flex_1().min_w_0().gap(px(1.)).child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT))
                        .child(model.label.clone()),
                );
                if show_id {
                    value = value.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(model.id.clone()),
                    );
                }
                model_rows = model_rows.child(
                    h_flex()
                        .px_4()
                        .py_3()
                        .gap_3()
                        .items_center()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .child(value)
                        .when(!model.note.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(FAINT))
                                    .child(model.note.clone()),
                            )
                        })
                        .when(is_current, |row| {
                            row.child(
                                div()
                                    .px_2()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(SURFACE_HOVER))
                                    .text_size(px(9.))
                                    .text_color(rgb(MUTED))
                                    .child("CLI 当前"),
                            )
                        }),
                );
            }
            if catalog.models.is_empty() {
                model_rows = model_rows.child(
                    h_flex()
                        .px_4()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(11.))
                        .text_color(rgb(FAINT))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
        }
        let sync_row = |label: &'static str, value: String, divider: bool| -> AnyElement {
            let mut row = h_flex().px_4().py_3().gap_4().items_center();
            if divider {
                row = row.border_t_1().border_color(rgb(BORDER));
            }
            row.child(settings_label(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(value),
                )
                .into_any_element()
        };
        let sync_view = view.clone();
        let mut model_card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE_STRONG))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("当前模型"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(SURFACE_HOVER))
                            .text_size(px(9.))
                            .text_color(rgb(FAINT))
                            .child("CLI"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(("settings-sync-models", index))
                            .label("重新读取")
                            .small()
                            .ghost()
                            .disabled(self.closing)
                            .when(!self.closing, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                sync_view.update(cx, |this, cx| {
                                    this.reload_models(provider, cx);
                                });
                            }),
                    ),
            )
            .child(sync_row("来源", source, false));
        if let Some(catalog_path) = catalog_path {
            model_card = model_card.child(sync_row("模型目录", catalog_path, true));
        }
        model_card = model_card
            .child(sync_row("当前默认", current_model, true))
            .child(sync_row("接口", endpoint, true));
        if let Some(detail) = detail {
            model_card = model_card.child(sync_row("附加配置", detail, true));
        }
        model_card = model_card
            .child(sync_row("可用模型", format!("{model_count} 个"), true))
            .child(model_rows);
        if let Some(catalog) = catalog {
            if let Some(error) = &catalog.error {
                model_card = model_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(10.))
                        .text_color(rgb(0xe6b4a8))
                        .child(error.clone()),
                );
            } else if let Some(note) = &catalog.note {
                model_card = model_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child(note.clone()),
                );
            }
        }
        v_flex()
            .w_full()
            .gap_3()
            .child(card)
            .child(model_card)
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(FAINT))
                    .child("版本检测只运行 --version，不调用模型，也不验证登录状态；请先在官方 CLI 中完成登录。"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(FAINT))
                    .child("此处只显示 CLI 当前模型；完整来源与 CCS 同步见左侧「模型同步」。不读取或显示密钥，也不会写回 CLI 配置。"),
            )
            .into_any_element()
    }
    fn settings_models_content(&self, view: Entity<Self>) -> AnyElement {
        let refresh_view = view.clone();
        let source_row = |label: &'static str, value: String, divider: bool| -> AnyElement {
            let mut row = h_flex().px_4().py_3().gap_4().items_center();
            if divider {
                row = row.border_t_1().border_color(rgb(BORDER));
            }
            row.child(settings_label(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(value),
                )
                .into_any_element()
        };
        let claude_path = self
            .catalogs
            .get(&Provider::Claude)
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let codex_path = self
            .catalogs
            .get(&Provider::Codex)
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let ccs_model_count: usize = self
            .ccs
            .providers
            .iter()
            .map(|entry| entry.models.len())
            .sum();
        let ccs_note = self.ccs.error.clone().or_else(|| self.ccs.note.clone());
        let mut source_card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE_STRONG))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("模型来源"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(SURFACE_HOVER))
                            .text_size(px(9.))
                            .text_color(rgb(FAINT))
                            .child("只读"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(format!(
                                "CCS {} 个 Provider · {} 个模型",
                                self.ccs.providers.len(),
                                ccs_model_count
                            )),
                    )
                    .child(
                        Button::new("settings-models-refresh")
                            .label("重新读取")
                            .small()
                            .ghost()
                            .disabled(self.closing)
                            .when(!self.closing, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |this, cx| {
                                    this.reload_all_models(cx);
                                });
                            }),
                    ),
            )
            .child(source_row("Claude Code", claude_path, false))
            .child(source_row("Codex", codex_path, true))
            .child(source_row(
                "cc-switch",
                display_path(&self.ccs.database_path),
                true,
            ));
        if let Some(note) = ccs_note {
            source_card = source_card.child(
                div()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(10.))
                    .text_color(rgb(if self.ccs.error.is_some() {
                        0xe6b4a8
                    } else {
                        FAINT
                    }))
                    .child(note),
            );
        }
        let mut page = v_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(FAINT))
                    .child("当前模型来自 CLI 正在使用的配置；CCS 模型来自 cc-switch 的 Provider 目录。只读取模型名与上下文信息，不查询凭证，也不修改任一配置。"),
            )
            .child(source_card);
        for provider in [Provider::Claude, Provider::Codex] {
            page = page.child(self.settings_models_provider(provider));
        }
        page.into_any_element()
    }
    fn settings_models_provider(&self, provider: Provider) -> AnyElement {
        let catalog = self.catalogs.get(&provider);
        let current = catalog
            .and_then(|catalog| catalog.current.clone())
            .unwrap_or_else(|| "CLI 默认".into());
        let config_path = catalog
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let model_count = catalog.map(|catalog| catalog.models.len()).unwrap_or(0);
        let badge = |text: &'static str| -> AnyElement {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgb(SURFACE_HOVER))
                .text_size(px(9.))
                .text_color(rgb(MUTED))
                .child(text)
                .into_any_element()
        };
        let model_row =
            |label: String, id: String, note: String, tag: Option<&'static str>| -> AnyElement {
                let mut value = v_flex().flex_1().min_w_0().gap(px(1.)).child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(TEXT))
                        .child(label.clone()),
                );
                if label != id {
                    value = value.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(id),
                    );
                }
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(value)
                    .when(!note.is_empty(), |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(rgb(FAINT))
                                .child(note),
                        )
                    })
                    .when_some(tag, |row, tag| row.child(badge(tag)))
                    .into_any_element()
            };
        let mut current_card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE_STRONG))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("当前 CLI 模型"),
                    )
                    .child(badge("CLI"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(format!("{model_count} 个")),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_center()
                    .child(settings_label("配置"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(config_path),
                    ),
            );
        if let Some(catalog) = catalog {
            for model in &catalog.models {
                current_card = current_card.child(model_row(
                    model.label.clone(),
                    model.id.clone(),
                    model.note.clone(),
                    (catalog.current.as_deref() == Some(model.id.as_str())).then_some("CLI 当前"),
                ));
            }
            if catalog.models.is_empty() {
                current_card = current_card.child(
                    div()
                        .px_4()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(11.))
                        .text_color(rgb(FAINT))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
            if let Some(error) = &catalog.error {
                current_card = current_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(10.))
                        .text_color(rgb(0xe6b4a8))
                        .child(error.clone()),
                );
            } else if let Some(note) = &catalog.note {
                current_card = current_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child(note.clone()),
                );
            }
        }
        let providers: Vec<_> = self.ccs.for_provider(provider).collect();
        let ccs_count: usize = providers.iter().map(|entry| entry.models.len()).sum();
        let mut ccs_card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(BORDER))
            .bg(rgb(SURFACE_STRONG))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("CCS 同步"),
                    )
                    .child(badge("只读"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(format!(
                                "{} 个 Provider · {ccs_count} 个模型",
                                providers.len()
                            )),
                    ),
            );
        for entry in &providers {
            ccs_card = ccs_card.child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child(entry.name.clone()),
                    )
                    .when(entry.current, |row| row.child(badge("当前 Provider")))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(format!("{} 个模型", entry.models.len())),
                    ),
            );
            for model in &entry.models {
                ccs_card = ccs_card.child(model_row(
                    model.label.clone(),
                    model.id.clone(),
                    model.note.clone(),
                    None,
                ));
            }
        }
        if providers.is_empty() {
            ccs_card = ccs_card.child(
                div()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(11.))
                    .text_color(rgb(FAINT))
                    .child(if provider == Provider::Claude {
                        "cc-switch 中没有可列出的 Claude 模型目录。"
                    } else {
                        "cc-switch 中没有可列出的 Codex 模型目录。"
                    }),
            );
        }
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child(provider.label()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child(format!("当前默认 {current}")),
                    ),
            )
            .child(current_card)
            .child(ccs_card)
            .into_any_element()
    }
    fn settings_git_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        v_flex()
            .w_full()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().font_semibold().text_size(px(13.)).child("Commit Message 提示词"))
                    .child(
                        Button::new("git-prompt-reset")
                            .label("恢复默认")
                            .small()
                            .ghost()
                            .disabled(disabled)
                            .when(!disabled, |button| button.cursor_pointer())
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.commit_message_prompt = crate::store::DEFAULT_COMMIT_PROMPT.into();
                                    this.git_prompt_input.update(cx, |input, cx| {
                                        input.set_value(crate::store::DEFAULT_COMMIT_PROMPT, window, cx);
                                    });
                                    this.persist();
                                    cx.notify();
                                });
                            }),
                    ),
            )
            .child(
                div().text_size(px(12.)).text_color(rgb(MUTED))
                    .child("设置生成说明的语言、格式和风格，应用于 Git 侧栏的 Generate 按钮。已暂存文件名与 diff 会自动附加，无需填写占位符。"),
            )
            .child(
                Textarea::new(&self.git_prompt_input)
                    .h(px(210.))
                    .w_full()
                    .text_size(px(12.))
                    .aria_label("Commit Message 生成提示词")
                    .disabled(disabled),
            )
            .child(
                div().text_size(px(11.)).text_color(rgb(FAINT))
                    .child("自动保存，适用于所有工作区；留空时使用默认提示词。修改将在下一次生成时生效。"),
            )
            .into_any_element()
    }
    fn settings_general_content(&self, view: Entity<Self>) -> AnyElement {
        let state_path = Store::path();
        let data_dir = state_path
            .parent()
            .map(display_path)
            .unwrap_or_else(|| "—".into());
        let state_file = display_path(&state_path);
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE_STRONG))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(TEXT))
                                    .child("本机数据"),
                            ),
                    )
                    .child(settings_row("数据目录", data_dir, false))
                    .child(settings_row("配置文件", state_file, true)),
            )
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE_STRONG))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(TEXT))
                                    .child("界面"),
                            ),
                    )
                    .child({
                        let value = self.store.ui.sessions_per_workspace.clamp(1, 50);
                        let decrease = view.clone();
                        let increase = view.clone();
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("工作区会话数"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(MUTED))
                                    .child("默认 5 条，超出时侧栏可展开"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        Button::new("sessions-limit-minus")
                                            .label("−")
                                            .small()
                                            .ghost()
                                            .disabled(value <= 1)
                                            .when(value > 1, |button| button.cursor_pointer())
                                            .on_click(move |_, _, cx| {
                                                decrease.update(cx, |this, cx| {
                                                    this.store.ui.sessions_per_workspace = this
                                                        .store
                                                        .ui
                                                        .sessions_per_workspace
                                                        .saturating_sub(1)
                                                        .max(1);
                                                    this.persist();
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        h_flex()
                                            .w(px(26.))
                                            .justify_center()
                                            .text_size(px(12.))
                                            .text_color(rgb(TEXT))
                                            .child(value.to_string()),
                                    )
                                    .child(
                                        Button::new("sessions-limit-plus")
                                            .label("+")
                                            .small()
                                            .ghost()
                                            .disabled(value >= 30)
                                            .when(value < 30, |button| button.cursor_pointer())
                                            .on_click(move |_, _, cx| {
                                                increase.update(cx, |this, cx| {
                                                    this.store.ui.sessions_per_workspace =
                                                        (this.store.ui.sessions_per_workspace + 1)
                                                            .min(30);
                                                    this.persist();
                                                    cx.notify();
                                                });
                                            }),
                                    ),
                            )
                    }),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(FAINT))
                    .child("配置只保存在本机，界面设置随本地状态保存。"),
            )
            .into_any_element()
    }
    fn settings_cli_content(&self) -> AnyElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE_STRONG))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(TEXT))
                                    .child("路径查找顺序"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(FAINT))
                                    .child("前者优先"),
                            ),
                    )
                    .child(settings_row(
                        "环境变量",
                        "CONDUIT_CLAUDE_PATH · CONDUIT_CODEX_PATH".into(),
                        false,
                    ))
                    .child(settings_row(
                        "应用配置",
                        "在本页 Claude Code / Codex 中手动指定".into(),
                        true,
                    ))
                    .child(settings_row("PATH", "claude.exe · codex.exe".into(), true))
                    .child(settings_row(
                        "常见安装位置",
                        "~/.local/bin · %LOCALAPPDATA%\\Programs\\OpenAI\\Codex".into(),
                        true,
                    )),
            )
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(SURFACE_STRONG))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(TEXT))
                                    .child("可执行文件与版本"),
                            ),
                    )
                    .child(settings_row(
                        "Claude Code",
                        "在左侧「Claude Code」页面选择本机 .exe 并检测版本".into(),
                        false,
                    ))
                    .child(settings_row(
                        "Codex",
                        "在左侧「Codex」页面选择本机 .exe 并检测版本".into(),
                        true,
                    )),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(FAINT))
                    .child("修改 CLI 路径或版本设置仅影响后续启动的会话；路径无效时会显式报错，不会自动改用另一份程序。"),
            )
            .into_any_element()
    }
    fn settings_modal_content(&self, window: &Window, view: Entity<Self>) -> AnyElement {
        let (title, subtitle) = match self.settings_tab {
            SettingsTab::General => ("通用", "本机数据位置与界面设置"),
            SettingsTab::Git => ("Git", "配置 Commit Message 的 AI 生成提示词"),
            SettingsTab::Cli => ("CLI", "可执行文件路径查找顺序与版本检测说明"),
            SettingsTab::Provider(provider) => (provider.label(), "管理可执行文件路径与版本检测"),
            SettingsTab::Models => ("模型同步", "查看 CLI 当前模型与 cc-switch 同步的模型"),
        };
        let nav_item = |tab: SettingsTab, label: &'static str, leading: AnyElement| -> AnyElement {
            let view = view.clone();
            let active = self.settings_tab == tab;
            h_flex()
                .id(match tab {
                    SettingsTab::General => "settings-tab-general",
                    SettingsTab::Git => "settings-tab-git",
                    SettingsTab::Cli => "settings-tab-cli",
                    SettingsTab::Provider(Provider::Claude) => "settings-tab-claude",
                    SettingsTab::Provider(Provider::Codex) => "settings-tab-codex",
                    SettingsTab::Models => "settings-tab-models",
                })
                .h(px(32.))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(rgb(if active { TEXT } else { MUTED }))
                .when(active, |row| row.bg(rgb(SURFACE_HOVER)))
                .when(!active, |row| {
                    row.hover(|style| style.bg(rgb(SURFACE_HOVER)).text_color(rgb(TEXT)))
                })
                .on_click(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.settings_tab = tab;
                        cx.notify();
                    });
                })
                .child(leading)
                .child(label)
                .into_any_element()
        };
        let section = |text: &'static str| {
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_size(px(10.))
                .text_color(rgb(FAINT))
                .child(text)
        };
        let general_icon = if self.settings_tab == SettingsTab::General {
            TEXT
        } else {
            FAINT
        };
        let general_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(IconName::Settings)
                    .small()
                    .text_color(rgb(general_icon)),
            )
            .into_any_element();
        let models_icon = if self.settings_tab == SettingsTab::Models {
            TEXT
        } else {
            FAINT
        };
        let models_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(IconName::Cpu)
                    .small()
                    .text_color(rgb(models_icon)),
            )
            .into_any_element();
        let cli_icon = if self.settings_tab == SettingsTab::Cli {
            TEXT
        } else {
            FAINT
        };
        let cli_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(gpui_kit::assets::IconName::Terminal)
                    .small()
                    .text_color(rgb(cli_icon)),
            )
            .into_any_element();
        let nav = v_flex()
            .w(px(208.))
            .h_full()
            .p_3()
            .gap_1()
            .bg(rgb(SIDEBAR_BG))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_3()
                    .text_size(px(16.))
                    .font_semibold()
                    .text_color(rgb(TEXT))
                    .child("设置"),
            )
            .child(section("应用"))
            .child(nav_item(SettingsTab::General, "通用", general_leading))
            .child(nav_item(
                SettingsTab::Git,
                "Git",
                Icon::new(gpui_kit::assets::IconName::GitFork)
                    .small()
                    .text_color(rgb(if self.settings_tab == SettingsTab::Git {
                        TEXT
                    } else {
                        FAINT
                    }))
                    .into_any_element(),
            ))
            .child(section("CLI"))
            .child(nav_item(SettingsTab::Cli, "CLI", cli_leading))
            .child(nav_item(
                SettingsTab::Provider(Provider::Claude),
                "Claude Code",
                provider_icon(Provider::Claude).into_any_element(),
            ))
            .child(nav_item(
                SettingsTab::Provider(Provider::Codex),
                "Codex",
                provider_icon(Provider::Codex).into_any_element(),
            ))
            .child(section("模型"))
            .child(nav_item(SettingsTab::Models, "模型同步", models_leading))
            .child(div().flex_1())
            .child(
                div()
                    .px_2()
                    .text_size(px(10.))
                    .text_color(rgb(FAINT))
                    .child("本地 CLI · Windows"),
            );
        let page = match self.settings_tab {
            SettingsTab::General => self.settings_general_content(view.clone()),
            SettingsTab::Git => self.settings_git_content(view.clone()),
            SettingsTab::Cli => self.settings_cli_content(),
            SettingsTab::Provider(provider) => {
                self.settings_provider_content(provider, view.clone())
            }
            SettingsTab::Models => self.settings_models_content(view.clone()),
        };
        let body = v_flex()
            .id("settings-modal-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .py_5()
            .gap_4()
            .when(!self.writable, |column| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(rgb(0x2a1d1a))
                        .text_size(px(11.))
                        .text_color(rgb(0xe6b4a8))
                        .child("当前数据目录为只读，无法保存设置或会话模型。"),
                )
            })
            .when_some(self.message.clone(), |column, message| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(rgb(0x2a1d1a))
                        .text_size(px(11.))
                        .text_color(rgb(0xe6b4a8))
                        .child(message),
                )
            })
            .child(page);
        let right = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(
                v_flex()
                    .px_6()
                    .pt_5()
                    .pb_4()
                    .pr(px(44.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(subtitle),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(BORDER)))
            .child(body)
            .child(
                h_flex()
                    .px_6()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(BORDER))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(FAINT))
                            .child("配置保存在本地，具体生效范围见各页面说明"),
                    )
                    .child(
                        Button::new("settings-done")
                            .label("完成")
                            .small()
                            .primary()
                            .cursor_pointer()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            );
        // Keep the footer reachable at the documented minimum window size
        // (850x500 client area). The dialog layer clips content taller than the
        // viewport minus its edge margins, so the height follows the window and
        // the body scrolls instead of losing the 完成 button.
        let modal_height = (f32::from(window.viewport_size().height) - 96.).clamp(360., 560.);
        h_flex()
            .w_full()
            .h(px(modal_height))
            .items_stretch()
            .overflow_hidden()
            .rounded(px(16.))
            .child(nav)
            .child(right)
            .into_any_element()
    }
    fn toggle_approval_detail(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.approval_details.remove(&id) {
            self.approval_details.insert(id);
        }
        cx.notify();
    }
    fn decide(
        &mut self,
        session: u64,
        request: String,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.closing
            && let Some(run) = self.runs.get_mut(&session)
            && !run.runtime.is_cancelled()
            && run
                .approvals
                .front()
                .is_some_and(|a| a.id == request && (!allow || a.allow_once))
            && run.approvals.begin(&request)
        {
            run.runtime.decide(request, allow);
            run.status = "正在提交审批…".into();
            self.input.read(cx).focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }
}

/// Whether a failure notice repeats text an earlier assistant/tool message
/// already shows. CLI wrappers often echo the same provider error in both the
/// assistant output and the task result, and showing it twice reads as a
/// duplicated reply.
fn same_failure(message: &str, error: &str) -> bool {
    let message = message.trim();
    let error = error.trim();
    if message.is_empty() || error.is_empty() {
        return false;
    }
    message == error || message.contains(error) || error.contains(message)
}

fn finish_tool_cards(session: &mut SessionRecord, prefix: &str, status: &str) {
    let prefix = format!("{prefix}-");
    for item in &mut session.messages {
        if item.kind == ItemKind::Tool
            && item.key.starts_with(&prefix)
            && matches!(item.status.as_str(), "运行中" | "输出中")
        {
            item.status = status.into();
        }
    }
}

pub fn apply_event(session: &mut SessionRecord, prefix: &str, event: Event) {
    if let Event::ToolInput { key, input } = event {
        let key = format!("{prefix}-{key}");
        if let Some(item) = session.messages.iter_mut().find(|m| m.key == key) {
            item.input = input;
        } else {
            session.messages.push(ChatItem {
                key,
                kind: ItemKind::Tool,
                text: "工具调用".into(),
                input,
                detail: String::new(),
                status: "运行中".into(),
            });
        }
        return;
    }
    let (key, kind, text, detail, status, delta) = match event {
        Event::Delta { key, text } => (
            key,
            ItemKind::Assistant,
            text,
            String::new(),
            String::new(),
            true,
        ),
        Event::Text { key, text } => (
            key,
            ItemKind::Assistant,
            text,
            String::new(),
            String::new(),
            false,
        ),
        Event::Tool {
            key,
            title,
            detail,
            status,
        } => {
            let delta = status == "输出中";
            (key, ItemKind::Tool, title, detail, status, delta)
        }
        Event::Notice(text) => (
            format!("notice-{}", session.messages.len()),
            ItemKind::Notice,
            text,
            String::new(),
            String::new(),
            false,
        ),
        _ => return,
    };
    let key = format!("{prefix}-{key}");
    if let Some(item) = session.messages.iter_mut().find(|m| m.key == key) {
        if kind == ItemKind::Assistant {
            if delta {
                item.text.push_str(&text);
            } else {
                item.text = text;
            }
        } else {
            if !text.is_empty() && !delta {
                item.text = text;
            }
            if delta {
                item.detail.push_str(&detail);
            } else if !detail.is_empty() {
                item.detail = detail;
            }
            item.status = status;
        }
    } else {
        // Guard against a provider resending the same completed block under a
        // new item key; keep one visible copy per consecutive run.
        let duplicate_text = kind == ItemKind::Assistant
            && !text.trim().is_empty()
            && session.messages.last().is_some_and(|last| {
                last.kind == ItemKind::Assistant && last.text.trim() == text.trim()
            });
        let duplicate_notice = kind == ItemKind::Notice
            && session
                .messages
                .last()
                .is_some_and(|last| last.kind == ItemKind::Notice && last.text == text);
        if !duplicate_text && !duplicate_notice {
            session.messages.push(ChatItem {
                key,
                kind,
                text,
                input: String::new(),
                detail,
                status,
            });
        }
    }
}

impl Render for ConduitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let narrow = window.viewport_size().width < px(1100.);
        let app_entity = cx.entity();
        let selected = self
            .selected_session
            .and_then(|id| self.session(id))
            .cloned();
        let busy = self
            .selected_workspace
            .is_some_and(|i| self.workspace_busy(i));
        let active = self
            .selected_session
            .is_some_and(|id| self.runs.contains_key(&id));
        let has_workspace = self.selected_workspace.is_some();
        let status = self
            .selected_session
            .and_then(|id| self.runs.get(&id))
            .map(|r| r.status.clone());
        let directory = self
            .selected_workspace
            .map(|i| self.store.workspaces[i].name())
            .unwrap_or_else(|| "未选择工作区".into());
        let can_edit = self.writable && !self.closing;
        let mut sidebar = v_flex()
            .id("sidebar")
            .h_full()
            .w_full()
            .bg(rgb(SIDEBAR_BG))
            .p_2()
            .gap_1()
            .child(
                h_flex()
                    .h(px(36.))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("sidebar-new-session")
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Plus)
                            .tooltip(format!("新建 {} 会话", self.new_session_provider.label()))
                            .disabled(!can_edit || self.selected_workspace.is_none())
                            .when(can_edit && self.selected_workspace.is_some(), |button| {
                                button.cursor_pointer()
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(index) = this.selected_workspace {
                                    this.new_session_here(index, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("sidebar-search")
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Search)
                            .tooltip("搜索会话")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_search_dialog(window, cx)
                            })),
                    )
                    .child(div().flex_1()),
            );
        let mut list = v_flex()
            .id("workspace-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt_3()
            .gap_1();
        let query = self.search_query.trim().to_lowercase();
        list = list.child(
            h_flex()
                .px_2()
                .pb_1()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(FAINT))
                        .child("工作区"),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .id("toggle-archived")
                                .h(px(22.))
                                .px_2()
                                .gap_1()
                                .items_center()
                                .rounded(px(6.))
                                .text_size(px(10.))
                                .text_color(rgb(if self.show_archived { TEXT } else { FAINT }))
                                .when(self.show_archived, |row| row.bg(rgb(SURFACE_HOVER)))
                                .cursor_pointer()
                                .hover(|style| style.bg(rgb(SURFACE_HOVER)).text_color(rgb(TEXT)))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.show_archived = !this.show_archived;
                                    cx.notify();
                                }))
                                .child(Icon::new(IconName::Inbox).small())
                                .child("已归档"),
                        )
                        .child(
                            h_flex()
                                .id("add-workspace")
                                .w(px(22.))
                                .h(px(22.))
                                .items_center()
                                .justify_center()
                                .rounded(px(6.))
                                .when(can_edit, |button| {
                                    button
                                        .cursor_pointer()
                                        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                                        .on_click(cx.listener(Self::add_workspace))
                                })
                                .child(
                                    Icon::new(IconName::Plus)
                                        .small()
                                        .text_color(rgb(if can_edit { MUTED } else { FAINT })),
                                ),
                        ),
                ),
        );
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            let sessions: Vec<&SessionRecord> = workspace
                .sessions
                .iter()
                .rev()
                .filter(|session| {
                    (self.show_archived || !session.archived) && session_matches(session, &query)
                })
                .collect();
            if !query.is_empty() && sessions.is_empty() {
                continue;
            }
            let last = sessions
                .first()
                .map(|session| session.id)
                .or_else(|| workspace.sessions.last().map(|session| session.id));
            let workspace_selected = self.selected_workspace == Some(index);
            let count = sessions.len();
            let limit = self.store.ui.sessions_per_workspace.clamp(1, 50);
            let expanded = self.expanded_workspaces.contains(&index);
            let total = sessions.len();
            list =
                list.child(
                    h_flex()
                        .id(("workspace", index))
                        .h(px(30.))
                        .px_2()
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .text_size(px(13.))
                        .text_color(rgb(if workspace_selected { TEXT } else { MUTED }))
                        .when(workspace_selected, |row| row.bg(rgb(SURFACE)))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch(index, last, window, cx)
                        }))
                        .child(Icon::new(IconName::Folder).small().text_color(rgb(FAINT)))
                        .child(div().flex_1().truncate().child(workspace.name()))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(FAINT))
                                .child(count.to_string()),
                        )
                        .child(
                            Button::new(("workspace-new-session", index))
                                .ghost()
                                .small()
                                .compact()
                                .icon(IconName::Plus)
                                .tooltip("在此工作区新建会话")
                                .disabled(!can_edit)
                                .when(can_edit, |button| button.cursor_pointer())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    // Keep the workspace row's own click
                                    // handler from reselecting the previous
                                    // session right after creation.
                                    cx.stop_propagation();
                                    this.new_session_here(index, window, cx)
                                })),
                        )
                        .child(self.workspace_menu(index, app_entity.clone())),
                );
            for session in sessions.iter().take(if expanded { total } else { limit }) {
                let id = session.id;
                let active = self.selected_session == Some(id);
                let run_status = self.runs.get(&id).map(|r| r.status.clone());
                let archived = session.archived;
                let menu = self.session_menu(id, app_entity.clone());
                list = list.child(
                    h_flex()
                        .id(("session", id))
                        .h(px(30.))
                        .pl(px(24.))
                        .pr(px(8.))
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .text_size(px(13.))
                        .text_color(rgb(if archived {
                            FAINT
                        } else if active {
                            TEXT
                        } else {
                            MUTED
                        }))
                        .when(active, |row| row.bg(rgb(SURFACE_HOVER)))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(SURFACE_HOVER)).text_color(rgb(TEXT)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch(index, Some(id), window, cx)
                        }))
                        .child(provider_icon(session.provider))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .child(if session.title.is_empty() {
                                    "新会话".into()
                                } else {
                                    session.title.clone()
                                }),
                        )
                        .when(archived, |row| {
                            row.child(
                                div()
                                    .text_size(px(9.))
                                    .text_color(rgb(FAINT))
                                    .child("已归档"),
                            )
                        })
                        .when_some(run_status, |row, status| {
                            row.child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(FAINT))
                                    .child(status),
                            )
                        })
                        .child(menu),
                );
            }
            if total > limit {
                let toggle_app = app_entity.clone();
                let more_label = if expanded {
                    "收起".to_string()
                } else {
                    format!("显示更多（还有 {} 条）", total - limit)
                };
                list = list.child(
                    h_flex()
                        .id(("workspace-more", index))
                        .h(px(24.))
                        .pl(px(24.))
                        .pr(px(4.))
                        .items_center()
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                        .on_click(move |_, _, cx| {
                            toggle_app.update(cx, |this, cx| {
                                if !this.expanded_workspaces.insert(index) {
                                    this.expanded_workspaces.remove(&index);
                                }
                                cx.notify();
                            });
                        })
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(FAINT))
                                .child(more_label),
                        ),
                );
            }
        }
        if self.store.workspaces.is_empty() {
            list = list.child(
                v_flex()
                    .mt_2()
                    .px_2()
                    .py_3()
                    .gap_1()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::new(gpui_kit::assets::IconName::FolderPlus)
                                    .small()
                                    .text_color(rgb(FAINT)),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(MUTED))
                                    .child("还没有工作区"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(FAINT))
                            .child("点击上方 + 添加一个本地目录"),
                    ),
            );
        }
        sidebar = sidebar.child(list).child(
            v_flex()
                .gap_1()
                .pt_2()
                .border_t_1()
                .border_color(rgb(BORDER))
                .child(
                    h_flex()
                        .id("settings")
                        .h(px(32.))
                        .px_2()
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .text_size(px(13.))
                        .text_color(rgb(MUTED))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(SURFACE_HOVER)).text_color(rgb(TEXT)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_settings(window, cx);
                        }))
                        .child(Icon::new(IconName::Settings).small().text_color(rgb(FAINT)))
                        .child("设置"),
                )
                .child(
                    div()
                        .px_2()
                        .text_size(px(10.))
                        .text_color(rgb(FAINT))
                        .child("本地 CLI · Windows · Preview"),
                ),
        );
        let mut messages = v_flex()
            .id("messages")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px_6()
            .py_5()
            .gap_5();
        if let Some(session) = &selected {
            if session.messages.is_empty() {
                messages = messages.child(
                    v_flex()
                        .flex_1()
                        .justify_center()
                        .items_center()
                        .gap_3()
                        .py_20()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(provider_icon(session.provider))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(FAINT))
                                        .child(format!("{} · 已就绪", session.provider.label())),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(30.))
                                .font_bold()
                                .text_color(rgb(TEXT))
                                .child("今天想完成什么？"),
                        )
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(rgb(MUTED))
                                .child(format!("在 {} 中开始你的任务", directory)),
                        ),
                );
            }
            for (index, item) in session.messages.iter().enumerate() {
                let element = match item.kind {
                    ItemKind::User => h_flex()
                        .w_full()
                        .justify_end()
                        .child(
                            div()
                                .max_w(px(680.))
                                .rounded(px(18.))
                                .px_4()
                                .py_3()
                                .bg(rgb(SURFACE))
                                .text_size(px(14.))
                                .text_color(rgb(TEXT))
                                .child(item.text.clone()),
                        )
                        .into_any_element(),
                    ItemKind::Assistant => v_flex()
                        .gap_2()
                        .w_full()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(provider_icon(session.provider))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(rgb(FAINT))
                                        .child(session.provider.label()),
                                ),
                        )
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("reply-{}-{}", session.id, item.key)),
                                item.text.clone(),
                            )
                            .text_size(px(15.))
                            .selectable(true),
                        )
                        .into_any_element(),
                    ItemKind::Tool => {
                        let detail = item.key.clone();
                        div()
                            .id(("tool", index))
                            .px_3()
                            .py_2()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(SURFACE))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.details = true;
                                this.detail_view = DetailView::Tool;
                                this.detail_text = Some(detail.clone());
                                cx.notify();
                            }))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(13.))
                                            .truncate()
                                            .child(item.text.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(FAINT))
                                            .child(item.status.clone()),
                                    )
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .small()
                                            .text_color(rgb(FAINT)),
                                    ),
                            )
                            .into_any_element()
                    }
                    ItemKind::Notice => div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(item.text.clone())
                        .into_any_element(),
                };
                messages = messages.child(div().w_full().max_w(px(840.)).mx_auto().child(element));
            }
        } else if has_workspace {
            messages = messages.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .py_20()
                    .child(
                        h_flex()
                            .w(px(44.))
                            .h(px(44.))
                            .items_center()
                            .justify_center()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(SURFACE))
                            .child(
                                div()
                                    .text_base()
                                    .font_bold()
                                    .text_color(rgb(MUTED))
                                    .child("C"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("今天想完成什么？"),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "会话将在 {directory} 中运行，输入任务后会按上次使用的引擎新建会话。"
                            )),
                    ),
            );
        } else {
            messages = messages.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .py_20()
                    .child(
                        h_flex()
                            .w(px(44.))
                            .h(px(44.))
                            .items_center()
                            .justify_center()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(rgb(BORDER))
                            .bg(rgb(SURFACE))
                            .child(
                                div()
                                    .text_base()
                                    .font_bold()
                                    .text_color(rgb(MUTED))
                                    .child("C"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("连接一个本地工作区"),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .line_height(relative(1.5))
                            .text_color(rgb(MUTED))
                            .child("Conduit 直接调用本机 CLI，文件留在你的机器上。"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(FAINT))
                            .child("CLI 只会在所选目录内执行任务"),
                    ),
            );
        }
        let mut conversation = v_flex().flex_1().min_w_0().h_full();
        let current_provider = selected.as_ref().map(|session| session.provider);
        let header_title = match &selected {
            Some(session) if !session.title.is_empty() => session.title.clone(),
            Some(_) => "新会话".to_string(),
            None if has_workspace => "新会话".to_string(),
            None => "未选择工作区".to_string(),
        };
        conversation = conversation.child(
            h_flex()
                .h(px(56.))
                .px_5()
                .items_center()
                .gap_3()
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(self.workspace_picker(app_entity.clone()))
                .when_some(current_provider, |header, provider| {
                    header.child(provider_icon(provider))
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .font_bold()
                        .text_color(rgb(TEXT))
                        .child(header_title),
                )
                .when(active, |header| {
                    header.child(
                        div()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(rgb(SURFACE))
                            .text_size(px(10.))
                            .text_color(rgb(MUTED))
                            .child(status.clone().unwrap_or_else(|| "运行中".into())),
                    )
                })
                .child(
                    Button::new("toggle-sidebar")
                        .ghost()
                        .small()
                        .compact()
                        .icon(gpui_kit::assets::IconName::PanelRight)
                        .cursor_pointer()
                        .tooltip(if self.details && self.detail_view == DetailView::Tool {
                            "收起工具详情"
                        } else {
                            "打开工具详情"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.details = !(this.details && this.detail_view == DetailView::Tool);
                            this.detail_view = DetailView::Tool;
                            cx.notify();
                        })),
                ),
        );
        conversation = conversation.child(messages);
        let approval_count = self
            .selected_session
            .and_then(|id| self.runs.get(&id))
            .map(|r| r.approvals.len())
            .unwrap_or(0);
        let approvals = self
            .selected_session
            .and_then(|id| self.runs.get(&id))
            .and_then(|r| r.approvals.front().cloned());
        for (index, approval) in approvals.into_iter().enumerate() {
            let submitted = self
                .selected_session
                .and_then(|id| self.runs.get(&id))
                .is_some_and(|r| r.approvals.is_submitted(&approval.id));
            let id = self.selected_session.unwrap();
            let allow_id = approval.id.clone();
            let deny_id = approval.id;
            let toggle_id = deny_id.clone();
            let expanded = self.approval_details.contains(&deny_id);
            let queue_label = if approval_count > 1 {
                format!("第 {} / {} 项", index + 1, approval_count)
            } else {
                String::new()
            };
            conversation = conversation.child(
                div().w_full().px_6().mb_4().child(
                    v_flex()
                        .w_full()
                        .max_w(px(840.))
                        .mx_auto()
                        .p_4()
                        .gap_3()
                        .rounded(px(14.))
                        .border_1()
                        .border_color(rgb(0x4a3b22))
                        .bg(rgb(0x1d1a14))
                        .child(
                            h_flex()
                                .items_start()
                                .gap_3()
                                .child(
                                    h_flex()
                                        .w(px(28.))
                                        .h(px(28.))
                                        .flex_shrink_0()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(8.))
                                        .bg(rgb(0x2a2317))
                                        .child(
                                            Icon::new(IconName::TriangleAlert)
                                                .small()
                                                .text_color(rgb(0xe3b96b)),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .text_size(px(13.))
                                                        .font_semibold()
                                                        .text_color(rgb(0xf0d49b))
                                                        .child(approval.title.clone()),
                                                )
                                                .when(!queue_label.is_empty(), |row| {
                                                    row.child(
                                                        h_flex()
                                                            .h(px(18.))
                                                            .px_2()
                                                            .items_center()
                                                            .rounded_full()
                                                            .bg(rgb(0x332b1b))
                                                            .text_size(px(10.))
                                                            .text_color(rgb(0xd6b26b))
                                                            .child(queue_label.clone()),
                                                    )
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .line_height(relative(1.4))
                                                .text_color(rgb(0x9c9284))
                                                .child("CLI 请求授权，仅对本次操作生效。"),
                                        ),
                                )
                                .child(
                                    Button::new(("approval-toggle", index))
                                        .label(if expanded { "收起" } else { "展开详情" })
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.toggle_approval_detail(toggle_id.clone(), cx)
                                        })),
                                ),
                        )
                        .when(expanded, |column| {
                            column.child(
                                div()
                                    .id(("approval-detail", index))
                                    .max_h(px((f32::from(window.viewport_size().height) * 0.4)
                                        .clamp(120., 360.)))
                                    .overflow_y_scroll()
                                    .p_3()
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(rgb(0x352c1b))
                                    .bg(rgb(0x15130e))
                                    .text_xs()
                                    .line_height(relative(1.5))
                                    .text_color(rgb(0xd9d3c7))
                                    .child(approval.detail.clone()),
                            )
                        })
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .when(!approval.allow_once, |row| {
                                    row.child(
                                        div()
                                            .flex_1()
                                            .text_size(px(11.))
                                            .text_color(rgb(0xd6a75a))
                                            .child("CLI 未提供「允许本次」选项，此请求只能拒绝。"),
                                    )
                                })
                                .when(approval.allow_once, |row| row.child(div().flex_1()))
                                .child(
                                    Button::new(("deny", index))
                                        .label("拒绝")
                                        .small()
                                        .ghost()
                                        .disabled(self.closing || submitted)
                                        .when(!self.closing && !submitted, |button| {
                                            button.cursor_pointer()
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.decide(id, deny_id.clone(), false, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new(("allow", index))
                                        .label("允许本次")
                                        .small()
                                        .primary()
                                        .disabled(self.closing || submitted || !approval.allow_once)
                                        .when(
                                            !self.closing && !submitted && approval.allow_once,
                                            |button| button.cursor_pointer(),
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.decide(id, allow_id.clone(), true, window, cx)
                                        })),
                                ),
                        ),
                ),
            );
        }
        if let Some(message) = &self.message {
            conversation = conversation.child(
                div()
                    .mx_6()
                    .mb_3()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x5c332c))
                    .bg(rgb(0x2a1d1a))
                    .text_sm()
                    .text_color(rgb(0xe6b4a8))
                    .child(message.clone()),
            );
        }
        let input_focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let provider = selected.as_ref().map(|session| session.provider);
        let send_ready = can_edit && !busy && has_workspace;
        // The floating pin replaces the old "最新消息" text button: it appears
        // only when the user has scrolled away from the end of the message list.
        let show_latest_pin = !self.at_bottom
            && selected
                .as_ref()
                .is_some_and(|session| !session.messages.is_empty());
        let session_id = selected
            .as_ref()
            .map(|session| session.id)
            .unwrap_or_default();
        let selected_model = selected.as_ref().and_then(|session| session.model.clone());
        let model_picker = provider
            .map(|provider| {
                self.composer_model_picker(
                    cx.entity(),
                    session_id,
                    provider,
                    selected_model,
                    !can_edit,
                )
            })
            .unwrap_or_else(|| div().into_any_element());
        // One composer for every state: an active session, a workspace without
        // a session (the first send creates one with the last engine), and no
        // workspace at all. Keeping the same shell in all three states keeps
        // the chat area from changing size when the session state changes.
        let composer_enabled = can_edit && has_workspace;
        let composer: AnyElement = v_flex()
            .id("composer")
            .key_context("ConduitComposer")
            .on_action(cx.listener(|this, _: &SendPrompt, window, cx| {
                cx.stop_propagation();
                this.send_prompt(window, cx)
            }))
            .on_action(cx.listener(|this, _: &Newline, window, cx| {
                cx.stop_propagation();
                this.input
                    .update(cx, |input, cx| input.insert("\n", window, cx));
                cx.notify();
            }))
            .w_full()
            .max_w(px(840.))
            .mx_auto()
            .p_2()
            .gap_1()
            .rounded(px(22.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(if input_focused {
                rgb(FOCUS)
            } else {
                rgb(BORDER)
            })
            .on_click(cx.listener(|this, event, window, cx| {
                if this.selected_workspace.is_some() {
                    this.input.read(cx).focus_handle(cx).focus(window, cx);
                } else {
                    this.add_workspace(event, window, cx);
                }
            }))
            .child(
                h_flex()
                    .h(px(32.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(10.))
                    .bg(rgb(SURFACE_STRONG))
                    .child(Icon::new(IconName::Folder).small().text_color(rgb(FAINT)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(MUTED))
                            .child(directory.clone()),
                    )
                    .when(selected.is_some(), |row| row.child(model_picker)),
            )
            .child(
                div().px_2().pt_2().child(
                    Textarea::new(&self.input)
                        .appearance(false)
                        .bordered(false)
                        .text_size(px(14.))
                        .disabled(self.closing || !composer_enabled),
                ),
            )
            .child(
                h_flex()
                    .px_2()
                    .pb_1()
                    .pt_1()
                    .gap_2()
                    .items_center()
                    .child(self.engine_picker(app_entity.clone()))
                    .child(div().flex_1())
                    .child(if active {
                        h_flex()
                            .id("stop")
                            .w(px(30.))
                            .h(px(30.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(TEXT))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0xffffff)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(run) =
                                    this.selected_session.and_then(|id| this.runs.get_mut(&id))
                                {
                                    run.runtime.cancel();
                                    run.approvals.clear();
                                    run.status = "正在停止…".into();
                                }
                                this.input.read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            }))
                            .child(div().w(px(8.)).h(px(8.)).rounded(px(2.)).bg(rgb(0x111113)))
                            .into_any_element()
                    } else {
                        h_flex()
                            .id("send")
                            .w(px(30.))
                            .h(px(30.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(if send_ready { TEXT } else { SURFACE_STRONG }))
                            .when(send_ready, |button| {
                                button
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(0xffffff)))
                                    .on_click(cx.listener(Self::send_message))
                            })
                            .child(
                                Icon::new(IconName::ArrowUp)
                                    .small()
                                    .text_color(rgb(if send_ready { 0x111113 } else { FAINT })),
                            )
                            .into_any_element()
                    }),
            )
            .into_any_element();
        // The "jump to latest" pin floats at the lower-right of the message
        // area, just above the composer's top-right corner, and only exists
        // while the list is scrolled away from its end.
        conversation = conversation.child(
            div().px_6().pt_3().pb_2().child(
                div()
                    .relative()
                    .w_full()
                    .max_w(px(840.))
                    .mx_auto()
                    .child(composer)
                    .when(show_latest_pin, |area| {
                        area.child(
                            h_flex()
                                .id("scroll-to-latest")
                                .absolute()
                                .top(px(-40.))
                                .right(px(14.))
                                .w(px(30.))
                                .h(px(30.))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(SURFACE_STRONG))
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(rgb(SURFACE_HOVER))
                                        .border_color(rgb(BORDER_STRONG))
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.scroll_to_end();
                                    this.at_bottom = true;
                                    cx.notify();
                                }))
                                .child(
                                    Icon::new(gpui_kit::assets::IconName::Pin)
                                        .small()
                                        .text_color(rgb(MUTED)),
                                ),
                        )
                    }),
            ),
        );
        // The toolbar belongs to the conversation; workspace navigation and
        // the independent right dock use their full height.
        let toolbar_hint: String = if self.closing {
            "正在清理进程…".into()
        } else if active {
            status.clone().unwrap_or_else(|| "正在执行…".into())
        } else {
            "本地 CLI · 文件保留在你的机器上".into()
        };
        let toolbar = h_flex()
            .h(px(34.))
            .flex_shrink_0()
            .px_6()
            .gap_3()
            .items_center()
            .border_t_1()
            .border_color(rgb(BORDER))
            .when(self.git_repository.is_some(), |bar| {
                bar.child(self.branch_picker(app_entity.clone()))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(rgb(FAINT))
                    .child(toolbar_hint),
            )
            .child(div().flex_1())
            .child(
                Button::new("toggle-git")
                    .ghost()
                    .small()
                    .compact()
                    .icon(gpui_kit::assets::IconName::GitFork)
                    .cursor_pointer()
                    .tooltip(if self.details && self.detail_view == DetailView::Git {
                        "收起 Git 变更"
                    } else {
                        "打开 Git 变更"
                    })
                    .when(
                        self.details && self.detail_view == DetailView::Git,
                        |button| button.bg(rgb(SURFACE_HOVER)).text_color(rgb(TEXT)),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.details = !(this.details && this.detail_view == DetailView::Git);
                        this.detail_view = DetailView::Git;
                        if this.details {
                            this.refresh_git(cx);
                        }
                        cx.notify();
                    })),
            )
            .when_some(self.push_button(app_entity.clone()), |bar, button| {
                bar.child(button)
            });
        let title_bar = TitleBar::new()
            .bg(rgb(CANVAS))
            .border_color(rgb(BORDER))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        h_flex()
                            .w(px(20.))
                            .h(px(20.))
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .bg(rgb(TEXT))
                            .text_size(px(11.))
                            .font_bold()
                            .text_color(rgb(0x111113))
                            .child("C"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(TEXT))
                            .child("Conduit"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(FAINT))
                            .child("Chat M0 · Preview"),
                    ),
            );
        conversation = conversation.child(toolbar);
        let mut detail_panel: Option<AnyElement> = None;
        let mut overlay: Option<AnyElement> = None;
        if self.details {
            let native_id = selected
                .as_ref()
                .and_then(|s| s.native_id.clone())
                .unwrap_or_else(|| "首次回复后绑定".into());
            let path = self
                .selected_workspace
                .map(|i| display_path(&self.store.workspaces[i].path))
                .unwrap_or_default();
            let tool_content = self
                .detail_text
                .as_ref()
                .and_then(|key| selected.as_ref()?.messages.iter().find(|m| &m.key == key))
                .map(|item| {
                    format!(
                        "输入\n{}\n\n输出 / 结果\n{}",
                        if item.input.is_empty() {
                            "未提供"
                        } else {
                            &item.input
                        },
                        if item.detail.is_empty() {
                            "尚未返回"
                        } else {
                            &item.detail
                        }
                    )
                })
                .unwrap_or_else(|| "点击工具调用卡片查看输入、命令输出或文件变更。".into());
            let tool_body: AnyElement = v_flex()
                .flex_1()
                .min_h_0()
                .gap_3()
                .child(div().text_xs().text_color(rgb(MUTED)).child("工作目录"))
                .child(div().text_xs().text_color(rgb(TEXT)).child(path))
                .child(div().text_xs().text_color(rgb(MUTED)).child("原生会话 ID"))
                .child(div().text_xs().child(native_id))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .id("detail-content")
                        .overflow_y_scroll()
                        .text_xs()
                        .child(tool_content),
                )
                .into_any_element();
            let body = match self.detail_view {
                DetailView::Tool => tool_body,
                DetailView::Git => v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(self.git_panel(app_entity.clone()))
                    .child(self.commit_editor(app_entity.clone(), cx))
                    .into_any_element(),
            };
            let panel = v_flex()
                .h_full()
                .w_full()
                .flex_shrink_0()
                .bg(rgb(SIDEBAR_BG))
                .p_3()
                .gap_3()
                .when(narrow, |panel| {
                    panel
                        .w(px(300.))
                        .absolute()
                        .top_0()
                        .right_0()
                        .h_auto()
                        .bottom(px(34.))
                        .border_l_1()
                        .border_color(rgb(BORDER))
                })
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(if self.detail_view == DetailView::Git {
                                gpui_kit::assets::IconName::GitFork
                            } else {
                                gpui_kit::assets::IconName::PanelRight
                            })
                            .small()
                            .text_color(rgb(MUTED)),
                        )
                        .child(div().text_size(px(12.)).font_semibold().child(
                            if self.detail_view == DetailView::Git {
                                "Git 变更"
                            } else {
                                "工具详情"
                            },
                        ))
                        .child(div().flex_1())
                        .child(
                            Button::new("close-details")
                                .ghost()
                                .small()
                                .compact()
                                .icon(gpui_kit::assets::IconName::X)
                                .cursor_pointer()
                                .tooltip("关闭侧边栏")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(body);
            if narrow {
                overlay = Some(panel.into_any_element());
            } else {
                detail_panel = Some(panel.into_any_element());
            }
        }
        // The left workspace rail stays a resizable group. The right sidebar is
        // laid out by hand so dragging its edge follows the pointer 1:1 and its
        // width can be capped at half of the remaining content area.
        let viewport_width = f32::from(window.viewport_size().width);
        let sidebar_state =
            window.use_keyed_state("body-resizable", cx, |_, _| ResizableState::default());
        let measured_sidebar = sidebar_state
            .read(cx)
            .sizes()
            .first()
            .map(|width| f32::from(*width))
            .filter(|width| *width > 0.)
            .unwrap_or(248.);
        self.sidebar_width = measured_sidebar;
        let content_width = (viewport_width - measured_sidebar - 6.).max(320.);
        let max_detail_width = (content_width * 0.5).max(260.);
        self.detail_width = self.detail_width.clamp(260., max_detail_width);
        let detail_width = self.detail_width;

        let mut body = h_resizable("body").with_state(&sidebar_state).child(
            resizable_panel()
                .size(px(248.))
                .size_range(px(200.)..px(420.))
                .flex_none()
                .child(sidebar),
        );
        let conversation_area: AnyElement = match detail_panel {
            Some(panel) => {
                let drag_app = app_entity.clone();
                let band = div()
                    .id("detail-resize")
                    .relative()
                    .w(px(6.))
                    .h_full()
                    .flex_shrink_0()
                    .cursor_col_resize()
                    .hover(|style| style.bg(rgb(SURFACE_HOVER)))
                    .on_drag(DetailResize, move |_, position, window, cx| {
                        // `position` is relative to the band; the panel's right
                        // edge is the window edge, so recover the pointer's
                        // window x to keep the grab offset constant while dragging.
                        let viewport = f32::from(window.viewport_size().width);
                        let local = f32::from(position.x);
                        drag_app.update(cx, |this, cx| {
                            let start_x = viewport - this.detail_width - 6. + local;
                            this.detail_drag = Some((start_x, this.detail_width));
                            cx.notify();
                        });
                        cx.new(|_| DetailResize)
                    });
                h_flex()
                    .h_full()
                    .w_full()
                    .child(conversation)
                    .child(band)
                    .child(
                        div()
                            .w(px(detail_width))
                            .h_full()
                            .flex_shrink_0()
                            .child(panel),
                    )
                    .into_any_element()
            }
            None => conversation.into_any_element(),
        };
        body = body.child(resizable_panel().child(conversation_area));

        let drag_app = app_entity.clone();
        let mut main_area = div()
            .relative()
            .flex_1()
            .min_h_0()
            .on_drag_move(move |event: &DragMoveEvent<DetailResize>, window, cx| {
                let x = f32::from(event.event.position.x);
                let viewport = f32::from(window.viewport_size().width);
                drag_app.update(cx, |this, cx| {
                    let Some((start_x, start_width)) = this.detail_drag else {
                        return;
                    };
                    let content = (viewport - this.sidebar_width - 6.).max(320.);
                    let max = (content * 0.5).max(260.);
                    let width = (start_width + (start_x - x)).clamp(260., max);
                    if (width - this.detail_width).abs() > 0.5 {
                        this.detail_width = width;
                        cx.notify();
                    }
                });
            })
            .child(body);
        if let Some(panel) = overlay {
            main_area = main_area.child(panel);
        }
        v_flex()
            .relative()
            .size_full()
            .bg(rgb(CANVAS))
            .text_color(rgb(TEXT))
            .child(title_bar)
            .child(main_area)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_event, finish_tool_cards, highlight_ranges, same_failure, search_snippet,
        session_matches,
    };
    use crate::{
        domain::{ChatItem, ItemKind, Provider, SessionRecord},
        protocol::Event,
    };
    #[test]
    fn commit_generation_snapshot_replaces_stream_without_duplicate_text() {
        let mut parts = vec![];
        super::update_commit_text(&mut parts, "msg".into(), "feat: ".into(), false);
        super::update_commit_text(&mut parts, "msg".into(), "说明".into(), false);
        super::update_commit_text(&mut parts, "msg".into(), "feat: 说明".into(), true);
        super::update_commit_text(&mut parts, "body".into(), "原因".into(), true);
        assert_eq!(
            parts,
            vec![
                ("msg".into(), "feat: 说明".into()),
                ("body".into(), "原因".into())
            ]
        );
    }
    fn session() -> SessionRecord {
        SessionRecord {
            id: 1,
            provider: Provider::Claude,
            created_at: 0,
            exit_code: None,
            title: String::new(),
            archived: false,
            native_id: None,
            model: None,
            messages: vec![],
            interrupted: false,
        }
    }
    #[test]
    fn final_snapshot_replaces_stream_and_turns_keep_distinct_identity() {
        let mut s = session();
        for part in ["Hello", " world"] {
            apply_event(
                &mut s,
                "turn-0",
                Event::Delta {
                    key: "m1".into(),
                    text: part.into(),
                },
            );
        }
        apply_event(
            &mut s,
            "turn-0",
            Event::Text {
                key: "m1".into(),
                text: "Hello world".into(),
            },
        );
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].text, "Hello world");
        apply_event(
            &mut s,
            "turn-1",
            Event::Text {
                key: "m1".into(),
                text: "Second turn".into(),
            },
        );
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].text, "Hello world");
    }
    #[test]
    fn command_output_updates_one_card_and_completion_replaces_partial_output() {
        let mut s = session();
        for (title, detail, status) in [
            ("cargo check", "", "运行中"),
            ("命令输出", "part", "输出中"),
            ("命令输出", "ial", "输出中"),
            ("cargo check", "partial\ndone", "completed"),
        ] {
            apply_event(
                &mut s,
                "turn-0",
                Event::Tool {
                    key: "cmd1".into(),
                    title: title.into(),
                    detail: detail.into(),
                    status: status.into(),
                },
            );
        }
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.messages[0].text, "cargo check");
        assert_eq!(s.messages[0].detail, "partial\ndone");
        assert_eq!(s.messages[0].status, "completed");
    }
    #[test]
    fn cancellation_only_marks_unfinished_tools_from_its_turn() {
        let mut s = session();
        for (prefix, key, status) in [
            ("turn-0", "old", "运行中"),
            ("turn-1", "done", "completed"),
            ("turn-1", "active", "输出中"),
        ] {
            apply_event(
                &mut s,
                prefix,
                Event::Tool {
                    key: key.into(),
                    title: "tool".into(),
                    detail: "partial result".into(),
                    status: status.into(),
                },
            );
        }
        finish_tool_cards(&mut s, "turn-1", "已取消");
        assert!(s.messages.iter().all(|m| m.kind == ItemKind::Tool));
        assert_eq!(s.messages[0].status, "运行中");
        assert_eq!(s.messages[1].status, "completed");
        assert_eq!(s.messages[2].status, "已取消");
        assert_eq!(s.messages[2].detail, "partial result");
    }
    #[test]
    fn adjacent_duplicate_completed_blocks_are_collapsed() {
        let mut s = session();
        for (key, text) in [("a", "duplicate"), ("b", "duplicate"), ("c", "next")] {
            apply_event(
                &mut s,
                "turn-0",
                Event::Text {
                    key: key.into(),
                    text: text.into(),
                },
            );
        }
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].text, "duplicate");
        assert_eq!(s.messages[1].text, "next");
    }
    #[test]
    fn failure_echoed_as_assistant_text_is_detected() {
        assert!(same_failure("执行失败：boom", "boom"));
        assert!(same_failure("boom", "执行失败：boom"));
        assert!(same_failure(
            "Failed to authenticate. API Error: 403\nmore detail",
            "Failed to authenticate. API Error: 403\nmore detail"
        ));
        assert!(!same_failure("ok", "boom"));
        assert!(!same_failure("", "boom"));
    }
    #[test]
    fn session_search_matches_title_and_message_text() {
        let mut s = session();
        s.title = "Rust 重构计划".into();
        s.messages.push(ChatItem {
            key: "m1".into(),
            kind: ItemKind::Assistant,
            text: "let me check the ALPHA dependency".into(),
            input: String::new(),
            detail: String::new(),
            status: String::new(),
        });
        assert!(session_matches(&s, ""));
        assert!(session_matches(&s, "rust"));
        assert!(session_matches(&s, "alpha"));
        assert!(!session_matches(&s, "beta"));
        assert!(!session_matches(&s, "已归档"));
    }
    #[test]
    fn highlight_ranges_handle_unicode_and_case() {
        assert_eq!(
            highlight_ranges("Hello hello HELLO", "hello"),
            vec![(0, 5), (6, 11), (12, 17)]
        );
        // "中文" is 6 bytes, so Markdown starts at byte 6 and ends at 14.
        assert_eq!(
            highlight_ranges("中文Markdown测试", "markdown"),
            vec![(6, 14)]
        );
        assert_eq!(highlight_ranges("多字节中文测试", "中文"), vec![(9, 15)]);
        assert!(highlight_ranges("", "x").is_empty());
        assert!(highlight_ranges("abc", "").is_empty());
        assert!(highlight_ranges("İstanbul", "i").iter().all(|(a, b)| a < b));
    }
    #[test]
    fn search_snippet_keeps_match_visible_for_multibyte_text() {
        let text = "这是很多中文内容，然后出现 Markdown 关键词，后面继续一些内容。";
        let snippet = search_snippet(text, "markdown");
        assert!(snippet.to_lowercase().contains("markdown"));
        assert!(snippet.chars().count() <= 162);
    }
}
