use crate::{
    approval::ApprovalQueue,
    ccs::{self, CcsCatalog},
    chat_runtime::ChatRuntime,
    cli::{self, VersionProbe},
    domain::{ChatItem, ItemKind, PermissionLevel, Provider, SessionRecord, Usage, Workspace},
    git::{self, BranchProbe, GitProbe},
    models::{self, ModelCatalog},
    performance::PerformanceMonitor,
    protocol::Event,
    store::{AppearanceMode, MAX_TAB_LIMIT, NamingMode, Store},
};
use chrono::{DateTime, Local};
use gpui_kit::{
    component::{
        alert::Alert,
        button::*,
        input::{Input, InputEvent, InputState, Textarea, TextareaState},
        menu::{ContextMenuExt, PopupMenuItem},
        popover::Popover,
        scroll::ScrollableElement,
        spinner::Spinner,
        switch::Switch,
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

struct SessionSummary {
    id: u64,
    provider: Provider,
    native_id: Option<String>,
    model: Option<String>,
    message_count: usize,
}

impl From<&SessionRecord> for SessionSummary {
    fn from(session: &SessionRecord) -> Self {
        Self {
            id: session.id,
            provider: session.provider,
            native_id: session.native_id.clone(),
            model: session.model.clone(),
            message_count: session.messages.len(),
        }
    }
}

struct ActiveRun {
    runtime: ChatRuntime,
    approvals: ApprovalQueue,
    prefix: String,
    status: String,
    /// What the engine is doing right now, shown in the chat viewport while
    /// the turn is still running (the tab keeps the shorter `status`).
    activity: String,
    message_index: MessageIndex,
    /// Usage reported by the running turn, plus turn-local token accounting
    /// used to derive the streaming tokens/s figure.
    usage: Usage,
    turn_tokens_at_start: u64,
    turn_started: Instant,
    last_usage_event: Instant,
}

/// Append-only message lookup, shared by every event in an active run.
#[derive(Default)]
struct MessageIndex {
    keys: HashMap<String, usize>,
    indexed_len: usize,
}

impl MessageIndex {
    fn find(&mut self, messages: &[ChatItem], key: &str) -> Option<usize> {
        if self.indexed_len > messages.len() {
            self.keys.clear();
            self.indexed_len = 0;
        }
        for (index, item) in messages.iter().enumerate().skip(self.indexed_len) {
            self.keys.entry(item.key.clone()).or_insert(index);
        }
        self.indexed_len = messages.len();
        self.keys.get(key).copied()
    }
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

/// One-shot background request that proposes a short session title.
struct SessionNaming {
    runtime: ChatRuntime,
    started: Instant,
    parts: Vec<(String, String)>,
    error: Option<String>,
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
    Appearance,
    Session,
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

/// Application colors that are not owned by the component theme. The dark
/// values preserve the original palette; the light variant is derived from the
/// same roles so both modes stay consistent.
struct Palette {
    canvas: u32,
    sidebar_bg: u32,
    surface: u32,
    surface_hover: u32,
    surface_strong: u32,
    border: u32,
    text: u32,
    muted: u32,
    faint: u32,
    focus: u32,
    on_text: u32,
    control_hover: u32,
    code_bg: u32,
    highlight_text: u32,
    highlight_bg: u32,
    diff_add_text: u32,
    diff_add_bg: u32,
    diff_remove_text: u32,
    diff_remove_bg: u32,
    diff_hunk_text: u32,
    danger_border: u32,
    danger_bg: u32,
    danger_text: u32,
    warning_border: u32,
    warning_bg: u32,
    warning_icon_bg: u32,
    warning_text: u32,
    warning_chip_bg: u32,
    warning_chip_text: u32,
    warning_muted: u32,
    warning_inner_border: u32,
    warning_code_bg: u32,
    warning_code_text: u32,
    warning_accent: u32,
    error_text: u32,
}

static DARK_PALETTE: Palette = Palette {
    canvas: 0x0e0e10,
    sidebar_bg: 0x131315,
    surface: 0x19191c,
    surface_hover: 0x232327,
    surface_strong: 0x202024,
    border: 0x29292e,
    text: 0xf3f3f4,
    muted: 0xa1a1a8,
    faint: 0x717179,
    focus: 0x4b4b54,
    on_text: 0x111113,
    control_hover: 0xffffff,
    code_bg: 0x141417,
    highlight_text: 0xf0d49b,
    highlight_bg: 0x3a3220,
    diff_add_text: 0x9fdcab,
    diff_add_bg: 0x18261c,
    diff_remove_text: 0xe6a1a1,
    diff_remove_bg: 0x2a1a1a,
    diff_hunk_text: 0x8fb8e6,
    danger_border: 0x5c332c,
    danger_bg: 0x2a1d1a,
    danger_text: 0xe6b4a8,
    warning_border: 0x4a3b22,
    warning_bg: 0x1d1a14,
    warning_icon_bg: 0x2a2317,
    warning_text: 0xe3b96b,
    warning_chip_bg: 0x332b1b,
    warning_chip_text: 0xd6b26b,
    warning_muted: 0x9c9284,
    warning_inner_border: 0x352c1b,
    warning_code_bg: 0x15130e,
    warning_code_text: 0xd9d3c7,
    warning_accent: 0xd6a75a,
    error_text: 0xe6a1a1,
};

static LIGHT_PALETTE: Palette = Palette {
    canvas: 0xf7f7f8,
    sidebar_bg: 0xf1f1f3,
    surface: 0xffffff,
    surface_hover: 0xe9e9ee,
    surface_strong: 0xf7f7f9,
    border: 0xe4e4e9,
    text: 0x1c1c20,
    muted: 0x5f5f68,
    faint: 0x8e8e97,
    focus: 0xb4b4bd,
    on_text: 0xffffff,
    control_hover: 0xe8e8ec,
    code_bg: 0xf4f4f6,
    highlight_text: 0x6b4a00,
    highlight_bg: 0xfff0bf,
    diff_add_text: 0x1e6631,
    diff_add_bg: 0xe9f7ed,
    diff_remove_text: 0xa43636,
    diff_remove_bg: 0xfdecec,
    diff_hunk_text: 0x2c5f9e,
    danger_border: 0xe7b8b1,
    danger_bg: 0xfdf1ef,
    danger_text: 0x8f2f29,
    warning_border: 0xe5cf9f,
    warning_bg: 0xfff8e9,
    warning_icon_bg: 0xf9eed2,
    warning_text: 0x8a5b00,
    warning_chip_bg: 0xf5e8c8,
    warning_chip_text: 0x785200,
    warning_muted: 0x8c7c5d,
    warning_inner_border: 0xe8d8b2,
    warning_code_bg: 0xf8f2e6,
    warning_code_text: 0x4b4336,
    warning_accent: 0x9a6a00,
    error_text: 0xb03b3b,
};

static DARK_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

fn palette() -> &'static Palette {
    if DARK_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        &DARK_PALETTE
    } else {
        &LIGHT_PALETTE
    }
}

pub(crate) fn set_palette(mode: gpui_kit::component::ThemeMode) {
    DARK_MODE.store(mode.is_dark(), std::sync::atomic::Ordering::Relaxed);
}

impl Render for PerformanceMonitor {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let memory = |bytes: Option<usize>| {
            bytes
                .map(|value| format!("{:.1} MiB", value as f64 / 1_048_576.0))
                .unwrap_or_else(|| "不可用".into())
        };
        let percent = |value: Option<f64>| {
            value
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| {
                    if self.sample_duration.is_zero() {
                        "采样中…".into()
                    } else {
                        "不可用".into()
                    }
                })
        };
        let timing = |value: crate::performance::Timings| {
            value
                .average()
                .map(|average| {
                    format!(
                        "{:.2} / {:.2} ms",
                        average.as_secs_f64() * 1000.0,
                        value.maximum.as_secs_f64() * 1000.0
                    )
                })
                .unwrap_or_else(|| "本周期空闲".into())
        };
        let rows = [
            ("CPU · 整机容量", percent(self.cpu_percent)),
            ("CPU · 近 60 秒峰值", percent(self.recent_cpu_peak())),
            ("驻留内存 · 工作集", memory(self.memory_bytes)),
            ("私有提交内存", memory(self.private_bytes)),
            ("工作集 · 进程生命周期峰值", memory(self.peak_memory_bytes)),
            (
                "私有提交 · 窗口内变化",
                self.private_growth()
                    .map(|bytes| format!("{:+.1} MiB", bytes as f64 / 1_048_576.0))
                    .unwrap_or_else(|| "等待后续样本".into()),
            ),
            ("主视图构建 · 均值 / 最大", timing(self.render)),
            ("单条消息构建 · 均值 / 最大", timing(self.message_render)),
            (
                "最近一帧构建 / 会话消息",
                format!("{} / {} 条", self.mounted_messages(), self.message_count),
            ),
            ("本周期主视图构建", format!("{} 次", self.render.count)),
            ("本周期引擎事件", format!("{} 条", self.event_count)),
            (
                "运行任务 / 已加载会话",
                format!("{} / {}", self.active_runs, self.session_count),
            ),
            ("运行时长", self.uptime()),
        ];
        v_flex()
            .id("performance-details")
            .w(px(350.))
            .max_h((window.viewport_size().height - px(72.)).min(px(460.)))
            .overflow_y_scroll()
            .p_3()
            .gap_2()
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .shadow_md()
            .text_color(rgb(palette().text))
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(gpui_kit::assets::IconName::Activity).small())
                    .child(div().text_size(px(13.)).font_semibold().child("性能检测"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(if cfg!(debug_assertions) {
                                "开发构建"
                            } else {
                                "发布构建"
                            }),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "Conduit · PID {} · {} 逻辑处理器",
                        std::process::id(),
                        self.logical_cpus
                    )),
            )
            .child(
                v_flex()
                    .gap_2()
                    .py_2()
                    .border_t_1()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .children(rows.into_iter().map(|(label, value)| {
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .text_size(px(12.))
                            .child(div().text_color(rgb(palette().muted)).child(label))
                            .child(div().flex_shrink_0().child(value))
                    })),
            )
            .child(
                v_flex()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child("仅统计 Conduit 主进程，CLI / Git 子进程不计入")
                    .child("工作集含共享页；私有提交不等于实际驻留内存")
                    .child("构建耗时不含布局、GPU 绘制，不代表帧率")
                    .child("约每 2 秒采样，保留最近 60 秒内最多 30 条")
                    .child("点击图标可固定展开；窗口变化按已有样本计算"),
            )
    }
}

fn provider_icon(provider: Provider) -> Icon {
    let (data, color): (&[u8], u32) = match provider {
        Provider::Claude => (include_bytes!("../assets/icons/claude.svg"), 0xd97757),
        Provider::Codex => (include_bytes!("../assets/icons/openai.svg"), palette().text),
    };
    Icon::default()
        .data(data)
        .size(px(14.))
        .text_color(rgb(color))
}

fn message_action_button(id: SharedString, icon: Icon, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .compact()
        .icon(icon)
        .tooltip(tooltip)
        .accessibility_label(tooltip)
        .cursor_pointer()
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
        .text_color(rgb(palette().faint))
        .child(text)
}

/// A read-only settings card row: fixed label on the left, value on the right.
fn settings_row(label: &'static str, value: String, divider: bool) -> AnyElement {
    let mut row = h_flex().px_4().py_3().gap_4().items_center();
    if divider {
        row = row.border_t_1().border_color(rgb(palette().border));
    }
    row.child(settings_label(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .child(value),
        )
        .into_any_element()
}

/// Titles imported from CLI transcripts may contain line breaks; list rows
/// render a single line, so embedded newlines must collapse to spaces.
fn single_line_title(title: &str) -> String {
    title.replace(['\r', '\n'], " ")
}

/// The most recent user/assistant exchange, flattened for AI titling. Bounded
/// by both message count and character count so the naming prompt stays small.
fn latest_naming_excerpt(session: &SessionRecord) -> Option<String> {
    const MAX_MESSAGES: usize = 4;
    const MAX_CHARS: usize = 800;
    let mut parts: Vec<&str> = session
        .messages
        .iter()
        .rev()
        .filter(|item| matches!(item.kind, ItemKind::User | ItemKind::Assistant))
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .take(MAX_MESSAGES)
        .collect();
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    let joined = parts.join("\n\n");
    let total = joined.chars().count();
    Some(
        joined
            .chars()
            .skip(total.saturating_sub(MAX_CHARS))
            .collect(),
    )
}

fn naming_mode_label(mode: NamingMode) -> &'static str {
    match mode {
        NamingMode::Latest => "根据最新内容",
        NamingMode::FirstMessage => "根据第一条消息",
    }
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

struct MessageOutlineEntry {
    key: String,
    index: usize,
    label: String,
}

#[derive(Default)]
struct MessageOutlineCache {
    session: Option<u64>,
    indexed_len: usize,
    entries: Vec<MessageOutlineEntry>,
    active: Option<usize>,
    scroll: UniformListScrollHandle,
}

impl MessageOutlineCache {
    fn refresh(&mut self, session: Option<&SessionRecord>) {
        let id = session.map(|session| session.id);
        let count = session.map_or(0, |session| session.messages.len());
        if self.session != id || count < self.indexed_len {
            *self = Self {
                session: id,
                ..Default::default()
            };
        }
        if let Some(session) = session {
            // Submitted user messages are immutable; only inspect appended items.
            self.entries
                .extend(message_outline_entries(session, self.indexed_len));
        }
        self.indexed_len = count;
    }
}

/// Tooltip text for an outline marker: long prompts are ellipsized so the
/// popup stays narrow and never has to flip to the right side of the rail.
fn outline_tooltip(label: &str) -> SharedString {
    let mut preview: String = label.chars().take(40).collect();
    if label.chars().count() > 40 {
        preview.push('…');
    }
    SharedString::from(preview)
}

fn outline_marker_width(label: &str) -> f32 {
    // Keep short prompts visibly light while allowing longer prompts to form a
    // stronger visual rhythm. The cap prevents one verbose message from
    // pushing the outline into the chat column.
    (8. + label.chars().count().min(20) as f32 * 0.8).min(24.)
}

fn message_outline_entries(session: &SessionRecord, start: usize) -> Vec<MessageOutlineEntry> {
    session
        .messages
        .iter()
        .enumerate()
        .skip(start)
        .filter(|(_, item)| item.kind == ItemKind::User)
        .map(|(index, item)| {
            // Bound preview work even when a user pastes a very large prompt.
            let mut chars = item
                .text
                .split_whitespace()
                .flat_map(|word| std::iter::once(' ').chain(word.chars()))
                .skip(1);
            let mut preview: String = chars.by_ref().take(100).collect();
            if chars.next().is_some() {
                preview.push('…');
            }
            MessageOutlineEntry {
                key: item.key.clone(),
                index,
                label: if preview.is_empty() {
                    "（空消息）".into()
                } else {
                    preview
                },
            }
        })
        .collect()
}

/// Tool output is literal text, never Markdown or HTML. Strip terminal control
/// sequences and expand tabs so logs, JSON and code remain readable together.
fn format_tool_text(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' {
                            break;
                        }
                        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                plain.push('\n');
            }
            '\t' => plain.push_str("    "),
            '\n' => plain.push(ch),
            ch if !ch.is_control() => plain.push(ch),
            _ => {}
        }
    }
    if let Ok(value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
        serde_json::from_str(&plain)
    {
        return serde_json::to_string_pretty(&value).unwrap_or(plain);
    }
    plain
}

/// Use a block viewport around natural-height flex content. Constraining the
/// flex column itself lets its text children shrink and erases scroll overflow.
fn tool_content(tool_id: &str, item: &ChatItem, scroll: &ScrollHandle) -> (AnyElement, String) {
    let input = format_tool_text(&item.input);
    let output = format_tool_text(&item.detail);
    let copy_text = format!("输入\n{input}\n\n输出 / 结果\n{output}");
    let mut content = v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .p_3()
        .gap_2()
        .font_family("Consolas")
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(rgb(palette().muted));
    for (section, label, text, empty) in [
        ("input", "输入", input, "未提供输入"),
        (
            "output",
            "输出 / 结果",
            output,
            if matches!(item.status.as_str(), "运行中" | "输出中") {
                "等待工具输出…"
            } else {
                "没有输出内容"
            },
        ),
    ] {
        content = content
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(palette().faint))
                    .child(label),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .child(gpui_kit::base::SelectableText::new(
                        SharedString::from(format!("{tool_id}-{section}")),
                        if text.trim().is_empty() {
                            empty.into()
                        } else {
                            text
                        },
                    )),
            );
    }
    let viewport = div()
        .id(SharedString::from(format!("{tool_id}-content")))
        .h(px(240.))
        .min_h_0()
        .flex_shrink_0()
        .w_full()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        // GPUI otherwise delivers the same wheel event to the chat viewport.
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .relative()
        .child(content)
        .vertical_scrollbar(scroll)
        .into_any_element();
    (viewport, copy_text)
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
                    color: Some(Hsla::from(rgb(palette().highlight_text))),
                    background_color: Some(Hsla::from(rgb(palette().highlight_bg))),
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
                git::DiffLine::Added => (palette().diff_add_text, Some(palette().diff_add_bg)),
                git::DiffLine::Removed => {
                    (palette().diff_remove_text, Some(palette().diff_remove_bg))
                }
                git::DiffLine::Hunk => (palette().diff_hunk_text, None),
                git::DiffLine::Header => (palette().faint, None),
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
        .border_color(rgb(palette().border))
        .bg(rgb(palette().code_bg))
        .font_family("Consolas")
        .text_size(px(10.))
        .line_height(relative(1.35))
        .text_color(rgb(palette().muted))
        .child(styled)
        .into_any_element()
}

/// Menu popovers render only compact buttons, so the default `p_3()` popover
/// padding (12px on every side) makes the menu look sparse. This refinement is
/// applied through `Popover`'s own `Styled` impl, which `refine_style`s the
/// content container after the default padding, overriding it.
fn menu_popover_style() -> StyleRefinement {
    StyleRefinement::default().p_1().style().clone()
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
                .child(Icon::new(icon).small().text_color(rgb(palette().faint)))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().muted))
                        .child(label),
                ),
        )
}

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
    /// In-flight session naming requests, keyed by session id.
    session_namings: HashMap<u64, SessionNaming>,
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
    scroll: gpui_kit::ListState,
    timeline_session: Option<u64>,
    outline: MessageOutlineCache,
    /// Whether the message list is scrolled to (or near) its end. Drives the
    /// floating "jump to latest" arrow so it only shows once the user scrolls up.
    at_bottom: bool,
    /// Scoped by session and tool key, independent of the right detail dock.
    expanded_tools: HashSet<(u64, String)>,
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
        let (mut store, message, writable) = match Store::load() {
            Ok(store) => (store, None, true),
            Err(error) => (Store::default(), Some(format!("{error:#}")), false),
        };
        let initial_mode = match store.ui.appearance {
            AppearanceMode::System => gpui_kit::component::ThemeMode::from(window.appearance()),
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
        };
        set_palette(initial_mode);
        gpui_kit::component::Theme::change(initial_mode, Some(window), cx);
        let next_id = store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .map(|s| s.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        // Import CLI-side (terminal) sessions for known workspaces so they
        // appear in the sidebar and can be resumed natively. Sessions whose
        // native id already exists are refreshed instead of duplicated.
        let imported_total = {
            let mut next_id = next_id;
            for workspace in &mut store.workspaces {
                crate::history::merge_into(workspace, &mut next_id);
            }
            next_id
        };
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
                        run.runtime.cancel();
                        run.approvals.clear();
                        run.status = "正在停止…".into();
                        run.activity = "正在停止…".into();
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
        let appearance_subscription = cx.observe_window_appearance(window, |this, window, cx| {
            if this.store.ui.appearance == AppearanceMode::System {
                this.apply_appearance(window, cx);
            }
        });
        Self {
            store,
            writable,
            selected_workspace,
            selected_session,
            open_tabs: selected_session.into_iter().collect(),
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
            next_id: imported_total,
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
            _subscriptions: vec![
                search_subscription,
                git_prompt_subscription,
                appearance_subscription,
            ],
            dirty: false,
            last_save: Instant::now(),
            scroll: gpui_kit::ListState::new(0, ListAlignment::Bottom, px(240.)),
            timeline_session: None,
            outline: MessageOutlineCache::default(),
            at_bottom: true,
            expanded_tools: HashSet::new(),
            tool_scrolls: HashMap::new(),
            message_focus: None,
            pending_message_jump: false,
            performance: cx.new(|_| PerformanceMonitor::new()),
            performance_open: false,
            copied_action: None,
            copied_action_seq: 0,
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
            self.push_tab(self.selected_session.unwrap());
            return;
        }
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            if let Some(session) = workspace.sessions.iter().find(|s| !s.archived) {
                self.selected_workspace = Some(index);
                self.selected_session = Some(session.id);
                self.push_tab(session.id);
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
    /// Open a tab without reordering the strip. New tabs append at the end and
    /// the oldest non-selected tab is evicted past the configured limit.
    fn push_tab(&mut self, id: u64) {
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id);
        }
        let limit = self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT);
        while self.open_tabs.len() > limit {
            if let Some(position) = self
                .open_tabs
                .iter()
                .position(|tab| Some(*tab) != self.selected_session)
            {
                self.open_tabs.remove(position);
            } else {
                break;
            }
        }
    }
    /// Drop a tab without touching the current selection; used when the tab
    /// is closed while another session stays active.
    fn remove_tab(&mut self, id: u64) {
        self.open_tabs.retain(|tab| *tab != id);
    }
    fn toggle_archive(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(id) {
            session.archived = !session.archived;
        }
        let archived_tabs: Vec<u64> = self
            .open_tabs
            .iter()
            .copied()
            .filter(|tab| self.session(*tab).is_none_or(|session| session.archived))
            .collect();
        self.open_tabs.retain(|tab| !archived_tabs.contains(tab));
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
        // A manual rename wins over any title the naming engine still owes us.
        if let Some(naming) = self.session_namings.remove(&id) {
            naming.runtime.cancel();
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
                                    .text_color(rgb(palette().muted))
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
        self.expanded_tools.retain(|(session, _)| *session != id);
        self.tool_scrolls.retain(|(session, _), _| *session != id);
        self.remove_tab(id);
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
        // Switching the engine targets the *next* new session: it never
        // jumps to an existing history session. The current session stays
        // selected and its tab stays open.
        let _ = window;
        cx.notify();
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
                            .text_color(rgb(palette().muted))
                            .child(provider.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
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
                                        .text_color(rgb(palette().muted))
                                        .child(option.label()),
                                ),
                        )
                        .when(active, |button| button.bg(rgb(palette().surface_hover)))
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
    /// Session permission selector next to the engine picker. FullAccess is
    /// visually flagged (warning colors + alert icon) because it removes the
    /// CLI's own safety gates for this session.
    fn permission_picker(&self, view: Entity<Self>) -> AnyElement {
        let permission = self
            .selected_session
            .and_then(|id| self.session(id))
            .map(|session| session.permission)
            .unwrap_or_default();
        let warning = permission == PermissionLevel::FullAccess;
        let trigger = Button::new("permission-picker-trigger")
            .ghost()
            .small()
            .h(px(22.))
            .px_1()
            .cursor_pointer()
            .tooltip(match permission {
                PermissionLevel::ReadOnly => "会话权限：仅可查看",
                PermissionLevel::WorkspaceWrite => "会话权限：工作区内修改",
                PermissionLevel::FullAccess => "会话权限：完全权限（警告：不受沙箱限制）",
            })
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(if warning {
                            gpui_kit::assets::IconName::TriangleAlert
                        } else {
                            gpui_kit::assets::IconName::Shield
                        })
                        .small()
                        .text_color(rgb(if warning {
                            palette().warning_text
                        } else {
                            palette().faint
                        })),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if warning {
                                palette().warning_text
                            } else {
                                palette().muted
                            }))
                            .child(permission.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("permission-picker")
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                for option in [
                    PermissionLevel::ReadOnly,
                    PermissionLevel::WorkspaceWrite,
                    PermissionLevel::FullAccess,
                ] {
                    let app = view.clone();
                    let popover = popover.clone();
                    let active = option == permission;
                    let is_danger = option == PermissionLevel::FullAccess;
                    list = list.child(
                        Button::new(match option {
                            PermissionLevel::ReadOnly => "permission-option-readonly",
                            PermissionLevel::WorkspaceWrite => "permission-option-workspace",
                            PermissionLevel::FullAccess => "permission-option-full",
                        })
                        .ghost()
                        .h(px(34.))
                        .w_full()
                        .px_2()
                        .cursor_pointer()
                        .accessibility_label(option.label())
                        .child(
                            v_flex()
                                .w_full()
                                .gap(px(2.))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            Icon::new(if is_danger {
                                                gpui_kit::assets::IconName::TriangleAlert
                                            } else {
                                                match option {
                                                    PermissionLevel::ReadOnly => {
                                                        gpui_kit::assets::IconName::Eye
                                                    }
                                                    _ => gpui_kit::assets::IconName::SquarePen,
                                                }
                                            })
                                            .small()
                                            .text_color(rgb(if is_danger {
                                                palette().warning_text
                                            } else {
                                                palette().muted
                                            })),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(rgb(if is_danger {
                                                    palette().warning_text
                                                } else {
                                                    palette().muted
                                                }))
                                                .child(option.label()),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl_6()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(match option {
                                            PermissionLevel::ReadOnly => {
                                                "CLI 沙箱只读，不允许写入与命令执行"
                                            }
                                            PermissionLevel::WorkspaceWrite => {
                                                "允许修改工作区内文件，其余仍需审批"
                                            }
                                            PermissionLevel::FullAccess => {
                                                "警告：不受沙箱限制，命令不经审批直接执行"
                                            }
                                        }),
                                ),
                        )
                        .when(active, |button| button.bg(rgb(palette().surface_hover)))
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.set_session_permission(option, window, cx)
                            });
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
    fn set_session_permission(
        &mut self,
        permission: PermissionLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        if let Some(id) = self.selected_session
            && let Some(session) = self.session_mut(id)
            && session.permission != permission
        {
            session.permission = permission;
            self.persist();
        }
        // Permission applies to the next turn; while a run is active the
        // current process keeps its existing sandbox.
        if self
            .selected_session
            .is_none_or(|id| !self.runs.contains_key(&id))
        {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }
    /// Compact token/usage indicators (tokens/s while streaming, cumulative
    /// usage, and a ring gauge for context window consumption) rendered on
    /// the composer's engine row. Reads from the active run when streaming
    /// and falls back to the session's last persisted snapshot when idle.
    /// Returns `None` when there is nothing to show yet.
    fn usage_status_parts(&self) -> Option<Vec<AnyElement>> {
        let session = self.selected_session.and_then(|id| self.session(id));
        let (usage, streaming, elapsed, since_event) = match self.selected_session {
            Some(id) if self.runs.contains_key(&id) => {
                let run = &self.runs[&id];
                (
                    run.usage,
                    true,
                    run.turn_started.elapsed(),
                    run.last_usage_event.elapsed(),
                )
            }
            _ => (
                session.map(|s| s.usage).unwrap_or_default(),
                false,
                Duration::ZERO,
                Duration::ZERO,
            ),
        };
        if usage.total_tokens == 0 {
            return None;
        }
        // Tokens/s is derived from this turn's delta; pause the clock while no
        // usage events arrive (approval wait, model idle) so the figure does
        // not decay toward zero during stalls.
        let run = self.selected_session.and_then(|id| self.runs.get(&id));
        let tokens_per_second = if streaming {
            let baseline = run
                .map(|r| r.turn_tokens_at_start)
                .unwrap_or(usage.total_tokens);
            let active = if since_event < Duration::from_secs(10) {
                elapsed
            } else {
                Duration::ZERO
            };
            let seconds = active.as_secs_f64();
            if seconds >= 1.0 && usage.total_tokens > baseline {
                Some(((usage.total_tokens - baseline) as f64 / seconds).round() as u64)
            } else {
                None
            }
        } else {
            None
        };
        let format_tokens = |tokens: u64| {
            if tokens >= 1_000_000 {
                format!("{:.1}M", tokens as f64 / 1_000_000.0)
            } else if tokens >= 1_000 {
                format!("{:.1}K", tokens as f64 / 1_000.0)
            } else {
                format!("{tokens}")
            }
        };
        let context_ratio = {
            // The CLI-reported window (Codex) wins; Claude stream-json carries
            // no window size, so fall back to the model catalog (context_window
            // field or the model ID's `[1M]` suffix). Estimated rings show a
            // "≈" prefix since the real window is model-side policy.
            let window = if usage.context_window > 0 {
                Some(usage.context_window)
            } else {
                let model_id = session
                    .and_then(|s| s.model.as_deref())
                    .map(str::to_owned)
                    .or_else(|| {
                        // No session model override: use the CLI's current model.
                        self.catalogs
                            .get(
                                &session
                                    .map(|s| s.provider)
                                    .unwrap_or(self.new_session_provider),
                            )
                            .and_then(|catalog| catalog.current.clone())
                    });
                let provider = session
                    .map(|s| s.provider)
                    .unwrap_or(self.new_session_provider);
                model_id.and_then(|id| {
                    self.catalogs
                        .get(&provider)
                        .and_then(|catalog| catalog.context_window(&id))
                })
            };
            match window {
                Some(window) if window > 0 => Some((
                    window,
                    (usage.total_tokens as f64 / window as f64).clamp(0.0, 1.0),
                )),
                _ => None,
            }
        };
        let (context_window_display, context_ratio, estimated) = match context_ratio {
            Some((window, ratio)) => (window, ratio, usage.context_window == 0),
            None => (0, f64::INFINITY, false),
        };
        let ring = if context_ratio.is_finite() {
            // 270° arc gauge drawn as an SVG stroke; gpui renders SVG paths
            // with a fixed viewBox, so the arc is recomputed per ratio.
            let radius = 7.0f64;
            let center = 9.0f64;
            let start_angle = 135.0f64;
            let sweep = 270.0 * context_ratio;
            let point = |angle: f64| {
                let radians = angle.to_radians();
                (
                    center + radius * radians.cos(),
                    center + radius * radians.sin(),
                )
            };
            let (sx, sy) = point(start_angle);
            let (ex, ey) = point(start_angle + sweep);
            let large = if sweep > 180.0 { 1 } else { 0 };
            let color = if context_ratio >= 0.9 {
                palette().danger_text
            } else if context_ratio >= 0.75 {
                palette().warning_text
            } else {
                palette().highlight_text
            };
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 18 18\">\
                 <circle cx=\"9\" cy=\"9\" r=\"7\" fill=\"none\" stroke=\"#{border:06x}\" stroke-width=\"2\"/>\
                 <path d=\"M {sx:.2} {sy:.2} A 7 7 0 {large} 1 {ex:.2} {ey:.2}\" fill=\"none\" stroke=\"#{color:06x}\" stroke-width=\"2\" stroke-linecap=\"round\"/>\
                 </svg>",
                border = palette().border,
                color = color,
            );
            h_flex()
                .id("context-ring")
                .w(px(18.))
                .h(px(18.))
                .flex_shrink_0()
                .child(
                    Icon::default()
                        .data(svg.as_bytes())
                        .size(px(18.))
                        .into_any_element(),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let mut parts: Vec<AnyElement> = vec![];
        if let Some(speed) = tokens_per_second {
            parts.push(
                h_flex()
                    .id("usage-speed")
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Zap)
                            .small()
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().muted))
                            .child(format!("{speed} tok/s")),
                    )
                    .into_any_element(),
            );
        }
        parts.push(
            h_flex()
                .id("usage-total")
                .gap_1()
                .items_center()
                .child(
                    Icon::new(gpui_kit::assets::IconName::Gauge)
                        .small()
                        .text_color(rgb(palette().faint)),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(palette().muted))
                        .child(format!(
                            "↑{} ↓{}",
                            format_tokens(usage.input_tokens + usage.cached_input_tokens),
                            format_tokens(usage.output_tokens)
                        )),
                )
                .into_any_element(),
        );
        if context_ratio.is_finite() {
            parts.push(
                h_flex()
                    .id("usage-context")
                    .gap_1()
                    .items_center()
                    .child(ring)
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "{}{} / {}",
                                if estimated { "≈" } else { "" },
                                format_tokens(usage.total_tokens),
                                format_tokens(context_window_display)
                            )),
                    )
                    .into_any_element(),
            );
        }
        if parts.is_empty() {
            return None;
        }
        Some(parts)
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
                                    .text_color(rgb(palette().muted))
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
            self.expanded_tools.retain(|(id, _)| *id != session.id);
            self.tool_scrolls.retain(|(id, _), _| *id != session.id);
        }
        let removed_ids: Vec<u64> = removed.sessions.iter().map(|session| session.id).collect();
        self.open_tabs.retain(|tab| !removed_ids.contains(tab));
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
            .refine_style(&menu_popover_style())
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
            for session in workspace.sessions.iter() {
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
                        Input::new(&input).h(px(34.)).prefix(
                            Icon::new(IconName::Search)
                                .small()
                                .text_color(rgb(palette().faint)),
                        ),
                    );
                    if hits.is_empty() {
                        body = body.child(
                            div()
                                .py_2()
                                .text_size(px(12.))
                                .text_color(rgb(palette().faint))
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
                                    .hover(|style| style.bg(rgb(palette().surface_hover)))
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
                                            .child(div().flex_1().min_w_0().child(
                                                highlighted_text(
                                                    &title,
                                                    &query,
                                                    13.,
                                                    palette().text,
                                                ),
                                            ))
                                            .when(archived, |row| {
                                                row.child(
                                                    div()
                                                        .text_size(px(9.))
                                                        .text_color(rgb(palette().faint))
                                                        .child("已归档"),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(workspace_name),
                                            ),
                                    )
                                    .when(!snippet.is_empty(), |row| {
                                        row.child(highlighted_text(
                                            &snippet,
                                            &query,
                                            11.,
                                            palette().muted,
                                        ))
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
    fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.store.ui.appearance {
            AppearanceMode::System => gpui_kit::component::ThemeMode::from(window.appearance()),
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
        };
        set_palette(mode);
        if gpui_kit::component::Theme::global(cx).mode != mode {
            gpui_kit::component::Theme::change(mode, Some(window), cx);
        }
        cx.notify();
    }
    /// Whether the tool detail dock is currently visible.
    fn details_open(&self) -> bool {
        self.details && self.detail_view == DetailView::Tool
    }
    /// Toggle button for the tool detail dock, pinned to the right of the tab bar.
    fn detail_toggle(&self, cx: &mut Context<Self>) -> Button {
        Button::new("toggle-sidebar")
            .ghost()
            .small()
            .compact()
            .icon(gpui_kit::assets::IconName::PanelRight)
            .cursor_pointer()
            .tooltip(if self.details_open() {
                "隐藏工具详情"
            } else {
                "显示工具详情"
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.details = !this.details_open();
                this.detail_view = DetailView::Tool;
                cx.notify();
            }))
    }
    /// Close a session tab. Closing the active tab reveals the next most
    /// recent one; the conversation itself is kept in the sidebar.
    fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.selected_session == Some(id) {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
            self.remove_tab(id);
            if let Some(next) = self
                .open_tabs
                .last()
                .copied()
                .filter(|next| self.session(*next).is_some())
            {
                let window = cx
                    .windows()
                    .into_iter()
                    .find_map(|handle| handle.downcast::<Self>());
                if let Some(window) = window {
                    let _ = window.update(cx, |_, window, cx| {
                        if let Some(workspace) =
                            self.store.workspaces.iter().position(|workspace| {
                                workspace.sessions.iter().any(|s| s.id == next)
                            })
                        {
                            self.switch(workspace, Some(next), window, cx);
                        }
                    });
                }
            } else {
                self.selected_session = None;
                self.detail_text = None;
                self.message = None;
                cx.notify();
            }
        } else {
            self.remove_tab(id);
            cx.notify();
        }
    }
    /// Conversation tab strip: one tab per open session, horizontally
    /// scrollable, with a fixed-width action area pinned to the right.
    fn session_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.selected_session;
        let mut tabs = h_flex()
            .id("session-tabs")
            .h_full()
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .pr(px(4.))
            .overflow_x_scroll()
            .lock_scroll_axis();
        for id in &self.open_tabs {
            let id = *id;
            let Some(session) = self.session(id) else {
                continue;
            };
            let selected = active == Some(id);
            let running = self.runs.contains_key(&id);
            let title = if session.title.is_empty() {
                "???".to_string()
            } else {
                session.title.clone()
            };
            let title = single_line_title(&title);
            let status = self
                .runs
                .get(&id)
                .map(|run| run.status.clone())
                .filter(|status| !status.is_empty());
            tabs = tabs.child(
                h_flex()
                    .id(("session-tab", id))
                    .h(px(30.))
                    .my_auto()
                    .px(px(10.))
                    .gap(px(6.))
                    .flex_shrink_0()
                    .items_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if selected {
                        palette().border
                    } else {
                        palette().sidebar_bg
                    }))
                    .when(selected, |tab| tab.bg(rgb(palette().surface)))
                    .text_size(px(12.))
                    .text_color(rgb(if selected {
                        palette().text
                    } else {
                        palette().muted
                    }))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(palette().surface_hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(workspace) = this
                            .store
                            .workspaces
                            .iter()
                            .position(|workspace| workspace.sessions.iter().any(|s| s.id == id))
                        {
                            this.switch(workspace, Some(id), window, cx);
                        }
                    }))
                    .child(provider_icon(session.provider))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .line_height(px(16.))
                            .whitespace_nowrap()
                            .child(title),
                    )
                    .when_some(status, |tab, status| {
                        tab.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child(status),
                        )
                    })
                    .child(
                        Button::new(("session-tab-close", id))
                            .ghost()
                            .small()
                            .compact()
                            .icon(gpui_kit::assets::IconName::X)
                            .cursor_pointer()
                            .tooltip(if running { "????????????" } else { "????" })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_tab(id, cx);
                                cx.stop_propagation();
                            })),
                    ),
            );
        }
        if self.open_tabs.is_empty() {
            tabs = tabs.child(
                h_flex()
                    .h_full()
                    .px_3()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(rgb(palette().faint))
                    .child("???????????"),
            );
        }
        h_flex()
            .h(px(44.))
            .pl_2()
            .pr_3()
            .border_b_1()
            .border_color(rgb(palette().border))
            .items_center()
            .child(tabs)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(div().w(px(1.)).h(px(18.)).mx_1().bg(rgb(palette().border)))
                    .child(self.detail_toggle(cx)),
            )
            .into_any_element()
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
        if let Some(id) = session {
            self.push_tab(id);
        }
        self.message_focus = None;
        self.pending_message_jump = false;
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
        self.timeline_session = None;
        // A newly selected conversation opens at its latest message.
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
                                    this.store.workspaces[index].sessions.first().map(|s| s.id);
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
        self.store.workspaces[index].sessions.insert(
            0,
            SessionRecord {
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
                permission: PermissionLevel::default(),
                usage: Usage::default(),
            },
        );
        self.selected_workspace = Some(index);
        self.selected_session = Some(id);
        self.push_tab(id);
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
        let prompt = self.input.read(cx).value().to_string();
        self.send_text(prompt, true, window, cx);
    }
    /// Starts one engine turn. `append_user` is false when regenerating: the
    /// user message is already in the transcript and must not be duplicated.
    fn send_text(
        &mut self,
        prompt: String,
        append_user: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.closing || !self.writable {
            return false;
        }
        let Some(index) = self.selected_workspace else {
            self.message = Some("请先在左上角选择工作区。".into());
            cx.notify();
            return false;
        };
        if self.workspace_busy(index) {
            self.message = Some("该工作区已有任务运行，请等待完成或取消后发送。".into());
            cx.notify();
            return false;
        }
        if prompt.trim().is_empty() {
            return false;
        }
        if prompt.len() > 256 * 1024 {
            self.message = Some("单条消息最多 256 KiB。".into());
            cx.notify();
            return false;
        }
        // Resolve the target engine and validate the environment before creating
        // a session, so a failed send cannot leave an empty conversation behind.
        // A pending engine switch (composer engine differs from the selected
        // session's) retargets the message to a fresh session instead of
        // jumping to history. Regeneration always stays on its own session.
        if append_user {
            let retarget = self.selected_session.is_some_and(|id| {
                self.session(id)
                    .is_some_and(|session| session.provider != self.new_session_provider)
            });
            if retarget && let Some(id) = self.selected_session {
                self.drafts
                    .insert(id, self.input.read(cx).value().to_string());
                self.selected_session = None;
            }
        } else if self.selected_session.is_none() {
            return false;
        }
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
                return false;
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
                return false;
            }
        };
        let cwd = self.store.workspaces[index].path.clone();
        if !cwd.is_dir() {
            self.message = Some("工作区目录已移动或不可访问。".into());
            cx.notify();
            return false;
        }
        // Typing in a workspace without a selected session starts one with the
        // last used engine instead of asking the user to click "新建会话".
        let id = match self.selected_session {
            Some(id) => id,
            None => match self.create_session(index, provider, cx) {
                Some(id) => id,
                None => return false,
            },
        };
        let native_id = self
            .session(id)
            .and_then(|session| session.native_id.clone());
        let model = self.session(id).and_then(|session| session.model.clone());
        let permission = self
            .session(id)
            .map(|session| session.permission)
            .unwrap_or_default();
        let session = self.session_mut(id).unwrap();
        let prefix = format!("turn-{}", session.messages.len());
        if append_user {
            if !session.messages.iter().any(|m| m.kind == ItemKind::User) {
                session.title = prompt
                    .lines()
                    .next()
                    .unwrap_or("新会话")
                    .trim()
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
                approval: None,
            });
        }
        session.interrupted = true;
        if !self.persist() {
            let session = self.session_mut(id).unwrap();
            if append_user {
                session.messages.pop();
            }
            session.interrupted = false;
            cx.notify();
            return false;
        }
        let first_turn = append_user
            && !self.session(id).is_some_and(|session| {
                session
                    .messages
                    .iter()
                    .any(|message| message.kind == ItemKind::Assistant)
            });
        let naming_snippet = (self.store.ui.naming_mode == NamingMode::FirstMessage && first_turn)
            .then(|| prompt.clone());
        let runtime = ChatRuntime::start(
            provider, executable, cwd, prompt, native_id, model, permission,
        );
        let usage = self
            .session(id)
            .map(|session| session.usage)
            .unwrap_or_default();
        let now = Instant::now();
        self.runs.insert(
            id,
            ActiveRun {
                runtime,
                approvals: ApprovalQueue::default(),
                prefix,
                status: "正在连接 CLI…".into(),
                activity: "正在启动 CLI…".into(),
                message_index: MessageIndex::default(),
                usage,
                turn_tokens_at_start: usage.total_tokens,
                turn_started: now,
                last_usage_event: now,
            },
        );
        if append_user {
            self.input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.drafts.remove(&id);
        }
        self.input.read(cx).focus_handle(cx).focus(window, cx);
        self.message = None;
        if let Some(snippet) = naming_snippet {
            self.start_session_naming(id, &snippet);
        }
        self.scroll_to_end();
        cx.notify();
        true
    }
    /// Re-runs the latest exchange: keeps its user message, drops everything
    /// after it, and starts a fresh engine turn with the same prompt.
    fn regenerate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || !self.writable {
            return;
        }
        let Some(id) = self.selected_session else {
            return;
        };
        let Some(session) = self.session(id) else {
            return;
        };
        if self.runs.contains_key(&id)
            || session
                .messages
                .get(index)
                .is_none_or(|item| item.kind != ItemKind::Assistant)
            || session.messages[index + 1..]
                .iter()
                .any(|item| matches!(item.kind, ItemKind::User | ItemKind::Assistant))
        {
            return;
        }
        let Some(user_index) = session.messages[..index]
            .iter()
            .rposition(|item| item.kind == ItemKind::User)
        else {
            return;
        };
        if session.native_id.is_none() {
            self.message =
                Some("该会话缺少可靠的 CLI 会话标识，请新建会话，避免丢失上下文。".into());
            cx.notify();
            return;
        }
        let prompt = session.messages[user_index].text.clone();
        let removed = self
            .session_mut(id)
            .map(|session| session.messages.split_off(user_index + 1))
            .unwrap_or_default();
        if !self.send_text(prompt, false, window, cx)
            && let Some(session) = self.session_mut(id)
        {
            session.messages.extend(removed);
            cx.notify();
        }
    }
    /// Ask the naming engine (settings → 会话) for a short title in the
    /// background; failures are silent and simply keep the current title.
    fn start_session_naming(&mut self, id: u64, content: &str) {
        if self.session_namings.contains_key(&id) {
            return;
        }
        let provider = self
            .store
            .ui
            .naming_provider
            .or_else(|| self.session(id).map(|session| session.provider))
            .unwrap_or(self.new_session_provider);
        let model = self
            .store
            .ui
            .naming_model
            .clone()
            .or_else(|| self.session(id).and_then(|session| session.model.clone()));
        let executable = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(_) => return,
        };
        let snippet: String = content.chars().take(1200).collect();
        let naming_prompt = format!(
            "请根据以下对话内容为这段对话起一个简短的标题（不超过 12 个字，与内容使用相同语言，不带标点、引号或换行）。只返回标题本身。\n\n<content>\n{snippet}\n</content>"
        );
        self.session_namings.insert(
            id,
            SessionNaming {
                runtime: ChatRuntime::start_text(provider, executable, naming_prompt, model),
                started: Instant::now(),
                parts: vec![],
                error: None,
            },
        );
    }
    /// Explicit "AI 重命名" from the session menu: drop any automatic naming in
    /// flight and regenerate from the latest exchange.
    fn ai_rename_session(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        let Some(excerpt) = self.session(id).and_then(latest_naming_excerpt) else {
            return;
        };
        if let Some(naming) = self.session_namings.remove(&id) {
            naming.runtime.cancel();
        }
        self.start_session_naming(id, &excerpt);
        cx.notify();
    }
    /// Daily session counts across every workspace for the last `weeks * 7`
    /// days, anchored to today's local date. Buckets use the local timezone so
    /// a session made just after midnight lands on the same day the user sees.
    fn session_activity(&self, weeks: usize) -> (chrono::NaiveDate, Vec<u32>) {
        let today = Local::now().date_naive();
        let days = weeks * 7;
        let start_day = today - chrono::Duration::days((days - 1) as i64);
        let mut counts = vec![0u32; days];
        for workspace in &self.store.workspaces {
            for session in &workspace.sessions {
                if session.created_at == 0 {
                    continue;
                }
                let Some(created) = DateTime::from_timestamp(session.created_at as i64, 0) else {
                    continue;
                };
                let day = created.with_timezone(&Local).date_naive();
                let offset = (day - start_day).num_days();
                if (0..days as i64).contains(&offset) {
                    counts[offset as usize] += 1;
                }
            }
        }
        (today, counts)
    }

    fn heatmap_cell_color(count: u32, max: u32) -> u32 {
        if count == 0 || max == 0 {
            return palette().surface;
        }
        let base = palette().surface_hover;
        let accent = palette().diff_add_text;
        let ratio = 0.25 + 0.75 * (count as f32 / max as f32);
        let mix = |a: u32, b: u32, t: f32| -> u8 { (a as f32 + (b as f32 - a as f32) * t) as u8 };
        let r = mix(base >> 16 & 0xff, accent >> 16 & 0xff, ratio);
        let g = mix(base >> 8 & 0xff, accent >> 8 & 0xff, ratio);
        let b = mix(base & 0xff, accent & 0xff, ratio);
        ((r as u32) << 16) | ((g as u32) << 8) | b as u32
    }

    /// Activity heatmap shown in place of the chat while no workspace is
    /// selected: 7 rows (weekdays) by trailing weeks, newest column last.
    fn session_heatmap(&self, weeks: usize) -> AnyElement {
        const WEEKDAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
        let (today, counts) = self.session_activity(weeks);
        let start_day = today - chrono::Duration::days((weeks * 7 - 1) as i64);
        let total: u32 = counts.iter().sum();
        let max = counts.iter().copied().max().unwrap_or(0);

        let mut grid = h_flex().gap(px(3.));
        for week in 0..weeks {
            let mut column = v_flex().gap(px(3.));
            for weekday in 0..7 {
                let index = week * 7 + weekday;
                let count = counts[index];
                let day = start_day + chrono::Duration::days(index as i64);
                let label: SharedString =
                    format!("{} · {} 次会话", day.format("%Y-%m-%d"), count).into();
                column = column.child(
                    div()
                        .id(("heatmap-cell", index))
                        .w(px(12.))
                        .h(px(12.))
                        .rounded(px(3.))
                        .bg(rgb(Self::heatmap_cell_color(count, max)))
                        .border_1()
                        .border_color(rgb(palette().border))
                        .when(day > today, |cell| cell.opacity(0.25))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                .build(window, cx)
                        }),
                );
            }
            grid = grid.child(column);
        }

        let mut weekday_labels = v_flex().gap(px(3.)).pt(px(0.));
        for weekday in WEEKDAYS {
            weekday_labels = weekday_labels.child(
                div()
                    .h(px(12.))
                    .text_size(px(9.))
                    .text_color(rgb(palette().faint))
                    .child(weekday),
            );
        }

        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_5()
            .py_20()
            .child(
                div()
                    .text_size(px(30.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child("连接一个本地工作区"),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(rgb(palette().muted))
                    .child("Conduit 直接调用本机 CLI，文件留在你的机器上。"),
            )
            .child(
                v_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(format!("近 {weeks} 周共 {total} 次会话")),
                    )
                    .child(h_flex().gap_2().child(weekday_labels).child(grid)),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child("CLI 只会在所选目录内执行任务"),
            )
            .into_any_element()
    }

    /// Shows the copied check mark on `id` for a short moment. Each call gets a
    /// fresh generation so a previous timer cannot hide newer feedback.
    fn show_copied(&mut self, id: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.copied_action_seq = self.copied_action_seq.wrapping_add(1);
        let seq = self.copied_action_seq;
        self.copied_action = Some(id);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.copied_action_seq == seq {
                    this.copied_action = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn render_message(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let started = Instant::now();
        let Some(session) = self
            .store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .find(|session| Some(session.id) == self.selected_session)
        else {
            return div().into_any_element();
        };
        let Some(item) = session.messages.get(index) else {
            return div().into_any_element();
        };
        let session_id = session.id;
        let provider = session.provider;
        let regenerable = item.kind == ItemKind::Assistant
            && self.writable
            && !self.closing
            && !self.runs.contains_key(&session_id)
            && session.messages[..index]
                .iter()
                .any(|message| message.kind == ItemKind::User)
            && !session.messages[index + 1..]
                .iter()
                .any(|message| matches!(message.kind, ItemKind::User | ItemKind::Assistant));
        let element = match item.kind {
            ItemKind::User => {
                let copy_text = item.text.clone();
                let copy_id = SharedString::from(format!("user-copy-{}-{}", session_id, item.key));
                let copied = self.copied_action.as_ref() == Some(&copy_id);
                let copy_icon = if copied {
                    Icon::new(IconName::Check).text_color(rgb(palette().diff_add_text))
                } else {
                    Icon::new(IconName::Copy)
                };
                let message_group =
                    SharedString::from(format!("user-message-{}-{}", session_id, item.key));
                v_flex()
                    .w_full()
                    .items_end()
                    .gap_1()
                    .group(message_group.clone())
                    .child(
                        div()
                            .id(("user-bubble", index))
                            .max_w(px(680.))
                            .rounded(px(18.))
                            .px_4()
                            .py_3()
                            .bg(rgb(palette().surface))
                            .text_size(px(14.))
                            .text_color(rgb(palette().text))
                            .child(gpui_kit::base::SelectableText::new(
                                SharedString::from(format!(
                                    "user-text-{}-{}",
                                    session_id, item.key
                                )),
                                item.text.clone(),
                            )),
                    )
                    .child(
                        h_flex()
                            .gap(px(2.))
                            .opacity(0.)
                            .group_hover(message_group, |this| this.opacity(1.))
                            .child(
                                message_action_button(
                                    copy_id.clone(),
                                    copy_icon,
                                    if copied { "已复制" } else { "复制消息" },
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_text.clone(),
                                        ));
                                        this.show_copied(copy_id.clone(), window, cx);
                                    },
                                )),
                            ),
                    )
                    .into_any_element()
            }
            ItemKind::Assistant => {
                let copy_text = item.text.clone();
                let copy_id = SharedString::from(format!("reply-copy-{}-{}", session_id, item.key));
                let copied = self.copied_action.as_ref() == Some(&copy_id);
                let copy_icon = if copied {
                    Icon::new(IconName::Check).text_color(rgb(palette().diff_add_text))
                } else {
                    Icon::new(IconName::Copy)
                };
                let message_group =
                    SharedString::from(format!("reply-message-{}-{}", session_id, item.key));
                let mut actions = h_flex()
                    .gap(px(2.))
                    .opacity(0.)
                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                    .child(
                        message_action_button(
                            copy_id.clone(),
                            copy_icon,
                            if copied { "已复制" } else { "复制消息" },
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                                this.show_copied(copy_id.clone(), window, cx);
                            },
                        )),
                    );
                if regenerable {
                    actions = actions.child(
                        message_action_button(
                            SharedString::from(format!(
                                "reply-regenerate-{}-{}",
                                session_id, item.key
                            )),
                            Icon::new(IconName::RefreshCw),
                            "重新生成",
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.regenerate(index, window, cx),
                        )),
                    );
                }
                v_flex()
                    .w_full()
                    .gap_1()
                    .group(message_group)
                    .child(
                        v_flex()
                            .w_full()
                            .gap_2()
                            .p_4()
                            .rounded(px(14.))
                            .border_1()
                            .border_color(rgb(palette().border))
                            .bg(rgb(palette().surface))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(provider_icon(provider))
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(rgb(palette().faint))
                                            .child(provider.label()),
                                    ),
                            )
                            .child(
                                TextView::markdown(
                                    SharedString::from(format!(
                                        "reply-{}-{}",
                                        session_id, item.key
                                    )),
                                    item.text.clone(),
                                )
                                .text_size(px(15.))
                                .selectable(true),
                            ),
                    )
                    .child(actions)
                    .into_any_element()
            }
            ItemKind::Tool => {
                let detail_key = (session_id, item.key.clone());
                let expanded = self.expanded_tools.contains(&detail_key);
                let tool_id = format!("tool-{}-{}", session_id, item.key);
                let header = Button::new(SharedString::from(format!("{tool_id}-toggle")))
                    .ghost()
                    .w_full()
                    .h(px(36.))
                    .px_3()
                    .cursor_pointer()
                    .accessibility_label(format!(
                        "{}工具调用：{}",
                        if expanded { "收起" } else { "展开" },
                        item.text
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .when_some(item.approval.as_ref(), |header, approval| {
                                header.child(
                                    div()
                                        .flex_shrink_0()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(5.))
                                        .bg(rgb(palette().surface_strong))
                                        .text_size(px(11.))
                                        .text_color(rgb(match approval.status.as_str() {
                                            "已允许" => palette().diff_add_text,
                                            "已拒绝" => palette().diff_remove_text,
                                            _ => palette().muted,
                                        }))
                                        .child(approval.status.clone()),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_left()
                                    .text_size(px(13.))
                                    .truncate()
                                    .child(item.text.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().faint))
                                    .child(item.status.clone()),
                            )
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .small()
                                .text_color(rgb(palette().faint)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.scroll.remeasure_items(index..index + 1);
                        if !this.expanded_tools.remove(&detail_key) {
                            this.expanded_tools.insert(detail_key.clone());
                        }
                        cx.notify();
                    }));
                let mut card = v_flex()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .child(header);
                if expanded {
                    let tool_scroll = self
                        .tool_scrolls
                        .entry((session_id, item.key.clone()))
                        .or_default()
                        .clone();
                    let dock_key = item.key.clone();
                    let (content, copy_text) = tool_content(&tool_id, item, &tool_scroll);
                    card = card.child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(
                                h_flex()
                                    .px_3()
                                    .pt_1()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette().faint))
                                            .child("工具调用内容"),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("{tool_id}-dock")))
                                            .ghost()
                                            .small()
                                            .compact()
                                            .icon(gpui_kit::assets::IconName::PanelRight)
                                            .tooltip("在右侧查看工具详情")
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.details = true;
                                                this.detail_view = DetailView::Tool;
                                                this.detail_text = Some(dock_key.clone());
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!("{tool_id}-copy")))
                                            .ghost()
                                            .small()
                                            .compact()
                                            .icon(gpui_kit::assets::IconName::Copy)
                                            .tooltip("复制输入与输出")
                                            .cursor_pointer()
                                            .on_click(move |_, _, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy_text.clone(),
                                                ))
                                            }),
                                    ),
                            )
                            .child(content),
                    );
                }
                card.into_any_element()
            }
            ItemKind::Notice => div()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .child(item.text.clone())
                .into_any_element(),
            ItemKind::Error => Alert::error(
                SharedString::from(format!("error-{}-{}", session_id, item.key)),
                item.text.clone(),
            )
            .banner()
            .into_any_element(),
        };
        let highlighted = self
            .message_focus
            .as_ref()
            .is_some_and(|(id, key, _)| *id == session_id && key == &item.key);
        let row = div().w_full().px_6().pb_5().child(
            div()
                .id(SharedString::from(format!(
                    "message-{}-{}",
                    session_id, item.key
                )))
                .w_full()
                .min_w_0()
                .when(!self.store.ui.full_width_content, |card| {
                    card.max_w(px(840.)).mx_auto()
                })
                .rounded(px(12.))
                .border_1()
                .border_color(if highlighted {
                    rgb(palette().focus).into()
                } else {
                    transparent_black()
                })
                .when(highlighted, |row| row.bg(rgb(palette().surface_hover)))
                .child(element),
        );
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.record_message_render(index, started.elapsed());
            });
        }
        row.into_any_element()
    }
    fn scroll_to_end(&self) {
        self.scroll.scroll_to_end();
    }
    fn jump_to_message(&mut self, session: u64, key: &str, cx: &mut Context<Self>) {
        if self.selected_session != Some(session) {
            return;
        }
        let Some(index) = self.session(session).and_then(|session| {
            session
                .messages
                .iter()
                .position(|item| item.kind == ItemKind::User && item.key == key)
        }) else {
            return;
        };
        self.scroll.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: px(0.),
        });
        self.at_bottom = false;
        self.pending_message_jump = true;
        self.message_focus = Some((session, key.into(), Instant::now()));
        cx.notify();
    }
    fn message_outline(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let session_id = self.outline.session.unwrap_or_default();
        let entries = &self.outline.entries;
        let top = self.scroll.logical_scroll_top().item_ix;
        let current = entries
            .partition_point(|entry| entry.index <= top)
            .saturating_sub(1);
        if self.outline.active != Some(current) {
            self.outline.active = Some(current);
            self.outline
                .scroll
                .scroll_to_item(current, ScrollStrategy::Nearest);
        }
        let app = cx.entity();
        let rail = uniform_list(
            ("message-outline", session_id),
            entries.len(),
            move |range, _, cx| {
                app.update(cx, |this, cx| {
                    range
                        .map(|ordinal| {
                            let entry = &this.outline.entries[ordinal];
                            let active = current == ordinal;
                            let marker_width = outline_marker_width(&entry.label);
                            let key = entry.key.clone();
                            Button::new(SharedString::from(format!("outline-{session_id}-{key}")))
                                .ghost()
                                .w_full()
                                .h(px(26.))
                                .p_0()
                                .flex_shrink_0()
                                .cursor_pointer()
                                // A short, fixed-length tooltip keeps the
                                // popup narrow enough to always fit on the
                                // left; longer text would flip to the right.
                                .tooltip(outline_tooltip(&entry.label))
                                .tooltip_placement(Placement::Left)
                                .accessibility_label(format!(
                                    "跳转到用户消息 {}：{}",
                                    ordinal + 1,
                                    entry.label
                                ))
                                .child(
                                    h_flex().w_full().justify_end().child(
                                        div()
                                            .w(px(if active {
                                                (marker_width + 2.).min(26.)
                                            } else {
                                                marker_width
                                            }))
                                            .h(px(3.))
                                            .rounded_full()
                                            .bg(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().faint
                                            })),
                                    ),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.jump_to_message(session_id, &key, cx);
                                }))
                        })
                        .collect::<Vec<_>>()
                })
            },
        )
        .w_full()
        .h(px(entries.len() as f32 * 26.))
        .max_h(relative(1.))
        .min_h_0()
        .track_scroll(&self.outline.scroll);
        v_flex()
            .w(px(32.))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .py_4()
            .justify_center()
            .child(rail)
            .into_any_element()
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
                                .bg(rgb(palette().surface_hover))
                                .text_size(px(10.))
                                .text_color(rgb(palette().text))
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
        // Git settings can pin the generation CLI & model; by default both
        // follow the currently selected session.
        let provider = self
            .store
            .git
            .generation_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let model = self.store.git.generation_model.clone().or_else(|| {
            self.selected_session
                .and_then(|id| self.session(id))
                .and_then(|session| session.model.clone())
        });
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
    /// Drain naming requests; apply a finished title only when the session
    /// still shows its prompt-derived placeholder (i.e. the user has not
    /// renamed it meanwhile).
    fn poll_session_namings(&mut self, cx: &mut Context<Self>) {
        let mut finished = vec![];
        for (id, naming) in &mut self.session_namings {
            if naming.started.elapsed() > Duration::from_secs(60) && naming.error.is_none() {
                naming.error = Some("timeout".into());
                naming.runtime.cancel();
            }
            loop {
                match naming.runtime.events.try_recv() {
                    Ok(Event::Text { key, text }) => {
                        update_commit_text(&mut naming.parts, key, text, true);
                    }
                    Ok(Event::Delta { key, text }) => {
                        update_commit_text(&mut naming.parts, key, text, false);
                    }
                    Ok(Event::Approval(approval)) => {
                        naming.runtime.decide(approval.id, false);
                        naming.error = Some("approval".into());
                        naming.runtime.cancel();
                    }
                    Ok(Event::Tool { .. }) | Ok(Event::ToolInput { .. }) => {
                        naming.error = Some("tool".into());
                        naming.runtime.cancel();
                    }
                    Ok(Event::Finished { .. }) => {
                        finished.push(*id);
                        break;
                    }
                    Ok(_) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished.push(*id);
                        break;
                    }
                }
            }
        }
        for id in finished {
            let Some(naming) = self.session_namings.remove(&id) else {
                continue;
            };
            if naming.error.is_some() {
                continue;
            }
            let title = naming
                .parts
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("")
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .trim_matches(|ch: char| {
                    ch == '"'
                        || ch == '\u{201c}'
                        || ch == '\u{201d}'
                        || ch == '\u{300c}'
                        || ch == '\u{300d}'
                        || ch == '\''
                })
                .trim_end_matches(|ch: char| {
                    ch == '.' || ch == '\u{3002}' || ch == '!' || ch == '\u{ff01}' || ch == '?'
                })
                .chars()
                .take(30)
                .collect::<String>();
            if title.is_empty() {
                continue;
            }
            if let Some(session) = self.session_mut(id)
                && session.messages.iter().any(|m| m.kind == ItemKind::User)
            {
                session.title = title;
                self.persist();
                cx.notify();
            }
        }
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
            .border_color(rgb(palette().border))
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
                    .bg(rgb(palette().surface))
                    .border_1()
                    .border_color(rgb(palette().border))
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
                    .text_color(rgb(palette().faint))
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
                    .text_color(rgb(if feedback.error {
                        palette().error_text
                    } else {
                        palette().muted
                    }))
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
                            .text_color(rgb(palette().faint)),
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
                        .text_color(rgb(palette().faint))
                        .child("工作区分支 · 只读"),
                );
            if repository.detached {
                list = list.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
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
                        .text_color(rgb(palette().faint))
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
                            .when(current, |row| row.bg(rgb(palette().surface_hover)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().text))
                                    .child(branch.name.clone()),
                            )
                            .when(current, |row| {
                                row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().muted))
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
                        .text_color(rgb(palette().faint))
                        .child("分支过多，列表已截断"),
                );
            }
            list.child(
                h_flex()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(palette().border))
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
                            .text_color(rgb(palette().muted))
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
                        .border_color(rgb(palette().danger_border))
                        .bg(rgb(palette().danger_bg))
                        .text_size(px(11.))
                        .line_height(relative(1.4))
                        .text_color(rgb(palette().danger_text))
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
                        .text_color(rgb(palette().faint))
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
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(snapshot.branch.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(9.))
                            .text_color(rgb(palette().faint))
                            .child("变更预览"),
                    )
                    .child(refresh("刷新", "git-refresh")),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(palette().faint))
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
                    .text_color(rgb(palette().muted))
                    .child("工作区没有未提交变更"),
            );
        } else {
            let mut list = v_flex().w_full().gap(px(1.));
            for file in &snapshot.files {
                let color = match file.label() {
                    "新增" => palette().diff_add_text,
                    "删除" => palette().diff_remove_text,
                    "冲突" => palette().warning_text,
                    "未跟踪" => palette().faint,
                    _ => palette().muted,
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
                                .text_color(rgb(palette().text))
                                .child(file.path.clone()),
                        )
                        .when_some(file.original.clone(), |row, original| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(9.))
                                    .text_color(rgb(palette().faint))
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
                                .text_color(rgb(palette().muted))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(9.))
                                .text_color(rgb(palette().faint))
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
                    .text_color(rgb(palette().faint))
                    .child(note.clone()),
            );
        }
        body.into_any_element()
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.ui.performance_monitor && self.performance.read(cx).sample_due() {
            // Notify only the diagnostics view; idle chats need no extra rebuild.
            self.performance.update(cx, |monitor, cx| {
                monitor.sample();
                cx.notify();
            });
        }
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
        self.poll_session_namings(cx);
        if self
            .message_focus
            .as_ref()
            .is_some_and(|(_, _, started)| started.elapsed() > Duration::from_secs(2))
        {
            self.message_focus = None;
            cx.notify();
        }
        let navigating = std::mem::take(&mut self.pending_message_jump);
        let follow = self.scroll.is_scrolled_to_end().unwrap_or_else(|| {
            self.scroll.logical_scroll_top().item_ix >= self.scroll.item_count()
        });
        // Re-render when the scroll position crosses the "at the end" threshold
        // so the floating arrow appears/disappears without polling every frame.
        if !navigating && follow != self.at_bottom {
            self.at_bottom = follow;
            cx.notify();
        }
        let mut incoming = vec![];
        for (id, run) in &self.runs {
            for event in run.runtime.events.try_iter().take(1000) {
                incoming.push((*id, run.prefix.clone(), event));
            }
        }
        if self.store.ui.performance_monitor {
            self.performance
                .update(cx, |monitor, _| monitor.record_events(incoming.len()));
        }
        let mut changed = false;
        let mut terminal = false;
        let mut visible = false;
        let mut dirty_messages = Vec::new();
        let mut structural_change = false;
        let mut completed_turns = Vec::new();
        for (id, prefix, event) in incoming {
            changed = true;
            visible |= Some(id) == self.selected_session;
            if Some(id) == self.selected_session
                && matches!(
                    event,
                    Event::Approval(_)
                        | Event::ApprovalCancelled(_)
                        | Event::ApprovalResolved { .. }
                        | Event::Finished { .. }
                )
            {
                structural_change = true;
            }
            match event {
                Event::Usage {
                    input_tokens,
                    cached_input_tokens,
                    output_tokens,
                    total_tokens,
                    context_window,
                } => {
                    // Usage is display-only state: update both the running turn
                    // and the session so the value survives run completion.
                    let now = Instant::now();
                    if let Some(run) = self.runs.get_mut(&id) {
                        // A falling total means a new turn or compaction reset;
                        // re-anchor the speed baseline.
                        if total_tokens < run.turn_tokens_at_start {
                            run.turn_tokens_at_start = total_tokens;
                            run.turn_started = now;
                        }
                        run.usage = Usage {
                            input_tokens,
                            cached_input_tokens,
                            output_tokens,
                            total_tokens,
                            context_window,
                        };
                        run.last_usage_event = now;
                    }
                    if let Some(session) = self.session_mut(id) {
                        session.usage = Usage {
                            input_tokens,
                            cached_input_tokens,
                            output_tokens,
                            total_tokens,
                            context_window,
                        };
                    }
                }
                Event::Approval(approval) => {
                    let accepted = self
                        .runs
                        .get(&id)
                        .is_some_and(|run| !self.closing && !run.runtime.is_cancelled());
                    if !accepted {
                        continue;
                    }
                    if let Some(session) = self.session_mut(id) {
                        session.record_tool_approval(
                            &prefix,
                            &approval.id,
                            &approval.title,
                            &approval.detail,
                            "等待中",
                        );
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.status = "等待你的审批".into();
                        run.activity = "等待你的审批".into();
                        run.approvals.push(approval);
                    }
                }
                Event::ApprovalCancelled(request) => {
                    self.approval_details.remove(&request);
                    let mut approval = None;
                    if let Some(run) = self.runs.get_mut(&id) {
                        approval = run.approvals.remove(&request);
                        if run.approvals.is_empty() && !run.runtime.is_cancelled() {
                            run.status = "正在执行…".into();
                            run.activity = "正在继续处理…".into();
                        }
                    }
                    if let (Some(approval), Some(session)) = (approval, self.session_mut(id)) {
                        session.record_tool_approval(
                            &prefix,
                            &request,
                            &approval.title,
                            &approval.detail,
                            "已取消",
                        );
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
                        run.activity = if run.approvals.is_empty() {
                            "正在继续处理…"
                        } else {
                            "等待你的审批"
                        }
                        .into();
                        approval
                    });
                    if let (Some(approval), Some(session)) = (approval, self.session_mut(id)) {
                        session.record_tool_approval(
                            &prefix,
                            &request,
                            &approval.title,
                            &approval.detail,
                            if allow { "已允许" } else { "已拒绝" },
                        );
                    }
                }
                Event::Finished { error, cancelled } => {
                    self.runs.remove(&id);
                    terminal = true;
                    if error.is_none() && !cancelled {
                        completed_turns.push(id);
                    }
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
                                    format!("执行失败：{}", friendly_failure(&error))
                                } else {
                                    "本轮已完成".into()
                                },
                                input: String::new(),
                                detail: String::new(),
                                status: String::new(),
                                approval: None,
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
                        run.activity = "已连接 CLI，正在等待模型响应…".into();
                    }
                    self.dirty = true;
                    self.persist();
                }
                other => {
                    if let Some(run) = self.runs.get_mut(&id) {
                        let activity = match &other {
                            Event::Delta { .. } | Event::Text { .. } => "正在生成回复…".to_string(),
                            Event::ToolInput { .. } => "正在准备工具调用…".to_string(),
                            Event::Tool { title, status, .. } => match status.as_str() {
                                "运行中" if !title.is_empty() => {
                                    format!("正在调用工具：{title}")
                                }
                                "运行中" => "正在调用工具…".to_string(),
                                "输出中" => "正在接收工具输出…".to_string(),
                                _ => "正在继续处理…".to_string(),
                            },
                            _ => String::new(),
                        };
                        if !activity.is_empty() {
                            run.activity = activity;
                        }
                    }
                    let mut index = self
                        .runs
                        .get_mut(&id)
                        .map(|run| std::mem::take(&mut run.message_index))
                        .unwrap_or_default();
                    let selected = Some(id) == self.selected_session;
                    if let Some(session) = self.session_mut(id)
                        && let Some(changed_index) =
                            apply_event_indexed(session, &prefix, other, &mut index)
                        && selected
                    {
                        dirty_messages.push(changed_index);
                    }
                    if let Some(run) = self.runs.get_mut(&id) {
                        run.message_index = index;
                    }
                }
            }
        }
        if self.store.ui.naming_mode == NamingMode::Latest {
            for id in completed_turns {
                if let Some(excerpt) = self.session(id).and_then(latest_naming_excerpt) {
                    self.start_session_naming(id, &excerpt);
                }
            }
        }
        if changed {
            if structural_change {
                self.scroll.remeasure();
            } else {
                dirty_messages.sort_unstable();
                dirty_messages.dedup();
                for index in dirty_messages {
                    if index < self.scroll.item_count() {
                        self.scroll.remeasure_items(index..index + 1);
                    }
                }
            }
            self.dirty = true;
            if terminal || self.last_save.elapsed() > Duration::from_secs(1) {
                self.persist();
            }
            cx.notify();
            if follow && visible && !navigating && self.message_focus.is_none() {
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
            prompt: Some(format!("选择 {} 可执行文件（.exe 或 .cmd）", provider.label()).into()),
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
                    .child(
                        Icon::new(IconName::Cpu)
                            .small()
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
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
                                .text_color(rgb(palette().text))
                                .child("会话模型"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child(format!("{provider_label} · {model_count} 个")),
                        ),
                );
                let option = |index: usize,
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
                        .when(selected, |button| button.bg(rgb(palette().surface_hover)))
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
                                                    palette().text
                                                } else {
                                                    palette().muted
                                                }))
                                                .child(label),
                                        )
                                        .when(!note.is_empty(), |column| {
                                            column.child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(note),
                                            )
                                        }),
                                )
                                .child(Icon::new(IconName::Check).small().text_color(rgb(
                                    if selected {
                                        palette().text
                                    } else {
                                        palette().surface_hover
                                    },
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
                            .text_color(rgb(palette().faint))
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
                            .text_color(rgb(palette().faint))
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
                        .border_color(rgb(palette().border))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child("仅影响本会话，下一条消息生效"),
                        )
                        .child(
                            h_flex()
                                .id("model-refresh")
                                .cursor_pointer()
                                .rounded(px(4.))
                                .px_1()
                                .text_size(px(10.))
                                .text_color(rgb(palette().muted))
                                .hover(|style| style.bg(rgb(palette().surface_hover)))
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
                .bg(rgb(palette().surface))
                .border_color(rgb(palette().border))
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
                .text_color(rgb(palette().faint))
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
            .border_color(rgb(palette().border))
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
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("可执行文件"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                            .text_color(rgb(palette().muted))
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
                    .border_color(rgb(palette().border))
                    .child(label("本次启动"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
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
                    .border_color(rgb(palette().border))
                    .child(label("状态"))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
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
                        .text_color(rgb(palette().text))
                        .child(model.label.clone()),
                );
                if show_id {
                    value = value.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                        .border_color(rgb(palette().border))
                        .child(value)
                        .when(!model.note.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child(model.note.clone()),
                            )
                        })
                        .when(is_current, |row| {
                            row.child(
                                div()
                                    .px_2()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(palette().surface_hover))
                                    .text_size(px(9.))
                                    .text_color(rgb(palette().muted))
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
                        .border_color(rgb(palette().border))
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
        }
        let sync_row = |label: &'static str, value: String, divider: bool| -> AnyElement {
            let mut row = h_flex().px_4().py_3().gap_4().items_center();
            if divider {
                row = row.border_t_1().border_color(rgb(palette().border));
            }
            row.child(settings_label(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
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
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("当前模型"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(palette().surface_hover))
                            .text_size(px(9.))
                            .text_color(rgb(palette().faint))
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
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                );
            } else if let Some(note) = &catalog.note {
                model_card = model_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
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
                    .text_color(rgb(palette().faint))
                    .child("版本检测只运行 --version，不调用模型，也不验证登录状态；请先在官方 CLI 中完成登录。"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("此处只显示 CLI 当前模型；完整来源与 CCS 同步见左侧「模型同步」。不读取或显示密钥，也不会写回 CLI 配置。"),
            )
            .into_any_element()
    }
    fn settings_models_content(&self, view: Entity<Self>) -> AnyElement {
        let refresh_view = view.clone();
        let source_row = |label: &'static str, value: String, divider: bool| -> AnyElement {
            let mut row = h_flex().px_4().py_3().gap_4().items_center();
            if divider {
                row = row.border_t_1().border_color(rgb(palette().border));
            }
            row.child(settings_label(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
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
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("模型来源"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(palette().surface_hover))
                            .text_size(px(9.))
                            .text_color(rgb(palette().faint))
                            .child("只读"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                    .border_color(rgb(palette().border))
                    .text_size(px(10.))
                    .text_color(rgb(if self.ccs.error.is_some() {
                        palette().danger_text
                    } else {
                        palette().faint
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
                    .text_color(rgb(palette().faint))
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
                .bg(rgb(palette().surface_hover))
                .text_size(px(9.))
                .text_color(rgb(palette().muted))
                .child(text)
                .into_any_element()
        };
        let model_row =
            |label: String, id: String, note: String, tag: Option<&'static str>| -> AnyElement {
                let mut value = v_flex().flex_1().min_w_0().gap(px(1.)).child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(palette().text))
                        .child(label.clone()),
                );
                if label != id {
                    value = value.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(id),
                    );
                }
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(value)
                    .when(!note.is_empty(), |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
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
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("当前 CLI 模型"),
                    )
                    .child(badge("CLI"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                            .text_color(rgb(palette().muted))
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
                        .border_color(rgb(palette().border))
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
            if let Some(error) = &catalog.error {
                current_card = current_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                );
            } else if let Some(note) = &catalog.note {
                current_card = current_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
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
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("CCS 同步"),
                    )
                    .child(badge("只读"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(entry.name.clone()),
                    )
                    .when(entry.current, |row| row.child(badge("当前 Provider")))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                    .border_color(rgb(palette().border))
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
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
                            .text_color(rgb(palette().text))
                            .child(provider.label()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!("当前默认 {current}")),
                    ),
            )
            .child(current_card)
            .child(ccs_card)
            .into_any_element()
    }
    /// Model catalog merged with cc-switch synced models, for the Git
    /// generation model picker.
    fn git_generation_catalog(&self, provider: Provider) -> ModelCatalog {
        let catalog = self.catalogs.get(&provider);
        let synced = self.ccs.current(provider);
        match (catalog, synced) {
            (Some(catalog), Some(synced)) if !synced.models.is_empty() => {
                catalog.with_synced(&format!("CCS · {}", synced.name), &synced.models)
            }
            (Some(catalog), _) => catalog.clone(),
            (None, Some(synced)) if !synced.models.is_empty() => ModelCatalog::default()
                .with_synced(&format!("CCS · {}", synced.name), &synced.models),
            (None, _) => ModelCatalog::default(),
        }
    }
    /// Picker for the CLI engine used to auto-name sessions.
    fn naming_provider_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.ui.naming_provider;
        let label = current
            .map(|provider| provider.label())
            .unwrap_or("跟随当前会话");
        let trigger = Button::new("naming-provider-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择命名引擎")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when_some(current, |row, provider| row.child(provider_icon(provider)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-provider-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                let options: [(Option<Provider>, &str); 3] = [
                    (None, "跟随当前会话"),
                    (Some(Provider::Claude), Provider::Claude.label()),
                    (Some(Provider::Codex), Provider::Codex.label()),
                ];
                for (index, (option, name)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == option;
                    list = list.child(
                        Button::new(("naming-provider-option", index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_provider = option;
                                    // A model picked for another engine no longer applies.
                                    if option.is_some() {
                                        this.store.ui.naming_model = None;
                                    }
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .when_some(option, |row, provider| {
                                        row.child(provider_icon(provider))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.))
                                            .text_color(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().muted
                                            }))
                                            .child(name),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }
    /// Picker for what automatic session titling reacts to.
    fn naming_mode_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.ui.naming_mode;
        let label = naming_mode_label(current);
        let trigger = Button::new("naming-mode-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择自动重命名方式")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-mode-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(240.)).gap(px(1.));
                for (index, (mode, name, detail)) in [
                    (
                        NamingMode::Latest,
                        "自动根据最新内容重命名",
                        "每轮结束后按最新对话更新标题",
                    ),
                    (
                        NamingMode::FirstMessage,
                        "根据第一条消息内容重命名",
                        "只在第一条消息后生成一次标题",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == mode;
                    list = list.child(
                        Button::new(("naming-mode-option", index))
                            .ghost()
                            .h(px(44.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_mode = mode;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
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
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(if active {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(name),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(detail),
                                            ),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }
    /// Picker for the model used to auto-name sessions.
    fn naming_model_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let follow_session = self.store.ui.naming_provider.is_none();
        let provider = self
            .store
            .ui
            .naming_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let merged = self.git_generation_catalog(provider);
        let selected_model = self.store.ui.naming_model.clone();
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| {
                if follow_session {
                    "跟随当前会话".to_string()
                } else {
                    "CLI 默认".to_string()
                }
            });
        let models = merged.models;
        let provider_label = provider.label();
        let trigger = Button::new("naming-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择命名模型")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(IconName::Cpu)
                            .small()
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("naming-model-list")
                    .w(px(320.))
                    .gap(px(1.))
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let default_label = if follow_session {
                    "跟随当前会话"
                } else {
                    "CLI 默认"
                };
                let options: Vec<(Option<String>, String, String)> =
                    std::iter::once((None, default_label.to_string(), String::new()))
                        .chain(models.iter().map(|model| {
                            (
                                Some(model.id.clone()),
                                model.label.clone(),
                                model.note.clone(),
                            )
                        }))
                        .collect();
                for (index, (id, label, note)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let selected = selected_model == id;
                    list = list.child(
                        Button::new(("naming-model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(format!("模型 {label}"))
                            .when(selected, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_model = id;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
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
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(10.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .when(selected, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(format!("{provider_label} · {} 个模型", models.len())),
                );
                list
            })
            .into_any_element()
    }
    /// Settings picker for the CLI used to generate commit messages.
    fn git_provider_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.git.generation_provider;
        let label = current
            .map(|provider| provider.label())
            .unwrap_or("跟随当前会话");
        let trigger = Button::new("git-provider-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择生成用 CLI")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when_some(current, |row, provider| row.child(provider_icon(provider)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("git-provider-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                let options: [(Option<Provider>, &str); 3] = [
                    (None, "跟随当前会话"),
                    (Some(Provider::Claude), Provider::Claude.label()),
                    (Some(Provider::Codex), Provider::Codex.label()),
                ];
                for (index, (option, name)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == option;
                    list = list.child(
                        Button::new(("git-provider-option", index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.generation_provider = option;
                                    // A model picked for another engine no longer applies.
                                    if option.is_some() {
                                        this.store.git.generation_model = None;
                                    }
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .when_some(option, |row, provider| {
                                        row.child(provider_icon(provider))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.))
                                            .text_color(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().muted
                                            }))
                                            .child(name),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }
    /// Settings picker for the model used to generate commit messages.
    fn git_model_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let provider = self
            .store
            .git
            .generation_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let merged = self.git_generation_catalog(provider);
        let selected_model = self.store.git.generation_model.clone();
        let follow_session = self.store.git.generation_provider.is_none();
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| {
                if follow_session {
                    "跟随当前会话".to_string()
                } else {
                    "CLI 默认".to_string()
                }
            });
        let models = merged.models;
        let provider_label = provider.label();
        let trigger = Button::new("git-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择生成用模型")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(IconName::Cpu)
                            .small()
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("git-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("git-model-list")
                    .w(px(320.))
                    .gap(px(1.))
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let default_label = if follow_session {
                    "跟随当前会话"
                } else {
                    "CLI 默认"
                };
                let options: Vec<(Option<String>, String, String)> =
                    std::iter::once((None, default_label.to_string(), String::new()))
                        .chain(models.iter().map(|model| {
                            (
                                Some(model.id.clone()),
                                model.label.clone(),
                                model.note.clone(),
                            )
                        }))
                        .collect();
                for (index, (id, label, note)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let selected = selected_model == id;
                    list = list.child(
                        Button::new(("git-model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(format!("模型 {label}"))
                            .when(selected, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.generation_model = id;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
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
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(10.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .when(selected, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(format!("{provider_label} · {} 个模型", models.len())),
                );
                list
            })
            .into_any_element()
    }
    fn settings_session_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let provider_picker = self.naming_provider_picker(view.clone(), disabled);
        let model_picker = self.naming_model_picker(view.clone(), disabled);
        let mode_picker = self.naming_mode_picker(view.clone(), disabled);
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("会话命名"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("立即生效"),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("命名引擎"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("生成会话标题使用的引擎；默认跟随当前会话"),
                            )
                            .child(provider_picker),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(settings_label("命名模型"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("指定引擎后可固定模型；默认跟随当前会话的模型"),
                            )
                            .child(model_picker),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(settings_label("自动重命名"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("选择标题跟随最新对话自动更新，或只按第一条消息生成一次"),
                            )
                            .child(mode_picker),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("命名在后台进行，失败时保留当前标题；命名完成前手动重命名的会话不会被覆盖。右键会话可随时使用 AI 重命名。"),
            )
            .into_any_element()
    }
    fn settings_git_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let provider_picker = self.git_provider_picker(view.clone(), disabled);
        let model_picker = self.git_model_picker(view.clone(), disabled);
        v_flex()
            .w_full()
            .gap_3()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("生成引擎"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("立即生效"),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("生成 CLI"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("用于生成 Commit Message 的 CLI；默认跟随当前会话"),
                            )
                            .child(provider_picker),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(settings_label("生成模型"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("指定 CLI 后可固定模型；默认跟随当前会话（或 CLI 默认）"),
                            )
                            .child(model_picker),
                    ),
            )
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
                div().text_size(px(12.)).text_color(rgb(palette().muted))
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
                div().text_size(px(11.)).text_color(rgb(palette().faint))
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
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
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
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("诊断"),
                            ),
                    )
                    .child({
                        let toggle = view.clone();
                        let disabled = !self.writable || self.closing;
                        let enabled = self.store.ui.performance_monitor;
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("性能监测"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("在底部工具栏显示性能入口并采集指标"),
                            )
                            .child(
                                Switch::new("performance-monitor-setting")
                                    .checked(enabled)
                                    .disabled(disabled)
                                    .on_change(move |checked, _, cx| {
                                        toggle.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.performance_monitor = *checked;
                                            if *checked {
                                                this.performance
                                                    .update(cx, |monitor, _| monitor.reset());
                                            } else {
                                                this.performance_open = false;
                                            }
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                    }),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("配置只保存在本机；外观与诊断设置随本地状态保存。"),
            )
            .into_any_element()
    }
    fn settings_appearance_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let selected = self.store.ui.appearance;
        let theme_option = |mode: AppearanceMode,
                            label: &'static str,
                            detail: &'static str,
                            icon: gpui_kit::assets::IconName,
                            id: &'static str|
         -> AnyElement {
            let view = view.clone();
            let active = selected == mode;
            let label_color = if disabled {
                palette().faint
            } else {
                palette().text
            };
            let icon_color = if disabled {
                palette().faint
            } else if active {
                palette().text
            } else {
                palette().muted
            };
            let check_color = if disabled {
                palette().faint
            } else {
                palette().text
            };
            Button::new(id)
                .ghost()
                .h(px(54.))
                .w_full()
                .px_3()
                .rounded(px(8.))
                .selected(active)
                .accessibility_label(label)
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        if !this.writable || this.closing {
                            return;
                        }
                        if this.store.ui.appearance != mode {
                            this.store.ui.appearance = mode;
                            this.persist();
                        }
                        this.apply_appearance(window, cx);
                    });
                })
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .child(Icon::new(icon).small().text_color(rgb(icon_color)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_medium()
                                        .text_color(rgb(label_color))
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(detail),
                                ),
                        )
                        .when(active, |row| {
                            row.child(
                                Icon::new(gpui_kit::assets::IconName::Check)
                                    .small()
                                    .text_color(rgb(check_color)),
                            )
                        }),
                )
                .into_any_element()
        };
        let value = self.store.ui.sessions_per_workspace.clamp(1, 30);
        let decrease = view.clone();
        let increase = view.clone();
        let max_tabs = self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT);
        let max_tabs_limit = MAX_TAB_LIMIT;
        let tabs_decrease = view.clone();
        let tabs_increase = view.clone();
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("主题"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("立即生效"),
                            ),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .p_2()
                            .gap_1()
                            .child(theme_option(
                                AppearanceMode::System,
                                "跟随系统",
                                "随 Windows 的浅色 / 深色外观自动切换",
                                gpui_kit::assets::IconName::Monitor,
                                "appearance-theme-system",
                            ))
                            .child(theme_option(
                                AppearanceMode::Light,
                                "浅色",
                                "始终使用浅色背景与深色文字",
                                gpui_kit::assets::IconName::Sun,
                                "appearance-theme-light",
                            ))
                            .child(theme_option(
                                AppearanceMode::Dark,
                                "深色",
                                "始终使用深色背景与浅色文字",
                                gpui_kit::assets::IconName::Moon,
                                "appearance-theme-dark",
                            )),
                    ),
            )
            .child({
                let toggle = view.clone();
                let full_width = self.store.ui.full_width_content;
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("内容区域"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("立即生效"),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("内容铺满"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("关闭时右侧内容以 840px 居中显示；开启后铺满整个内容区"),
                            )
                            .child(
                                Switch::new("full-width-content-setting")
                                    .checked(full_width)
                                    .disabled(disabled)
                                    .on_change(move |checked, _, cx| {
                                        toggle.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.full_width_content = *checked;
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
            })
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("侧栏"),
                            ),
                    )
                    .child(
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
                                    .text_color(rgb(palette().muted))
                                    .child("默认 5 条，范围 1–30，超出时侧栏可展开"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        Button::new("sessions-limit-minus")
                                            .label("−")
                                            .accessibility_label("减少工作区会话数")
                                            .small()
                                            .ghost()
                                            .disabled(disabled || value <= 1)
                                            .when(!disabled && value > 1, |button| {
                                                button.cursor_pointer()
                                            })
                                            .on_click(move |_, _, cx| {
                                                decrease.update(cx, |this, cx| {
                                                    if !this.writable || this.closing {
                                                        return;
                                                    }
                                                    this.store.ui.sessions_per_workspace = this
                                                        .store
                                                        .ui
                                                        .sessions_per_workspace
                                                        .clamp(1, 30)
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
                                            .text_color(rgb(if disabled {
                                                palette().faint
                                            } else {
                                                palette().text
                                            }))
                                            .child(value.to_string()),
                                    )
                                    .child(
                                        Button::new("sessions-limit-plus")
                                            .label("+")
                                            .accessibility_label("增加工作区会话数")
                                            .small()
                                            .ghost()
                                            .disabled(disabled || value >= 30)
                                            .when(!disabled && value < 30, |button| {
                                                button.cursor_pointer()
                                            })
                                            .on_click(move |_, _, cx| {
                                                increase.update(cx, |this, cx| {
                                                    if !this.writable || this.closing {
                                                        return;
                                                    }
                                                    this.store.ui.sessions_per_workspace = (this
                                                        .store
                                                        .ui
                                                        .sessions_per_workspace
                                                        .clamp(1, 30)
                                                        + 1)
                                                    .min(30);
                                                    this.persist();
                                                    cx.notify();
                                                });
                                            }),
                                    ),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("标签页"),
                            ),
                    )
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .gap_4()
                            .items_center()
                            .child(settings_label("最大 tab 数量"))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("默认 8 个，范围 1–20；超过数量自动关闭最老的标签"),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        Button::new("max-tabs-minus")
                                            .label("−")
                                            .accessibility_label("减少最大标签数量")
                                            .small()
                                            .ghost()
                                            .disabled(disabled || max_tabs <= 1)
                                            .when(!disabled && max_tabs > 1, |button| {
                                                button.cursor_pointer()
                                            })
                                            .on_click(move |_, _, cx| {
                                                tabs_decrease.update(cx, |this, cx| {
                                                    if !this.writable || this.closing {
                                                        return;
                                                    }
                                                    this.store.ui.max_tabs = this
                                                        .store
                                                        .ui
                                                        .max_tabs
                                                        .clamp(1, max_tabs_limit)
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
                                            .text_color(rgb(if disabled {
                                                palette().faint
                                            } else {
                                                palette().text
                                            }))
                                            .child(max_tabs.to_string()),
                                    )
                                    .child(
                                        Button::new("max-tabs-plus")
                                            .label("+")
                                            .accessibility_label("增加最大标签数量")
                                            .small()
                                            .ghost()
                                            .disabled(disabled || max_tabs >= max_tabs_limit)
                                            .when(
                                                !disabled && max_tabs < max_tabs_limit,
                                                |button| button.cursor_pointer(),
                                            )
                                            .on_click(move |_, _, cx| {
                                                tabs_increase.update(cx, |this, cx| {
                                                    if !this.writable || this.closing {
                                                        return;
                                                    }
                                                    this.store.ui.max_tabs = (this
                                                        .store
                                                        .ui
                                                        .max_tabs
                                                        .clamp(1, max_tabs_limit)
                                                        + 1)
                                                    .min(max_tabs_limit);
                                                    this.persist();
                                                    cx.notify();
                                                });
                                            }),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("主题切换会立即应用并保存；跟随系统时，Windows 明暗变化会自动同步。"),
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
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("路径查找顺序"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
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
                    .child(settings_row(
                        "PATH",
                        "claude.exe / .cmd · codex.exe / .cmd".into(),
                        true,
                    ))
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
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("可执行文件与版本"),
                            ),
                    )
                    .child(settings_row(
                        "Claude Code",
                        "在左侧「Claude Code」页面选择本机 .exe / .cmd 并检测版本".into(),
                        false,
                    ))
                    .child(settings_row(
                        "Codex",
                        "在左侧「Codex」页面选择本机 .exe / .cmd 并检测版本".into(),
                        true,
                    )),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("修改 CLI 路径或版本设置仅影响后续启动的会话；路径无效时会显式报错，不会自动改用另一份程序。"),
            )
            .into_any_element()
    }
    fn settings_modal_content(&self, window: &Window, view: Entity<Self>) -> AnyElement {
        let (title, subtitle) = match self.settings_tab {
            SettingsTab::General => ("通用", "本机数据与诊断设置"),
            SettingsTab::Appearance => ("外观", "主题与侧栏显示密度"),
            SettingsTab::Session => ("会话", "会话命名与标签页行为"),
            SettingsTab::Git => ("Git", "配置 Commit Message 的 AI 生成提示词"),
            SettingsTab::Cli => ("CLI", "可执行文件路径查找顺序与版本检测说明"),
            SettingsTab::Provider(provider) => (provider.label(), "管理可执行文件路径与版本检测"),
            SettingsTab::Models => ("模型同步", "查看 CLI 当前模型与 cc-switch 同步的模型"),
        };
        let nav_item = |tab: SettingsTab, label: &'static str, leading: AnyElement| -> AnyElement {
            let view = view.clone();
            let active = self.settings_tab == tab;
            Button::new(match tab {
                SettingsTab::General => "settings-tab-general",
                SettingsTab::Appearance => "settings-tab-appearance",
                SettingsTab::Session => "settings-tab-session",
                SettingsTab::Git => "settings-tab-git",
                SettingsTab::Cli => "settings-tab-cli",
                SettingsTab::Provider(Provider::Claude) => "settings-tab-claude",
                SettingsTab::Provider(Provider::Codex) => "settings-tab-codex",
                SettingsTab::Models => "settings-tab-models",
            })
            .ghost()
            .h(px(32.))
            .w_full()
            .px_2()
            .rounded(px(8.))
            .selected(active)
            .accessibility_label(label)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.settings_tab = tab;
                    cx.notify();
                });
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(leading)
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(if active {
                                palette().text
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    ),
            )
            .into_any_element()
        };
        let section = |text: &'static str| {
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_size(px(10.))
                .text_color(rgb(palette().faint))
                .child(text)
        };
        let general_icon = if self.settings_tab == SettingsTab::General {
            palette().text
        } else {
            palette().faint
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
        let appearance_icon = if self.settings_tab == SettingsTab::Appearance {
            palette().text
        } else {
            palette().faint
        };
        let appearance_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(IconName::Palette)
                    .small()
                    .text_color(rgb(appearance_icon)),
            )
            .into_any_element();
        let models_icon = if self.settings_tab == SettingsTab::Models {
            palette().text
        } else {
            palette().faint
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
            palette().text
        } else {
            palette().faint
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
            .bg(rgb(palette().sidebar_bg))
            .border_r_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_3()
                    .text_size(px(16.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child("设置"),
            )
            .child(section("应用"))
            .child(nav_item(SettingsTab::General, "通用", general_leading))
            .child(nav_item(
                SettingsTab::Appearance,
                "外观",
                appearance_leading,
            ))
            .child(nav_item(
                SettingsTab::Session,
                "会话",
                Icon::new(gpui_kit::assets::IconName::Sparkles)
                    .small()
                    .text_color(rgb(if self.settings_tab == SettingsTab::Session {
                        palette().text
                    } else {
                        palette().faint
                    }))
                    .into_any_element(),
            ))
            .child(nav_item(
                SettingsTab::Git,
                "Git",
                Icon::new(gpui_kit::assets::IconName::GitFork)
                    .small()
                    .text_color(rgb(if self.settings_tab == SettingsTab::Git {
                        palette().text
                    } else {
                        palette().faint
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
                    .text_color(rgb(palette().faint))
                    .child("本地 CLI · Windows"),
            );
        let page = match self.settings_tab {
            SettingsTab::General => self.settings_general_content(view.clone()),
            SettingsTab::Appearance => self.settings_appearance_content(view.clone()),
            SettingsTab::Session => self.settings_session_content(view.clone()),
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
                        .bg(rgb(palette().danger_bg))
                        .text_size(px(11.))
                        .text_color(rgb(palette().danger_text))
                        .child("当前数据目录为只读，无法保存设置或会话模型。"),
                )
            })
            .when_some(self.message.clone(), |column, message| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(rgb(palette().danger_bg))
                        .text_size(px(11.))
                        .text_color(rgb(palette().danger_text))
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
                            .text_color(rgb(palette().text))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(subtitle),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(palette().border)))
            .child(body)
            .child(
                h_flex()
                    .px_6()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
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
/// Turn a raw CLI failure blob into a one-line banner summary. The runtime
/// reports some failures as pretty-printed JSON (e.g. a Codex `turn.error`
/// object); those get parsed so the banner shows the cause, not the payload.
fn friendly_failure(error: &str) -> String {
    let trimmed = error.trim();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return trimmed.to_string();
    };
    let get = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| {
            json.get(*key)
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .filter(|value| !value.is_empty())
        })
    };
    let message = get(&["message", "error"]);
    let detail = json
        .get("codexErrorInfo")
        .and_then(|info| info.get("responseStreamDisconnected"))
        .map(|disconnected| {
            let status = disconnected["httpStatusCode"].as_i64();
            let detail = disconnected["additionalDetails"].as_str().unwrap_or("");
            let brief = detail.split(['\n', ';']).next().unwrap_or(detail).trim();
            let brief = if brief.chars().count() > 120 {
                format!("{}…", brief.chars().take(120).collect::<String>())
            } else {
                brief.to_string()
            };
            match (status, brief.is_empty()) {
                (Some(code), false) => format!("连接中断（HTTP {code}）：{brief}"),
                (Some(code), true) => format!("连接中断（HTTP {code}）"),
                (None, false) => format!("连接中断：{brief}"),
                (None, true) => "连接中断".to_string(),
            }
        });
    detail.or(message).unwrap_or_else(|| trimmed.to_string())
}
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
            && let Some(approval) = item.approval.as_mut()
            && approval.status == "等待中"
        {
            approval.status = "已取消".into();
        }
        if item.kind == ItemKind::Tool
            && item.key.starts_with(&prefix)
            && matches!(item.status.as_str(), "运行中" | "输出中")
        {
            item.status = status.into();
        }
    }
}

#[cfg(test)]
pub fn apply_event(session: &mut SessionRecord, prefix: &str, event: Event) {
    apply_event_indexed(session, prefix, event, &mut MessageIndex::default());
}

fn apply_event_indexed(
    session: &mut SessionRecord,
    prefix: &str,
    event: Event,
    index: &mut MessageIndex,
) -> Option<usize> {
    if let Event::ToolInput { key, input } = event {
        let key = format!("{prefix}-{key}");
        if let Some(position) = index.find(&session.messages, &key) {
            session.messages[position].input = input;
            return Some(position);
        } else {
            session.messages.push(ChatItem {
                key,
                kind: ItemKind::Tool,
                text: "工具调用".into(),
                input,
                detail: String::new(),
                status: "运行中".into(),
                approval: None,
            });
        }
        return Some(session.messages.len() - 1);
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
        Event::ErrorBanner(text) => (
            format!("error-{}", session.messages.len()),
            ItemKind::Error,
            text,
            String::new(),
            String::new(),
            false,
        ),
        _ => return None,
    };
    let key = format!("{prefix}-{key}");
    if let Some(position) = index.find(&session.messages, &key) {
        let item = &mut session.messages[position];
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
        Some(position)
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
                approval: None,
            });
            Some(session.messages.len() - 1)
        } else {
            None
        }
    }
}

impl Render for ConduitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let render_started = Instant::now();
        let narrow = window.viewport_size().width < px(1100.);
        let app_entity = cx.entity();
        let selected = self
            .selected_session
            .and_then(|id| self.session(id))
            .map(SessionSummary::from);
        let message_count = selected.as_ref().map_or(0, |session| session.message_count);
        let outline_session = self.selected_session.and_then(|id| {
            self.store
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.sessions)
                .find(|session| session.id == id)
        });
        self.outline.refresh(outline_session);
        if self.timeline_session != self.selected_session {
            self.scroll.reset(message_count);
            self.timeline_session = self.selected_session;
        } else {
            let previous_count = self.scroll.item_count();
            if message_count > previous_count {
                self.scroll.splice(
                    previous_count..previous_count,
                    message_count - previous_count,
                );
            } else if message_count < previous_count {
                self.scroll.splice(message_count..previous_count, 0);
            }
        }
        if self.at_bottom {
            self.scroll.scroll_to_end();
        }
        self.scroll
            .set_scroll_handler(cx.listener(|this, event: &ListScrollEvent, _, cx| {
                this.at_bottom = !event.is_scrolled;
                cx.notify();
            }));
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.begin_render(
                    message_count,
                    self.runs.len(),
                    self.store.workspaces.iter().map(|w| w.sessions.len()).sum(),
                );
            });
        }
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
            .bg(rgb(palette().sidebar_bg))
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
                        .text_color(rgb(palette().faint))
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
                                .text_color(rgb(if self.show_archived {
                                    palette().text
                                } else {
                                    palette().faint
                                }))
                                .when(self.show_archived, |row| {
                                    row.bg(rgb(palette().surface_hover))
                                })
                                .cursor_pointer()
                                .hover(|style| {
                                    style
                                        .bg(rgb(palette().surface_hover))
                                        .text_color(rgb(palette().text))
                                })
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
                                        .hover(|style| style.bg(rgb(palette().surface_hover)))
                                        .on_click(cx.listener(Self::add_workspace))
                                })
                                .child(Icon::new(IconName::Plus).small().text_color(rgb(
                                    if can_edit {
                                        palette().muted
                                    } else {
                                        palette().faint
                                    },
                                ))),
                        ),
                ),
        );
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            let sessions: Vec<&SessionRecord> = workspace
                .sessions
                .iter()
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
                .or_else(|| workspace.sessions.first().map(|session| session.id));
            let workspace_selected = self.selected_workspace == Some(index);
            let count = sessions.len();
            let limit = self.store.ui.sessions_per_workspace.clamp(1, 30);
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
                        .text_color(rgb(if workspace_selected {
                            palette().text
                        } else {
                            palette().muted
                        }))
                        .when(workspace_selected, |row| row.bg(rgb(palette().surface)))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(palette().surface_hover)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.switch(index, last, window, cx)
                        }))
                        .child(
                            Icon::new(IconName::Folder)
                                .small()
                                .text_color(rgb(palette().faint)),
                        )
                        .child(div().flex_1().truncate().child(workspace.name()))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
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
                let can_ai_rename = can_edit
                    && session
                        .messages
                        .iter()
                        .any(|item| item.kind == ItemKind::User);
                list =
                    list.child(
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
                                palette().faint
                            } else if active {
                                palette().text
                            } else {
                                palette().muted
                            }))
                            .when(active, |row| row.bg(rgb(palette().surface_hover)))
                            .cursor_pointer()
                            .hover(|style| {
                                style
                                    .bg(rgb(palette().surface_hover))
                                    .text_color(rgb(palette().text))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch(index, Some(id), window, cx)
                            }))
                            .context_menu({
                                let menu_app = app_entity.clone();
                                move |menu, _, _| {
                                    let rename_app = menu_app.clone();
                                    let ai_rename_app = menu_app.clone();
                                    let archive_app = menu_app.clone();
                                    let delete_app = menu_app.clone();
                                    menu.min_w(px(150.))
                                        .item(
                                            PopupMenuItem::new("重命名")
                                                .icon(IconName::Replace)
                                                .on_click(move |_, window, cx| {
                                                    rename_app.update(cx, |this, cx| {
                                                        this.open_rename_dialog(id, window, cx)
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new("AI 重命名")
                                                .icon(gpui_kit::assets::IconName::Sparkles)
                                                .disabled(!can_ai_rename)
                                                .on_click(move |_, _, cx| {
                                                    ai_rename_app.update(cx, |this, cx| {
                                                        this.ai_rename_session(id, cx)
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new(if archived {
                                                "取消归档"
                                            } else {
                                                "归档"
                                            })
                                            .icon(IconName::Inbox)
                                            .on_click(move |_, _, cx| {
                                                archive_app.update(cx, |this, cx| {
                                                    this.toggle_archive(id, cx)
                                                });
                                            }),
                                        )
                                        .item(
                                            PopupMenuItem::new("删除")
                                                .icon(IconName::Delete)
                                                .on_click(move |_, window, cx| {
                                                    delete_app.update(cx, |this, cx| {
                                                        this.open_delete_dialog(id, window, cx)
                                                    });
                                                }),
                                        )
                                }
                            })
                            .child(provider_icon(session.provider))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .line_height(px(16.))
                                    .child(if session.title.is_empty() {
                                        "新会话".to_string()
                                    } else {
                                        single_line_title(&session.title)
                                    }),
                            )
                            .when(self.session_namings.contains_key(&id), |row| {
                                row.child(Spinner::new().small().color(rgb(palette().muted).into()))
                            })
                            .when(archived, |row| {
                                row.child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(rgb(palette().faint))
                                        .child("已归档"),
                                )
                            })
                            .when_some(run_status, |row, status| {
                                row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(status),
                                )
                            }),
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
                        .hover(|style| style.bg(rgb(palette().surface_hover)))
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
                                .text_color(rgb(palette().faint))
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
                                    .text_color(rgb(palette().faint)),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(palette().muted))
                                    .child("还没有工作区"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
                            .child("点击上方 + 添加一个本地目录"),
                    ),
            );
        }
        sidebar = sidebar.child(list).child(
            v_flex()
                .gap_1()
                .pt_2()
                .border_t_1()
                .border_color(rgb(palette().border))
                .child(
                    h_flex()
                        .id("settings")
                        .h(px(32.))
                        .px_2()
                        .gap_2()
                        .items_center()
                        .rounded(px(8.))
                        .text_size(px(13.))
                        .text_color(rgb(palette().muted))
                        .cursor_pointer()
                        .hover(|style| {
                            style
                                .bg(rgb(palette().surface_hover))
                                .text_color(rgb(palette().text))
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_settings(window, cx);
                        }))
                        .child(
                            Icon::new(IconName::Settings)
                                .small()
                                .text_color(rgb(palette().faint)),
                        )
                        .child("设置"),
                )
                .child(
                    div()
                        .px_2()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child("本地 CLI · Windows · Preview"),
                ),
        );
        let mut messages = div()
            .v_flex()
            .id("messages")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
            .min_w_0()
            .px_6()
            .py_5()
            .gap_5();
        if let Some(session) = &selected {
            if session.message_count == 0 {
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
                                        .text_color(rgb(palette().faint))
                                        .child(format!("{} · 已就绪", session.provider.label())),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(30.))
                                .font_bold()
                                .text_color(rgb(palette().text))
                                .child("今天想完成什么？"),
                        )
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(rgb(palette().muted))
                                .child(format!("在 {} 中开始你的任务", directory)),
                        ),
                );
            }
            if session.message_count > 0 {
                let app = app_entity.clone();
                messages = div()
                    .id("messages")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        gpui_kit::list(self.scroll.clone(), move |index, _, cx| {
                            app.update(cx, |this, cx| this.render_message(index, cx))
                        })
                        .size_full()
                        .pt_5(),
                    )
                    .vertical_scrollbar(&self.scroll);
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
                            .border_color(rgb(palette().border))
                            .bg(rgb(palette().surface))
                            .child(
                                div()
                                    .text_base()
                                    .font_bold()
                                    .text_color(rgb(palette().muted))
                                    .child("C"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("今天想完成什么？"),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "会话将在 {directory} 中运行，输入任务后会按上次使用的引擎新建会话。"
                            )),
                    ),
            );
        } else {
            messages = messages.child(self.session_heatmap(13));
        }

        let mut conversation = v_flex().flex_1().min_w_0().h_full();
        conversation = conversation.child(self.session_tab_bar(cx));
        // Keep the outline attached to the centered message column instead of
        // the window edge. It stays outside the scrolling list, while the
        // rail itself is vertically centered in the chat viewport.
        let mut message_area = h_flex().flex_1().min_h_0().min_w_0().items_stretch();
        if !self.outline.entries.is_empty() {
            message_area = message_area.child(
                h_flex()
                    .w_full()
                    .when(!self.store.ui.full_width_content, |row| {
                        row.max_w(px(952.)).mx_auto()
                    })
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .items_stretch()
                    .child(self.message_outline(cx))
                    .child(messages)
                    // Balance the rail so messages stay aligned with the composer.
                    .child(div().w(px(32.)).flex_shrink_0()),
            );
        } else {
            message_area = message_area.child(messages);
        }
        conversation = conversation.child(message_area);
        // Live engine activity, shown in the conversation itself rather than
        // only through the send button and the tab's short status.
        let run_activity = self
            .selected_session
            .and_then(|id| self.runs.get(&id))
            .map(|run| run.activity.clone());
        if let Some(activity) = run_activity {
            conversation = conversation.child(
                div().w_full().px_6().pb_3().child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .when(!self.store.ui.full_width_content, |row| {
                            row.max_w(px(840.)).mx_auto()
                        })
                        .child(Spinner::new().small().color(rgb(palette().muted).into()))
                        .when_some(
                            selected.as_ref().map(|session| session.provider),
                            |row, provider| row.child(provider_icon(provider)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(rgb(palette().muted))
                                .child(activity),
                        ),
                ),
            );
        }
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
                        .when(!self.store.ui.full_width_content, |card| {
                            card.max_w(px(840.)).mx_auto()
                        })
                        .p_4()
                        .gap_3()
                        .rounded(px(14.))
                        .border_1()
                        .border_color(rgb(palette().warning_border))
                        .bg(rgb(palette().warning_bg))
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
                                        .bg(rgb(palette().warning_icon_bg))
                                        .child(
                                            Icon::new(IconName::TriangleAlert)
                                                .small()
                                                .text_color(rgb(palette().warning_text)),
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
                                                        .text_color(rgb(palette().highlight_text))
                                                        .child(approval.title.clone()),
                                                )
                                                .when(!queue_label.is_empty(), |row| {
                                                    row.child(
                                                        h_flex()
                                                            .h(px(18.))
                                                            .px_2()
                                                            .items_center()
                                                            .rounded_full()
                                                            .bg(rgb(palette().warning_chip_bg))
                                                            .text_size(px(10.))
                                                            .text_color(rgb(
                                                                palette().warning_chip_text
                                                            ))
                                                            .child(queue_label.clone()),
                                                    )
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .line_height(relative(1.4))
                                                .text_color(rgb(palette().warning_muted))
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
                                    .border_color(rgb(palette().warning_inner_border))
                                    .bg(rgb(palette().warning_code_bg))
                                    .text_xs()
                                    .line_height(relative(1.5))
                                    .text_color(rgb(palette().warning_code_text))
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
                                            .text_color(rgb(palette().warning_accent))
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
                    .border_color(rgb(palette().danger_border))
                    .bg(rgb(palette().danger_bg))
                    .text_sm()
                    .text_color(rgb(palette().danger_text))
                    .child(message.clone()),
            );
        }
        let input_focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let provider = selected.as_ref().map(|session| session.provider);
        let send_ready = can_edit && !busy && has_workspace;
        // The latest-message arrow appears
        // only when the user has scrolled away from the end of the message list.
        let show_latest_button = !self.at_bottom
            && selected
                .as_ref()
                .is_some_and(|session| session.message_count > 0);
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
        let usage_parts = self.usage_status_parts();
        let mut composer = v_flex()
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
            .p_2()
            .gap_1()
            .rounded(px(22.))
            .bg(rgb(palette().surface))
            .border_1()
            .border_color(if input_focused {
                rgb(palette().focus)
            } else {
                rgb(palette().border)
            })
            .on_click(cx.listener(|this, _, window, cx| {
                if this.selected_workspace.is_some() {
                    this.input.read(cx).focus_handle(cx).focus(window, cx);
                }
            }))
            .child(
                h_flex()
                    .id("composer-workspace")
                    .h(px(32.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(10.))
                    .bg(rgb(palette().surface_strong))
                    // This row is only for the model/workspace context. Do not
                    // let clicks on its empty area bubble to the composer shell:
                    // when no workspace is selected, that handler opens the
                    // workspace directory picker.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(!has_workspace, |row| {
                        row.cursor_pointer()
                            .hover(|style| style.bg(rgb(palette().control_hover)))
                            .on_click(cx.listener(Self::add_workspace))
                    })
                    .child(
                        Icon::new(IconName::Folder)
                            .small()
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
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
                    .when(selected.is_some(), |row| {
                        row.child(self.permission_picker(app_entity.clone()))
                    })
                    .when_some(usage_parts, |row, parts| {
                        row.child(h_flex().gap_3().items_center().min_w_0().children(parts))
                    })
                    .child(div().flex_1())
                    .child(if active {
                        h_flex()
                            .id("stop")
                            .w(px(30.))
                            .h(px(30.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(palette().text))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(palette().control_hover)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(run) =
                                    this.selected_session.and_then(|id| this.runs.get_mut(&id))
                                {
                                    run.runtime.cancel();
                                    run.approvals.clear();
                                    run.status = "正在停止…".into();
                                    run.activity = "正在停止…".into();
                                }
                                this.input.read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .w(px(8.))
                                    .h(px(8.))
                                    .rounded(px(2.))
                                    .bg(rgb(palette().on_text)),
                            )
                            .into_any_element()
                    } else {
                        h_flex()
                            .id("send")
                            .w(px(30.))
                            .h(px(30.))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(if send_ready {
                                palette().text
                            } else {
                                palette().surface_strong
                            }))
                            .when(send_ready, |button| {
                                button
                                    .cursor_pointer()
                                    .hover(|style| style.bg(rgb(palette().control_hover)))
                                    .on_click(cx.listener(Self::send_message))
                            })
                            .child(Icon::new(IconName::ArrowUp).small().text_color(rgb(
                                if send_ready {
                                    palette().on_text
                                } else {
                                    palette().faint
                                },
                            )))
                            .into_any_element()
                    }),
            );
        if !self.store.ui.full_width_content {
            composer = composer.max_w(px(840.)).mx_auto();
        }
        let composer: AnyElement = composer.into_any_element();
        // The "jump to latest" arrow floats at the lower-right of the message
        // area, just above the composer's top-right corner, and only exists
        // while the list is scrolled away from its end.
        conversation = conversation.child(
            div().px_6().pt_3().pb_2().child(
                div()
                    .relative()
                    .w_full()
                    .when(!self.store.ui.full_width_content, |card| {
                        card.max_w(px(840.)).mx_auto()
                    })
                    .child(composer)
                    .when(show_latest_button, |area| {
                        area.child(
                            Button::new("scroll-to-latest")
                                .ghost()
                                .tooltip("转到最新消息")
                                .accessibility_label("转到最新消息")
                                .absolute()
                                .top(px(-40.))
                                .right(px(14.))
                                .w(px(30.))
                                .h(px(30.))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(palette().border))
                                .bg(rgb(palette().surface_strong))
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.scroll_to_end();
                                    this.at_bottom = true;
                                    this.message_focus = None;
                                    this.pending_message_jump = false;
                                    cx.notify();
                                }))
                                .child(
                                    Icon::new(gpui_kit::assets::IconName::ArrowDown)
                                        .small()
                                        .text_color(rgb(palette().muted)),
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
            .border_color(rgb(palette().border))
            .when(self.git_repository.is_some(), |bar| {
                bar.child(self.branch_picker(app_entity.clone()))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child(toolbar_hint),
            )
            .when(!self.closing && !self.session_namings.is_empty(), |bar| {
                bar.child(
                    h_flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1()
                        .child(Spinner::new().small().color(rgb(palette().muted).into()))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child("自动命名中…"),
                        ),
                )
            })
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
                        |button| {
                            button
                                .bg(rgb(palette().surface_hover))
                                .text_color(rgb(palette().text))
                        },
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
            })
            .when(self.store.ui.performance_monitor, |bar| {
                bar.child({
                    let hover_monitor = self.performance.clone();
                    let pinned_monitor = self.performance.clone();
                    div()
                        .id("performance-tooltip")
                        .flex_shrink_0()
                        .when(!self.performance_open, |trigger| {
                            trigger.tooltip(move |_, _| hover_monitor.clone().into())
                        })
                        .child(
                            Popover::new("performance-popover")
                                .anchor(Anchor::BottomRight)
                                .appearance(false)
                                .open(self.performance_open)
                                .on_open_change(cx.listener(|this, open, _, cx| {
                                    this.performance_open = *open;
                                    cx.notify();
                                }))
                                .trigger(
                                    Button::new("performance-monitor")
                                        .ghost()
                                        .small()
                                        .compact()
                                        .icon(gpui_kit::assets::IconName::Activity)
                                        .accessibility_label("性能检测")
                                        .cursor_pointer(),
                                )
                                .content(move |_, _, _| pinned_monitor.clone()),
                        )
                })
            });
        let title_bar = TitleBar::new()
            .bg(rgb(palette().canvas))
            .border_color(rgb(palette().border))
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
                            .bg(rgb(palette().text))
                            .text_size(px(11.))
                            .font_bold()
                            .text_color(rgb(palette().on_text))
                            .child("C"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("Conduit"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
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
                .and_then(|key| {
                    self.session(self.selected_session?)?
                        .messages
                        .iter()
                        .find(|m| &m.key == key)
                })
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
                .unwrap_or_else(|| {
                    "展开工具调用卡片后，点击右侧详情图标可在此查看完整输入与输出。".into()
                });
            let tool_body: AnyElement = v_flex()
                .flex_1()
                .min_h_0()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("工作目录"),
                )
                .child(div().text_xs().text_color(rgb(palette().text)).child(path))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("原生会话 ID"),
                )
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
                .bg(rgb(palette().sidebar_bg))
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
                        .border_color(rgb(palette().border))
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
                            .text_color(rgb(palette().muted)),
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
                    .hover(|style| style.bg(rgb(palette().surface_hover)))
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
        let view = v_flex()
            .relative()
            .size_full()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .child(title_bar)
            .child(main_area);
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.record_render(render_started.elapsed());
            });
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_event, finish_tool_cards, highlight_ranges, latest_naming_excerpt, same_failure,
        search_snippet, session_matches, single_line_title,
    };
    use crate::{
        domain::{ChatItem, ItemKind, PermissionLevel, Provider, SessionRecord, Usage},
        protocol::Event,
    };
    #[test]
    fn indexed_events_keep_order_inputs_and_stream_snapshots_across_batches() {
        let mut s = session();
        let mut index = super::MessageIndex::default();
        for ordinal in 0..500 {
            super::apply_event_indexed(
                &mut s,
                "turn-1",
                Event::ToolInput {
                    key: ordinal.to_string(),
                    input: format!("input-{ordinal}"),
                },
                &mut index,
            );
        }
        for ordinal in 0..500 {
            super::apply_event_indexed(
                &mut s,
                "turn-1",
                Event::Tool {
                    key: ordinal.to_string(),
                    title: "Bash".into(),
                    detail: "result".into(),
                    status: "完成".into(),
                },
                &mut index,
            );
        }
        assert_eq!(s.messages.len(), 500);
        for (ordinal, item) in s.messages.iter().enumerate() {
            assert_eq!(item.key, format!("turn-1-{ordinal}"));
            assert_eq!(item.input, format!("input-{ordinal}"));
            assert_eq!(item.detail, "result");
        }
        // Approval handling appends messages outside the event index.
        s.record_tool_approval(
            "turn-1",
            "ticket",
            "请求使用 Write",
            r#"{"tool_use_id":"write"}"#,
            "等待中",
        );
        super::apply_event_indexed(
            &mut s,
            "turn-1",
            Event::ToolInput {
                key: "write".into(),
                input: "payload".into(),
            },
            &mut index,
        );
        assert_eq!(s.messages.len(), 501);
        assert!(s.messages[500].approval.is_some());
        for text in ["first", " second"] {
            super::apply_event_indexed(
                &mut s,
                "turn-1",
                Event::Delta {
                    key: "reply".into(),
                    text: text.into(),
                },
                &mut index,
            );
        }
        super::apply_event_indexed(
            &mut s,
            "turn-1",
            Event::Text {
                key: "reply".into(),
                text: "complete snapshot".into(),
            },
            &mut index,
        );
        assert_eq!(s.messages.last().unwrap().text, "complete snapshot");
    }
    #[test]
    fn message_outline_keeps_real_indices_and_keys_amid_tools_and_replies() {
        let mut s = session();
        for (key, kind, text) in [
            ("user-1", ItemKind::User, "第一轮\n  修复\tbug"),
            ("tool-1", ItemKind::Tool, "cargo check"),
            ("assistant-1", ItemKind::Assistant, "完成"),
            ("user-2", ItemKind::User, "第一轮\n  修复\tbug"),
            ("notice", ItemKind::Notice, "notice"),
            ("user-3", ItemKind::User, ""),
        ] {
            s.messages.push(ChatItem {
                key: key.into(),
                kind,
                text: text.into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
            });
        }
        let entries = super::message_outline_entries(&s, 0);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.index, entry.key.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "user-1"), (3, "user-2"), (5, "user-3")]
        );
        assert_eq!(entries[0].label, "第一轮 修复 bug");
        assert_eq!(entries[2].label, "（空消息）");
        s.messages[0].text = "中🙂".repeat(90);
        let entries = super::message_outline_entries(&s, 0);
        assert_eq!(entries[0].label.chars().count(), 101);
        assert!(entries[0].label.ends_with('…'));
    }

    #[test]
    fn message_outline_cache_only_indexes_appends_and_resets_with_session() {
        let mut session = session();
        let mut cache = super::MessageOutlineCache::default();
        let item = |key: &str, kind| ChatItem {
            key: key.into(),
            kind,
            text: "用户消息".into(),
            input: String::new(),
            detail: String::new(),
            status: String::new(),
            approval: None,
        };
        session.messages.push(item("user-1", ItemKind::User));
        cache.refresh(Some(&session));
        let label = cache.entries[0].label.as_ptr();
        cache.refresh(Some(&session));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries[0].label.as_ptr(), label);
        session.messages.push(item("reply", ItemKind::Assistant));
        session.messages.push(item("user-2", ItemKind::User));
        cache.refresh(Some(&session));
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.entries[1].index, 2);
        session.messages.truncate(1);
        cache.refresh(Some(&session));
        assert_eq!(cache.entries.len(), 1);
        session.id += 1;
        cache.refresh(Some(&session));
        assert_eq!(cache.session, Some(session.id));
        cache.refresh(None);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn tool_text_preserves_literal_logs_code_and_removes_terminal_escapes() {
        let text = "\u{1b}[31m错误\u{1b}[0m\r\n\tcode();\rretry\n\u{1b}]8;;https://example.test\u{1b}\\label\u{1b}]8;;\u{7}\n<b>literal</b> **text**";
        assert_eq!(
            super::format_tool_text(text),
            "错误\n    code();\nretry\nlabel\n<b>literal</b> **text**"
        );
        let json = "{\"command\":\"echo 中文\",\"args\":[1,2]}";
        let formatted = super::format_tool_text(json);
        assert!(formatted.contains('\n'));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&formatted).unwrap(),
            serde_json::from_str::<serde_json::Value>(json).unwrap()
        );
        // A partial streaming JSON result remains useful text, never discarded.
        assert_eq!(
            super::format_tool_text("{\"output\":\"partial"),
            "{\"output\":\"partial"
        );
    }
    #[test]
    fn tool_approval_matches_provider_id_and_survives_execution_updates() {
        let mut s = session();
        for key in ["first", "second"] {
            apply_event(
                &mut s,
                "turn-1",
                Event::Tool {
                    key: key.into(),
                    title: "same command".into(),
                    detail: String::new(),
                    status: "运行中".into(),
                },
            );
        }
        let detail = r#"{"itemId":"first"}"#;
        s.record_tool_approval("turn-1", "ticket", "请求执行命令", detail, "等待中");
        s.record_tool_approval("turn-1", "ticket", "请求执行命令", detail, "已允许");
        apply_event(
            &mut s,
            "turn-1",
            Event::Tool {
                key: "first".into(),
                title: "same command".into(),
                detail: "done".into(),
                status: "completed".into(),
            },
        );
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].approval.as_ref().unwrap().status, "已允许");
        assert_eq!(s.messages[0].status, "completed");
        assert!(s.messages[1].approval.is_none());
        assert!(s.messages.iter().all(|item| item.kind == ItemKind::Tool));
    }
    #[test]
    fn permission_before_tool_start_merges_and_pending_cancellation_is_scoped() {
        let mut s = session();
        for prefix in ["turn-0", "turn-1"] {
            s.record_tool_approval(
                prefix,
                "ticket",
                "请求使用 WebSearch",
                r#"{"tool_use_id":"search"}"#,
                "等待中",
            );
            apply_event(
                &mut s,
                prefix,
                Event::Tool {
                    key: "search".into(),
                    title: "WebSearch".into(),
                    detail: String::new(),
                    status: "运行中".into(),
                },
            );
        }
        finish_tool_cards(&mut s, "turn-1", "已取消");
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].approval.as_ref().unwrap().status, "等待中");
        assert_eq!(s.messages[1].approval.as_ref().unwrap().status, "已取消");
        s.record_tool_approval(
            "turn-1",
            "ticket",
            "请求使用 WebSearch",
            r#"{"tool_use_id":"search"}"#,
            "已拒绝",
        );
        assert_eq!(s.messages[1].approval.as_ref().unwrap().status, "已拒绝");
    }
    #[test]
    fn claude_approval_without_id_matches_input_and_never_another_turn() {
        let mut s = session();
        for (prefix, key, query) in [
            ("old", "a", "one"),
            ("now", "b", "one"),
            ("now", "c", "two"),
        ] {
            apply_event(
                &mut s,
                prefix,
                Event::Tool {
                    key: key.into(),
                    title: "WebSearch".into(),
                    detail: String::new(),
                    status: "运行中".into(),
                },
            );
            apply_event(
                &mut s,
                prefix,
                Event::ToolInput {
                    key: key.into(),
                    input: format!(r#"{{"query":"{query}"}}"#),
                },
            );
        }
        s.record_tool_approval(
            "now",
            "request",
            "请求使用 WebSearch",
            r#"{"input":{"query":"one"}}"#,
            "已允许",
        );
        assert_eq!(s.messages.len(), 3);
        assert!(s.messages[0].approval.is_none());
        assert_eq!(s.messages[1].approval.as_ref().unwrap().status, "已允许");
        assert!(s.messages[2].approval.is_none());
    }
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
            permission: PermissionLevel::default(),
            usage: Usage::default(),
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
    fn latest_naming_excerpt_prefers_recent_turns() {
        let mut s = session();
        assert_eq!(latest_naming_excerpt(&s), None);
        for (index, (kind, text)) in [
            (ItemKind::User, "旧问题"),
            (ItemKind::Assistant, "旧回答"),
            (ItemKind::Tool, "工具输出应忽略"),
            (ItemKind::User, "新问题"),
            (ItemKind::Assistant, "新回答"),
        ]
        .into_iter()
        .enumerate()
        {
            s.messages.push(ChatItem {
                key: format!("m{index}"),
                kind,
                text: text.into(),
                input: String::new(),
                detail: String::new(),
                status: String::new(),
                approval: None,
            });
        }
        assert_eq!(
            latest_naming_excerpt(&s).as_deref(),
            Some("旧问题\n\n旧回答\n\n新问题\n\n新回答")
        );
        s.messages.push(ChatItem {
            key: "long".into(),
            kind: ItemKind::Assistant,
            text: "长".repeat(1_000),
            input: String::new(),
            detail: String::new(),
            status: String::new(),
            approval: None,
        });
        assert_eq!(latest_naming_excerpt(&s).unwrap().chars().count(), 800);
    }
    #[test]
    fn single_line_title_collapses_line_breaks() {
        assert_eq!(
            single_line_title("# AGENTS.md\n<INSTRUCTIONS>"),
            "# AGENTS.md <INSTRUCTIONS>"
        );
        assert_eq!(single_line_title("第一行\r第二行"), "第一行 第二行");
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
            approval: None,
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

#[cfg(all(test, feature = "ui-tests"))]
mod ui_tests {
    use super::{ChatItem, tool_content};
    use gpui_kit::{
        Context, InteractiveElement, IntoElement, ParentElement, Render, ScrollDelta, ScrollHandle,
        ScrollWheelEvent, StatefulInteractiveElement, Styled, TestAppContext, Window,
        component::v_flex, div, point, px,
    };

    #[gpui_kit::test]
    fn long_chat_virtualizes_real_rows_and_keeps_history_jump_on_append(cx: &mut TestAppContext) {
        use crate::{
            domain::{ItemKind, PermissionLevel, Provider, SessionRecord, Usage, Workspace},
            store::Store,
        };
        use gpui_kit::AppContext;
        cx.update(gpui_kit::init);
        let app_slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = app_slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = super::ConduitApp::new(window, cx, false, false);
                let messages = (0..10_000)
                    .map(|index| ChatItem {
                        key: format!("message-{index}"),
                        kind: if index % 2 == 0 {
                            ItemKind::User
                        } else {
                            ItemKind::Notice
                        },
                        text: format!("历史消息 {index}"),
                        input: String::new(),
                        detail: String::new(),
                        status: String::new(),
                        approval: None,
                    })
                    .collect();
                app.store = Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir(),
                        sessions: vec![SessionRecord {
                            id: 1,
                            provider: Provider::Claude,
                            title: "Long chat".into(),
                            created_at: 0,
                            exit_code: None,
                            archived: false,
                            native_id: None,
                            model: None,
                            messages,
                            interrupted: false,
                            permission: PermissionLevel::default(),
                            usage: Usage::default(),
                        }],
                    }],
                    ..Default::default()
                };
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app
            });
            *slot.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = app_slot.borrow().as_ref().unwrap().clone();
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let monitor = app.performance.read(cx);
            assert_eq!(monitor.message_count, 10_000);
            assert_eq!(app.outline.entries.len(), 5_000);
            assert!(app.outline.scroll.0.borrow().last_item_size.is_some());
            assert!(monitor.mounted_messages() > 0);
            assert!(
                monitor.mounted_messages() < 100,
                "offscreen rows must not be constructed"
            );
        });
        cx.update(|_, cx| app.update(cx, |app, cx| app.jump_to_message(1, "message-5000", cx)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let before = cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(app.scroll.bounds_for_item(5000).is_some());
            assert!(!app.at_bottom);
            app.scroll.logical_scroll_top()
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                super::apply_event(
                    app.session_mut(1).unwrap(),
                    "turn-new",
                    crate::protocol::Event::Delta {
                        key: "reply".into(),
                        text: "new streaming content".into(),
                    },
                );
                cx.notify();
            })
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let after = app.scroll.logical_scroll_top();
            assert_eq!(
                (after.item_ix, after.offset_in_item),
                (before.item_ix, before.offset_in_item),
                "new tail content must not pull readers away from history"
            );
            assert!(app.performance.read(cx).mounted_messages() < 100);
        });
    }

    struct ToolScrollFixture {
        outer: ScrollHandle,
        inner: ScrollHandle,
        sibling: ScrollHandle,
        item: ChatItem,
    }

    impl Render for ToolScrollFixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            // Exercise the production tool body inside another scrolling list.
            div().size_full().child(
                div()
                    .id("outer")
                    .w(px(600.))
                    .h(px(320.))
                    .overflow_y_scroll()
                    .track_scroll(&self.outer)
                    .child(
                        v_flex()
                            .gap_4()
                            .child(tool_content("first", &self.item, &self.inner).0)
                            .child(tool_content("second", &self.item, &self.sibling).0),
                    ),
            )
        }
    }

    #[gpui_kit::test]
    fn expanded_tool_long_text_scrolls_inside_its_fixed_viewport(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let outer = ScrollHandle::new();
        let inner = ScrollHandle::new();
        let sibling = ScrollHandle::new();
        let (_, cx) = cx.add_window_view(|_, _| ToolScrollFixture {
            outer: outer.clone(),
            inner: inner.clone(),
            sibling: sibling.clone(),
            item: serde_json::from_value(serde_json::json!({
                "key":"tool", "kind":"Tool", "text":"Bash", "input":"{\"command\":\"test\"}",
                "detail": (0..200).map(|i| format!("line {i}: 中文 output\n")).collect::<String>(),
                "status":"完成"
            }))
            .unwrap(),
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(inner.bounds().size.height, px(240.));
        assert!(
            inner.max_offset().y > px(1000.),
            "long output must retain its natural height"
        );
        let outer_before = outer.offset();
        let sibling_before = sibling.offset();
        cx.simulate_event(ScrollWheelEvent {
            position: inner.bounds().center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(inner.offset().y < px(0.), "wheel must scroll tool text");
        assert_eq!(
            outer.offset(),
            outer_before,
            "tool scrolling must leave chat in place"
        );
        assert_eq!(
            sibling.offset(),
            sibling_before,
            "other tools have independent scroll state"
        );
        let inner_before = inner.offset();
        cx.simulate_event(ScrollWheelEvent {
            // The gap between cards belongs to the outer chat viewport.
            position: point(px(30.), px(248.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-80.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(outer.offset().y < outer_before.y);
        assert_eq!(inner.offset(), inner_before);
    }
}
