//! Session inspector. Consumption comes from deduplicated turn observations;
//! current context always remains a separate provider snapshot.
mod files;

use super::right::{RightBody, RightView};
use super::*;
use crate::app::state::{PlanStatus, plan_from_messages};
use crate::subagents::{Agent, Status as AgentStatus};
use crate::usage_ledger::{ApiRequestOutcome, Observation};
use conduit::plugins::{context::ScopeKey, state::ViewInstanceKey};
use gpui_kit::component::{
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia},
    shimmer::ShimmerText,
    tooltip::Tooltip,
};
use std::{cell::RefCell, rc::Rc};

const MAX_DETAILS_HEIGHT: f32 = 320.;

#[derive(Default)]
pub(super) struct Sections {
    files_expanded: bool,
    files: files::Cache,
    files_scroll: ScrollHandle,
    tools: bool,
    tools_scroll: ScrollHandle,
    timing: bool,
    scroll: ScrollHandle,
    tool_stats: RefCell<Option<(ToolStatsKey, Rc<ToolStats>)>>,
    api_emoji: RefCell<Option<ApiQualityEmoji>>,
}

#[derive(PartialEq, Eq)]
struct ToolStatsKey {
    revision: u64,
    messages: usize,
    running: bool,
    locale: conduit::i18n::Locale,
}

#[derive(Default)]
struct ToolStats {
    count: usize,
    failed: usize,
    running: usize,
    interrupted: usize,
    unknown: usize,
    categories: BTreeMap<String, (usize, usize)>,
}

impl Sections {
    fn tools(&self, key: ToolStatsKey, messages: &[ChatItem]) -> Rc<ToolStats> {
        use crate::message_status::ToolStatus;

        let mut cache = self.tool_stats.borrow_mut();
        if let Some((cached, stats)) = &*cache
            && *cached == key
        {
            return stats.clone();
        }
        let mut stats = ToolStats::default();
        for tool in messages.iter().filter(|item| item.kind == ItemKind::Tool) {
            stats.count += 1;
            let (count, failures) = stats.categories.entry(tool_category(tool)).or_default();
            *count += 1;
            if matches!(
                ToolStatus::from_wire(&tool.status),
                ToolStatus::Cancelled | ToolStatus::Interrupted
            ) {
                stats.interrupted += 1;
            } else if tool_running(tool) && key.running {
                stats.running += 1;
            } else if tool_failed(tool) {
                stats.failed += 1;
                *failures += 1;
            } else if !tool_succeeded(tool) {
                stats.unknown += 1;
            }
        }
        let stats = Rc::new(stats);
        *cache = Some((key, stats.clone()));
        stats
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Consumption {
    input: Option<u64>,
    cached: Option<u64>,
    cache_write: Option<u64>,
    output: Option<u64>,
    elapsed_ms: u64,
    turns: usize,
    incomplete: bool,
    cache_hit_input: u128,
    cache_hit_cached: u128,
}
impl Consumption {
    fn add(&mut self, turn: &Observation) {
        fn add_known(total: &mut Option<u64>, value: Option<u64>) {
            if let Some(value) = value {
                *total = Some(total.unwrap_or(0).saturating_add(value));
            }
        }
        add_known(&mut self.input, turn.input);
        add_known(&mut self.cached, turn.cached);
        add_known(&mut self.cache_write, turn.cache_write);
        add_known(&mut self.output, turn.output);
        self.elapsed_ms = self.elapsed_ms.saturating_add(turn.elapsed_ms);
        self.turns += 1;
        self.incomplete |= turn.incomplete
            || turn.input.is_none()
            || turn.output.is_none()
            || turn.cached.is_none()
            || turn.cache_write.is_none();
        if let (Some(input), Some(cached)) = (turn.input, turn.cached)
            && cached <= input
        {
            self.cache_hit_input += u128::from(input);
            self.cache_hit_cached += u128::from(cached);
        }
    }
    fn cache_hit(self) -> Option<f64> {
        match self.cache_hit_input {
            0 => None,
            input => Some(self.cache_hit_cached as f64 * 100. / input as f64),
        }
    }
}

fn consumption<'a>(
    records: impl Iterator<Item = &'a Observation>,
    current: Option<&'a Observation>,
) -> (Consumption, Option<Observation>) {
    let mut total = Consumption::default();
    let mut latest = None;
    for turn in records {
        if current.is_some_and(|current| current.key == turn.key) {
            continue;
        }
        total.add(turn);
        if latest
            .as_ref()
            .is_none_or(|last: &&Observation| turn.finished_at >= last.finished_at)
        {
            latest = Some(turn);
        }
    }
    if let Some(turn) = current {
        total.add(turn);
        latest = Some(turn);
    }
    (total, latest.cloned())
}

fn history_consumption(usage: crate::domain::Usage) -> Consumption {
    let input = usage.input_tokens_known.then_some(usage.input_tokens);
    let cached = usage
        .cached_input_tokens_known
        .then_some(usage.cached_input_tokens);
    let cache_hit = input.zip(cached).filter(|(input, cached)| cached <= input);
    Consumption {
        input,
        cached,
        cache_write: usage.cache_write_input_tokens,
        output: usage.output_tokens_known.then_some(usage.output_tokens),
        incomplete: input.is_none() || !usage.output_tokens_known,
        cache_hit_input: cache_hit.map_or(0, |(input, _)| u128::from(input)),
        cache_hit_cached: cache_hit.map_or(0, |(_, cached)| u128::from(cached)),
        ..Default::default()
    }
}

fn text(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}
fn tokens(value: Option<u64>) -> String {
    fn abbreviated(value: u64, divisor: f64, suffix: char) -> String {
        let number = format!("{:.1}", value as f64 / divisor);
        format!("{}{suffix}", number.strip_suffix(".0").unwrap_or(&number))
    }
    match value {
        Some(value) if value >= 1_000_000 => abbreviated(value, 1_000_000., 'M'),
        Some(value) if value >= 1_000 => abbreviated(value, 1_000., 'k'),
        Some(value) => value.to_string(),
        None => "—".into(),
    }
}
fn duration(value: Option<u64>) -> String {
    match value {
        Some(ms) if ms >= 3_600_000 => format!("{}h{:02}m", ms / 3_600_000, ms / 60_000 % 60),
        Some(ms) if ms >= 60_000 => format!("{}m{:02}s", ms / 60_000, ms / 1_000 % 60),
        Some(ms) => format!("{:.1}s", ms as f64 / 1_000.),
        None => "—".into(),
    }
}
const API_QUALITY_WINDOW: usize = 5;
const API_QUALITY_BAR_HEIGHT: f32 = 34.;
const API_QUALITY_EMOJI_DURATION: Duration = Duration::from_millis(300);

struct ApiQualityEmoji {
    current: &'static str,
    previous: &'static str,
    changed_at: Instant,
    revision: u64,
}

fn api_quality_emoji(status: ApiQualityLevel, state: &Sections, cx: &App) -> AnyElement {
    let emoji = status.emoji();
    let now = Instant::now();
    let mut stored = state.api_emoji.borrow_mut();
    let animation = stored.get_or_insert(ApiQualityEmoji {
        current: emoji,
        previous: emoji,
        changed_at: now,
        revision: 0,
    });
    if animation.current != emoji {
        animation.previous = animation.current;
        animation.current = emoji;
        animation.changed_at = now;
        animation.revision += 1;
    }
    let slot = div()
        .id("session-data-api-quality-emoji")
        .test_support()
        .relative()
        .size(px(24.))
        .flex_shrink_0();
    let glyph = |emoji, scale, offset, opacity| {
        div()
            .absolute()
            .size_full()
            .left_0()
            .top(px(offset))
            .flex()
            .items_center()
            .justify_center()
            .text_size(ui_text_size(18. * scale))
            .line_height(relative(1.))
            .opacity(opacity)
            .child(emoji)
    };
    if cx.reduce_motion()
        || animation.previous == emoji
        || now.duration_since(animation.changed_at) >= API_QUALITY_EMOJI_DURATION
    {
        return slot.child(glyph(emoji, 1., 0., 1.)).into_any_element();
    }
    let previous = animation.previous;
    slot.with_animation(
        ("session-data-api-quality-emoji-change", animation.revision),
        Animation::new(API_QUALITY_EMOJI_DURATION),
        move |slot, progress| {
            let exit = (progress / 0.4).min(1.);
            let enter = ((progress - 0.25) / 0.75).clamp(0., 1.);
            // A small overshoot gives the incoming face a soft, single bounce.
            let pop = 1. + 3.4 * (enter - 1.).powi(3) + 2.4 * (enter - 1.).powi(2);
            slot.when(exit < 1., |slot| {
                slot.child(glyph(previous, 1. - 0.2 * exit, -4. * exit, 1. - exit))
            })
            .child(glyph(
                emoji,
                0.7 + 0.3 * pop,
                4. * (1. - pop),
                (enter * 4.).min(1.),
            ))
        },
    )
    .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApiQualityLevel {
    Preparing,
    Waiting,
    Responding,
    Unknown,
    Smooth,
    Good,
    Slow,
    Unstable,
    Failed,
}

impl ApiQualityLevel {
    fn emoji(self) -> &'static str {
        match self {
            Self::Preparing | Self::Waiting => "\u{23F3}",
            Self::Responding | Self::Good => "\u{1F642}",
            Self::Unknown => "\u{1F636}",
            Self::Smooth => "\u{1F604}",
            Self::Slow => "\u{1F610}",
            Self::Unstable => "\u{1F622}",
            Self::Failed => "\u{1F62D}",
        }
    }

    fn label(self) -> &'static str {
        text(match self {
            Self::Preparing => "session_data.api_quality_preparing",
            Self::Waiting => "session_data.api_quality_waiting",
            Self::Responding => "session_data.api_quality_responding",
            Self::Unknown => "session_data.api_quality_unknown",
            Self::Smooth => "session_data.api_quality_smooth",
            Self::Good => "session_data.api_quality_good",
            Self::Slow => "session_data.api_quality_slow",
            Self::Unstable => "session_data.api_quality_unstable",
            Self::Failed => "session_data.api_quality_failed",
        })
    }
}

fn api_quality_samples<'a>(records: impl Iterator<Item = &'a Observation>) -> Vec<&'a Observation> {
    let mut samples = records
        .filter(|record| match record.api_request_outcome {
            Some(ApiRequestOutcome::Cancelled) => false,
            Some(_) => true,
            // Live responses already have a measured latency, but no final outcome yet.
            None => record.api_first_response_ms.is_some(),
        })
        .collect::<Vec<_>>();
    samples.sort_by_key(|record| record.finished_at);
    samples.drain(..samples.len().saturating_sub(API_QUALITY_WINDOW));
    samples
}

fn api_quality<'a>(records: impl Iterator<Item = &'a Observation>) -> ApiQualityLevel {
    let recent = api_quality_samples(records);
    if recent.is_empty() {
        return ApiQualityLevel::Unknown;
    }
    if recent.last().unwrap().api_request_outcome == Some(ApiRequestOutcome::Failed) {
        return ApiQualityLevel::Failed;
    }
    if recent
        .iter()
        .any(|record| record.api_request_outcome == Some(ApiRequestOutcome::Failed))
    {
        return ApiQualityLevel::Unstable;
    }
    let mut latencies = recent
        .iter()
        .filter_map(|record| record.api_first_response_ms)
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let Some(median) = latencies.get(latencies.len() / 2) else {
        return ApiQualityLevel::Unknown;
    };
    match *median {
        ms if ms <= 2_000 => ApiQualityLevel::Smooth,
        ms if ms <= 5_000 => ApiQualityLevel::Good,
        ms if ms <= 10_000 => ApiQualityLevel::Slow,
        _ => ApiQualityLevel::Unstable,
    }
}

// Normalize first response latency to quality: <= 2s is best, >= 20s is worst.
// Missing measurements stay unknown; failed requests always have zero quality.
fn api_quality_score(record: &Observation) -> Option<f32> {
    match record.api_request_outcome {
        Some(ApiRequestOutcome::Failed) => Some(0.),
        Some(ApiRequestOutcome::Succeeded) | None => record
            .api_first_response_ms
            .map(|ms| 1. - (ms.saturating_sub(2_000).min(18_000) as f32 / 18_000.)),
        _ => None,
    }
}

fn api_quality_bar_color(score: Option<f32>) -> u32 {
    let colors = palette();
    let Some(score) = score else {
        return colors.muted;
    };
    let (from, to, weight) = if score >= 0.5 {
        (
            colors.warning_text,
            colors.activity_completed,
            (score - 0.5) * 2.,
        )
    } else {
        (colors.error_text, colors.warning_text, score * 2.)
    };
    [16, 8, 0].into_iter().fold(0, |color, shift| {
        let a = ((from >> shift) & 0xff) as f32;
        let b = ((to >> shift) & 0xff) as f32;
        color | (((a + (b - a) * weight).round() as u32) << shift)
    })
}

fn api_quality_chart(samples: &[&Observation], status: ApiQualityLevel) -> impl IntoElement {
    let mut chart = h_flex()
        .id("session-data-api-quality-chart")
        .test_support()
        .role(Role::Group)
        .aria_label(text("session_data.api_quality_note"))
        .flex_shrink_0()
        .w(px(52.))
        .h_full()
        .py(px(4.))
        .gap(px(3.))
        .justify_end()
        .items_end();
    if samples.is_empty() {
        return chart
            .items_center()
            .justify_center()
            .aria_label(status.label())
            .child(caption("—"));
    }
    for (index, record) in samples.iter().enumerate() {
        let score = api_quality_score(record);
        let quality = api_quality(std::iter::once(*record));
        let detail = SharedString::from(format!(
            "{} · {}",
            quality.label(),
            conduit::i18n::format(
                "session_data.api_quality_first_response",
                &[("duration", duration(record.api_first_response_ms))],
            ),
        ));
        chart = chart.child(
            div()
                .id(("session-data-api-quality-sample", index))
                .test_support()
                .role(Role::Group)
                .aria_label(detail.clone())
                .w(px(8.))
                .h_full()
                .flex_shrink_0()
                .flex()
                .items_end()
                .tooltip(move |window, cx| tooltip(detail.clone()).build(window, cx))
                .child(
                    div()
                        .id(SharedString::from(format!(
                            "session-data-api-quality-bar-{index}"
                        )))
                        .test_support()
                        .w_full()
                        .h(px(4. + score.unwrap_or(0.) * (API_QUALITY_BAR_HEIGHT - 4.)))
                        .rounded(px(2.))
                        .bg(rgb(api_quality_bar_color(score))),
                ),
        );
    }
    chart
}

fn tooltip(note: SharedString) -> Tooltip {
    Tooltip::element(move |window, _| {
        div()
            .max_w(px(300.).min((window.viewport_size().width - px(48.)).max(px(0.))))
            .whitespace_normal()
            .child(note.clone())
    })
}

fn caption(value: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(11.))
        .text_color(rgb(palette().muted))
        .child(value.into())
}
fn heading(label: &'static str, note: Option<SharedString>) -> Div {
    h_flex()
        .min_w_0()
        .items_center()
        .gap_1()
        .text_size(ui_text_size(11.))
        .text_color(rgb(palette().muted))
        .child(div().min_w_0().truncate().child(text(label)))
        .when_some(note, |heading, note| {
            heading.child(
                div()
                    .id(SharedString::from(format!("session-data-hint-{label}")))
                    .test_support()
                    .tab_index(0)
                    .role(Role::Group)
                    .aria_label(note.clone())
                    .size(px(18.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .cursor_default()
                    .focus_visible(|hint| hint.bg(rgb(palette().surface_hover)))
                    .tooltip(move |window, cx| tooltip(note.clone()).build(window, cx))
                    .child(
                        Icon::new(FullIconName::Info)
                            .size(px(12.))
                            .text_color(rgb(palette().muted)),
                    ),
            )
        })
}
fn metric(label: &'static str, value: String, detail: impl IntoElement) -> Div {
    let note = match label {
        "session_data.cache_hit" => Some(text("session_data.cache_hit_note").into()),
        "session_data.turn_elapsed" | "session_data.latest_elapsed" => {
            Some(text("session_data.speed_note").into())
        }
        _ => None,
    };
    card()
        .flex_1()
        .min_w_0()
        .gap_1()
        .p_3()
        .child(heading(label, note))
        .child(
            div()
                .text_size(ui_text_size(23.))
                .font_semibold()
                .truncate()
                .child(value),
        )
        .child(detail)
}
fn usage_row(label: &'static str, latest: Option<u64>, total: Option<u64>, cx: &App) -> Div {
    let value = |value| {
        div()
            .w(px(78.))
            .flex_shrink_0()
            .text_right()
            .text_size(ui_text_size(13.))
            .font_semibold()
            .font_family(Theme::global(cx).mono_font_family.clone())
            .child(tokens(value))
    };
    h_flex()
        .w_full()
        .gap_2()
        .child(caption(text(label)).flex_1())
        .child(value(latest))
        .child(value(total))
}
fn row(label: &'static str, value: impl Into<SharedString>, cx: &App) -> Div {
    h_flex()
        .w_full()
        .justify_between()
        .gap_3()
        .child(caption(text(label)))
        .child(
            div()
                .text_size(ui_text_size(12.))
                .font_family(Theme::global(cx).mono_font_family.clone())
                .child(value.into()),
        )
}
fn section(
    id: &'static str,
    label: &'static str,
    summary: String,
    expanded: bool,
    state: &Entity<Sections>,
    toggle: fn(&mut Sections),
    cx: &Context<ConduitApp>,
) -> Button {
    let state = state.clone();
    let note = match label {
        "session_data.timing" => Some(text("session_data.speed_note").into()),
        _ => None,
    };
    Button::new(id)
        .ghost()
        .small()
        .w_full()
        .h(px(58.))
        .px_3()
        .focus_ring(true)
        .toggled(expanded)
        .accessibility_label(text(label))
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_1()
                        .child(
                            heading(label, note)
                                .text_size(ui_text_size(13.))
                                .font_semibold()
                                .text_color(rgb(palette().text)),
                        )
                        .child(caption(summary).truncate()),
                )
                .child(
                    Icon::new(if expanded {
                        FullIconName::ChevronDown
                    } else {
                        FullIconName::ChevronRight
                    })
                    .size(px(14.))
                    .text_color(rgb(palette().faint)),
                ),
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            state.update(cx, |state, cx| {
                toggle(state);
                cx.notify();
            });
            cx.notify();
        }))
}
fn card() -> Div {
    v_flex()
        .w_full()
        .border_1()
        .rounded(px(12.))
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
        .overflow_hidden()
}

fn divider() -> Div {
    div().w_full().h(px(1.)).bg(rgb(palette().border))
}

fn agent_row(agent: &Agent) -> impl IntoElement {
    let colors = palette();
    let (icon, color) = match agent.status {
        AgentStatus::Starting => (FullIconName::LoaderCircle, colors.activity_running),
        AgentStatus::Running => (FullIconName::CirclePlay, colors.activity_running),
        AgentStatus::WaitingApproval => (FullIconName::ShieldAlert, colors.warning_text),
        AgentStatus::WaitingInput => (FullIconName::MessageCircle, colors.warning_text),
        AgentStatus::Completed => (FullIconName::CircleCheck, colors.activity_completed),
        AgentStatus::Failed => (FullIconName::CircleX, colors.error_text),
        AgentStatus::Stopped => (FullIconName::CircleStop, colors.muted),
        AgentStatus::Idle => (FullIconName::CirclePause, colors.muted),
        AgentStatus::Unknown => (FullIconName::Info, colors.muted),
    };
    let name = if !agent.name.trim().is_empty() {
        agent.name.trim().to_owned()
    } else if agent.id.chars().count() > 16 {
        let start: String = agent.id.chars().take(8).collect();
        let end: String = agent
            .id
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{start}…{end}")
    } else {
        agent.id.clone()
    };
    let identity = SharedString::from(format!("session-data-agent-{}-{}", agent.run, agent.id));
    let status = if matches!(agent.status, AgentStatus::Starting | AgentStatus::Running) {
        ShimmerText::new(agent.status.label())
            .id(identity.clone())
            .duration(Duration::from_secs(3))
            .into_any_element()
    } else {
        div().child(agent.status.label()).into_any_element()
    };
    let summary = if agent.activity.trim().is_empty() {
        &agent.task
    } else {
        &agent.activity
    };
    let summary = summary.lines().find(|line| !line.trim().is_empty());
    let tooltip_text = SharedString::from(
        [
            agent.name.as_str(),
            agent.id.as_str(),
            agent.status.label(),
            agent.task.as_str(),
            agent.activity.as_str(),
        ]
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n"),
    );
    let content = v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(ui_text_size(12.))
                        .font_medium()
                        .child(name),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(color))
                        .child(status),
                ),
        )
        .when_some(summary, |row, summary| {
            row.child(caption(summary.trim().to_owned()).truncate())
        });
    h_flex()
        .id(identity)
        .test_support()
        .role(Role::Group)
        .aria_label(tooltip_text.clone())
        .w_full()
        .min_w_0()
        .min_h(px(40.))
        .p_2()
        .gap_2()
        .items_start()
        .rounded(px(8.))
        .bg(rgb(colors.surface_strong))
        .hover(|row| row.bg(rgb(colors.surface_hover)))
        .tooltip(move |window, cx| tooltip(tooltip_text.clone()).build(window, cx))
        .child(
            div()
                .size(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(rgb(color).opacity(0.1))
                .child(Icon::new(icon).size(px(14.)).text_color(rgb(color))),
        )
        .child(content)
}

fn disclosure_details() -> Div {
    v_flex().px_3().pb_3().gap_2()
}

impl ConduitApp {
    pub(in crate::app) fn render_session_data(
        &mut self,
        key: &ViewInstanceKey,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> RightView {
        let ScopeKey::Session { workspace, id } = &key.scope else {
            return RightBody::Loading.into();
        };
        let state = self
            .plugins
            .session_data
            .entry(SessionKey {
                workspace: workspace.clone(),
                id: *id,
            })
            .or_insert_with(|| {
                cx.new(|_| Sections {
                    files_expanded: true,
                    tools: true,
                    timing: true,
                    ..Sections::default()
                })
            })
            .clone();
        let Some(session) = self.session(*id) else {
            return RightBody::Loading.into();
        };
        if session.transcript.loaded {
            files::refresh(
                &state,
                &session.messages,
                &workspace.path().to_string_lossy(),
                cx,
            );
        }
        let run = self.session_control.runs.get(id);
        let plan = plan_from_messages(session);
        let expanded = state.read(cx);
        let scroll = expanded.scroll.clone();
        let show_tools = expanded.tools;
        let show_timing = expanded.timing;
        let usage = run.map_or(session.usage, |run| run.usage);
        let current = run.and_then(|run| self.turn_usage_observation(*id, run, false));
        let records = self
            .store
            .usage_ledger
            .observations
            .iter()
            .filter(|turn| turn.session == *id && turn.workspace == workspace.path());
        let (mut total, latest) = consumption(records.clone(), current.as_ref());
        let history = (total.turns == 0).then_some(usage.history).flatten();
        if history.is_some() {
            total = history_consumption(usage);
        }
        let model = match run {
            Some(run) => run.model.as_deref(),
            None => session.model.as_deref(),
        }
        .unwrap_or(text("session_data.cli_default"));
        let effort = run.map_or(session.reasoning_effort, |run| run.reasoning_effort);
        let variant = run
            .and_then(|run| run.acp_variant.as_deref())
            .or_else(|| session.opencode.effective_variant());
        let (status, running) = if let Some(run) = run {
            if run.runtime.is_cancelled() {
                (text("session_data.stopping"), false)
            } else if !run.approvals.is_empty() {
                (run.waiting_status(), false)
            } else {
                (text("session_data.running"), true)
            }
        } else if session.interrupted {
            (text("session_data.interrupted"), false)
        } else {
            (text("session_data.idle"), false)
        };
        let color = if running {
            if palette().restrained_projects {
                palette().activity_running
            } else {
                palette().activity_completed
            }
        } else if run.is_some() || session.interrupted {
            palette().warning_text
        } else {
            palette().muted
        };
        let status_dot = div()
            .size(px(6.))
            .flex_shrink_0()
            .rounded_full()
            .bg(rgb(color));
        let status_dot = if running && !cx.reduce_motion() {
            status_dot
                .with_animation(
                    ("session-data-status-pulse", *id),
                    Animation::new(Duration::from_millis(1600)).repeat(),
                    |dot, delta| {
                        dot.opacity(0.4 + 0.6 * (0.5 + 0.5 * (delta * std::f32::consts::TAU).cos()))
                    },
                )
                .into_any_element()
        } else {
            status_dot.into_any_element()
        };
        let mut content = v_flex()
            .w_full()
            .min_w_0()
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(provider_icon(session.provider))
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .font_semibold()
                                    .child(session.provider.label()),
                            ),
                    )
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_1p5()
                            .child(status_dot)
                            .child(caption(status).text_color(rgb(color)).truncate()),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(ui_text_size(17.))
                            .font_semibold()
                            .truncate()
                            .child(model.to_owned()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .rounded_full()
                            .border_1()
                            .border_color(rgb(palette().border))
                            .bg(rgb(palette().surface_strong))
                            .child(caption(format!(
                                "{} · {}",
                                text("session_data.reasoning"),
                                variant.unwrap_or(effort.label())
                            ))),
                    ),
            );
        if let Some(run) = run
            && (run.model != session.model || run.reasoning_effort != session.reasoning_effort)
        {
            content = content.child(caption(conduit::i18n::format(
                "session_data.next_model",
                &[
                    (
                        "model",
                        session
                            .model
                            .clone()
                            .unwrap_or_else(|| text("session_data.cli_default").into()),
                    ),
                    ("effort", session.reasoning_effort.label().into()),
                ],
            )));
        }
        let context_ratio = usage.context_ratio();
        let mut context = card()
            .id("session-data-context-card")
            .test_support()
            .debug_selector(|| "session-data-context-card".into())
            .p_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(heading(
                        "session_data.context",
                        (context_ratio.is_some() && usage.context_window_estimated)
                            .then(|| text("session_data.estimated").into()),
                    ))
                    .when_some(context_ratio, |header, _| {
                        header.child(
                            h_flex()
                                .gap_2()
                                .child(caption(text("session_data.tokens_unit"))),
                        )
                    }),
            );
        if let Some(ratio) = context_ratio {
            let colors = palette();
            let warning_threshold = if colors.restrained_projects {
                0.8
            } else {
                0.75
            };
            let critical = if colors.restrained_projects {
                ratio > 0.95
            } else {
                ratio >= 0.9
            };
            let color = if critical {
                palette().error_text
            } else if ratio >= warning_threshold {
                palette().warning_text
            } else if colors.restrained_projects {
                colors.activity_running
            } else {
                palette().highlight_text
            };
            let bar_color = if colors.restrained_projects && !critical && ratio >= warning_threshold
            {
                colors.context_warning_fill
            } else {
                color
            };
            context = context
                .child(
                    h_flex()
                        .w_full()
                        .items_end()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(40.))
                                .font_semibold()
                                .text_color(rgb(color))
                                .whitespace_nowrap()
                                .child(format!(
                                    "{}{:.0}%",
                                    if usage.context_window_estimated {
                                        "≈"
                                    } else {
                                        ""
                                    },
                                    ratio * 100.
                                )),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .font_semibold()
                                .font_family(Theme::global(cx).mono_font_family.clone())
                                .child(format!(
                                    "{} / {}",
                                    tokens(usage.context_tokens),
                                    tokens(Some(usage.context_window))
                                )),
                        ),
                )
                .child(
                    progress::Progress::new("session-context-progress")
                        .value((ratio * 100.).clamp(0., 100.) as f32)
                        .color(rgb(bar_color))
                        .with_size(px(9.))
                        .w_full()
                        .accessibility_label(conduit::i18n::format(
                            "sessions.context_usage",
                            &[("arg0", format!("{:.0}", ratio * 100.))],
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .gap_2()
                        .child(caption(conduit::i18n::format(
                            "session_data.context_remaining",
                            &[(
                                "count",
                                tokens(
                                    usage
                                        .context_tokens
                                        .map(|used| usage.context_window.saturating_sub(used)),
                                ),
                            )],
                        )))
                        .when(critical, |row| {
                            row.child(
                                caption(text("session_data.context_high")).text_color(rgb(color)),
                            )
                        }),
                );
        } else if let Some(used) = usage.context_tokens {
            context = context.child(caption(conduit::i18n::format(
                "session_data.context_limit_unknown",
                &[("tokens", tokens(Some(used)))],
            )));
        } else {
            context = context.child(
                Empty::new().p_0().py_2().header(
                    EmptyHeader::new()
                        .flex_row()
                        .gap_2()
                        .media(
                            EmptyMedia::new().mb_0().child(
                                Icon::new(FullIconName::Info)
                                    .size(px(16.))
                                    .text_color(rgb(palette().muted)),
                            ),
                        )
                        .description(
                            EmptyDescription::new()
                                .flex_1()
                                .text_left()
                                .text_size(ui_text_size(12.))
                                .child(text("session_data.context_unknown")),
                        ),
                ),
            );
        }
        content = content.child(context);
        let active_request = run.filter(|run| {
            matches!(run.intent, crate::commands::RequestIntent::Message)
                && !run.runtime.is_cancelled()
        });
        let current_request = current.as_ref().filter(|_| active_request.is_some());
        let quality_samples = api_quality_samples(
            records
                .filter(|record| current_request.is_none_or(|current| current.key != record.key))
                .chain(current_request),
        );
        let request_quality = if let Some(run) = active_request {
            if run.api_request_started.is_none() {
                ApiQualityLevel::Preparing
            } else if run.api_first_response_ms.is_none() {
                ApiQualityLevel::Waiting
            } else {
                ApiQualityLevel::Responding
            }
        } else {
            api_quality(quality_samples.iter().copied())
        };
        let turn = latest.as_ref();
        let cache_hit = total.cache_hit();
        let turn_elapsed = turn.map(|turn| turn.elapsed_ms);
        let speed = turn.and_then(|turn| {
            let elapsed = turn.elapsed_ms;
            (elapsed > 0)
                .then(|| {
                    turn.output
                        .map(|output| (output as f64 * 1_000. / elapsed as f64).round() as u64)
                })
                .flatten()
        });
        if total.turns == 0 && history.is_none() {
            content = content.child(
                card()
                    .id("session-data-empty-state")
                    .test_support()
                    .debug_selector(|| "session-data-empty-state".into())
                    .border_dashed()
                    .items_center()
                    .gap_2()
                    .p_4()
                    .py_6()
                    .child(
                        div()
                            .size(px(36.))
                            .rounded_full()
                            .bg(rgb(palette().surface_strong))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(FullIconName::ChartColumn)
                                    .size(px(16.))
                                    .text_color(rgb(palette().muted)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .font_semibold()
                            .child(text("session_data.empty_turns_title")),
                    )
                    .child(
                        caption(text("session_data.empty_turns_description"))
                            .max_w(px(230.))
                            .w_full()
                            .text_center(),
                    ),
            );
        } else {
            let cache_detail = if let Some(hit) = cache_hit {
                progress::Progress::new("session-data-cache-progress")
                    .value(hit as f32)
                    .color(rgb(palette().activity_completed))
                    .small()
                    .w_full()
                    .accessibility_label(text("session_data.cache_hit"))
                    .into_any_element()
            } else {
                caption(text("session_data.metric_unknown")).into_any_element()
            };
            content = content.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_stretch()
                    .child(metric(
                        "session_data.cache_hit",
                        cache_hit.map_or_else(|| "—".into(), |value| format!("{value:.0}%")),
                        cache_detail,
                    ))
                    .child(metric(
                        if run.is_some() {
                            "session_data.turn_elapsed"
                        } else {
                            "session_data.latest_elapsed"
                        },
                        duration(turn_elapsed),
                        caption(speed.map_or_else(
                            || text("session_data.metric_unknown").into(),
                            |value| format!("{value} tok/s"),
                        ))
                        .truncate(),
                    )),
            );
            let mut usage_note = format!(
                "{}\n{}",
                text("session_data.cache_subset"),
                text(if history.is_some() {
                    "session_data.history_usage_note"
                } else {
                    "session_data.recorded_note"
                }),
            );
            if total.incomplete {
                usage_note.push('\n');
                usage_note.push_str(text("session_data.partial_note"));
            }
            let summary = card()
                .id("session-data-usage-summary")
                .test_support()
                .debug_selector(|| "session-data-usage-summary".into())
                .gap_2()
                .p_3()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .gap_2()
                        .child(heading("session_data.usage", Some(usage_note.into())).flex_1())
                        .child(
                            caption(text(if history.is_some() {
                                "session_data.latest_request"
                            } else if run.is_some() {
                                "session_data.current_turn"
                            } else {
                                "session_data.latest_turn"
                            }))
                            .w(px(78.))
                            .flex_shrink_0()
                            .text_right(),
                        )
                        .child(
                            caption(text(if history.is_some() {
                                "session_data.history_total"
                            } else {
                                "session_data.recorded_total"
                            }))
                            .w(px(78.))
                            .flex_shrink_0()
                            .text_right(),
                        ),
                )
                .child(usage_row(
                    "session_data.input",
                    history.map_or_else(|| turn.and_then(|turn| turn.input), |usage| usage.input),
                    total.input,
                    cx,
                ))
                .child(usage_row(
                    "session_data.output",
                    history.map_or_else(|| turn.and_then(|turn| turn.output), |usage| usage.output),
                    total.output,
                    cx,
                ))
                .child(divider())
                .child(usage_row(
                    "session_data.cache_read",
                    history.map_or_else(|| turn.and_then(|turn| turn.cached), |usage| usage.cached),
                    total.cached,
                    cx,
                ))
                .child(usage_row(
                    "session_data.cache_write",
                    history.map_or_else(
                        || turn.and_then(|turn| turn.cache_write),
                        |usage| usage.cache_write,
                    ),
                    total.cache_write,
                    cx,
                ));
            content = content.child(summary);
        }

        content = content.child(
            card()
                .id("session-data-api-quality-card")
                .test_support()
                .debug_selector(|| "session-data-api-quality-card".into())
                .role(Role::Group)
                .aria_label(format!(
                    "{} · {}",
                    text("session_data.api_quality"),
                    request_quality.label(),
                ))
                .px_3()
                .h(px(44.))
                .child(
                    h_flex()
                        .w_full()
                        .h_full()
                        .items_center()
                        .gap_2()
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .py_2()
                                .gap_2()
                                .child(api_quality_emoji(request_quality, state.read(cx), cx))
                                .child(heading(
                                    "session_data.api_quality",
                                    Some(text("session_data.api_quality_note").into()),
                                )),
                        )
                        .child(api_quality_chart(&quality_samples, request_quality)),
                ),
        );
        content = content.child(files::render(*id, session.transcript.loaded, &state, cx));
        let stats = session.transcript.loaded.then(|| {
            expanded.tools(
                ToolStatsKey {
                    revision: self.session_control.revision(*id),
                    messages: session.messages.len(),
                    running: run.is_some(),
                    locale: conduit::i18n::locale(),
                },
                &session.messages,
            )
        });
        let active_agents = session
            .subagents
            .iter()
            .filter(|agent| run.is_some_and(|run| agent.run == run.prefix) && agent.status.active())
            .count();
        let tool_summary = if let Some(stats) = &stats {
            conduit::i18n::format(
                "session_data.tool_summary",
                &[
                    ("calls", stats.count.to_string()),
                    ("failed", stats.failed.to_string()),
                    ("agents", session.subagents.len().to_string()),
                ],
            )
        } else {
            text("session_data.loading_history").into()
        };
        let mut tools = card().child(section(
            "session-data-tool-toggle",
            "session_data.tools_tasks",
            tool_summary,
            show_tools,
            &state,
            |state| state.tools = !state.tools,
            cx,
        ));
        if show_tools {
            let mut tool_details = disclosure_details()
                .id("session-data-tool-details")
                .test_support()
                .debug_selector(|| "session-data-tool-details".into());
            if let Some(stats) = &stats {
                let mut categories: Vec<_> = stats.categories.iter().collect();
                categories.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
                for (name, (count, failures)) in categories.into_iter().take(8) {
                    tool_details = tool_details.child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .child(caption(name.clone()).flex_1().min_w_0().truncate())
                            .when(*failures > 0, |row| {
                                row.child(
                                    caption(conduit::i18n::format(
                                        "session_data.category_failures",
                                        &[("count", failures.to_string())],
                                    ))
                                    .text_color(rgb(palette().error_text)),
                                )
                            })
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .font_semibold()
                                    .child(count.to_string()),
                            ),
                    );
                }
                for (label, count) in [
                    ("session_data.tool_running", stats.running),
                    ("session_data.tool_interrupted", stats.interrupted),
                    ("session_data.tool_unknown", stats.unknown),
                ] {
                    if count > 0 {
                        tool_details = tool_details.child(row(label, count.to_string(), cx));
                    }
                }
            } else {
                tool_details = tool_details.child(caption(text("session_data.loading_history")));
            }
            let mut details = v_flex().child(tool_details);
            if !session.subagents.is_empty() {
                // History and terminal results remain inspectable after the run ends.
                // Only the active count above is scoped to the current run.
                let agents = v_flex()
                    .px_3()
                    .pb_3()
                    .gap_2()
                    .child(div().text_size(ui_text_size(13.)).font_semibold().child(
                        conduit::i18n::format(
                            "subagents.title",
                            &[("total", session.subagents.len().to_string())],
                        ),
                    ))
                    .child(row(
                        "session_data.active_agents",
                        active_agents.to_string(),
                        cx,
                    ))
                    .children(session.subagents.iter().map(agent_row));
                details = details.child(agents);
            }
            tools = tools.child(
                div()
                    .id("session-data-tools-scroll")
                    .test_support()
                    .max_h(px(MAX_DETAILS_HEIGHT))
                    .overflow_y_scroll()
                    .track_scroll(&expanded.tools_scroll)
                    .child(details),
            );
        }

        let timing_summary = conduit::i18n::format(
            "session_data.timing_summary",
            &[
                ("turns", total.turns.to_string()),
                (
                    "duration",
                    duration((total.turns > 0).then_some(total.elapsed_ms)),
                ),
            ],
        );
        let mut timing = tools.child(divider());
        if !plan.is_empty() {
            let completed = plan
                .iter()
                .filter(|step| step.status == PlanStatus::Completed)
                .count();
            let mut plan_list = v_flex()
                .id("session-data-plan-list")
                .test_support()
                .debug_selector(|| "session-data-plan-list".into())
                .w_full()
                .px_3()
                .py_2()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .font_semibold()
                                .child(text("session_data.plan_tasks")),
                        )
                        .child(caption(format!("{completed}/{}", plan.len()))),
                );
            for (index, step) in plan.iter().enumerate() {
                let metadata = crate::app::chat::task_metadata(step, &plan);
                let (icon, color, status) = match step.status {
                    PlanStatus::Completed => (
                        FullIconName::CircleCheck,
                        palette().diff_add_text,
                        text("session_data.plan_completed"),
                    ),
                    PlanStatus::InProgress => (
                        FullIconName::CircleDot,
                        palette().warning_text,
                        text("session_data.plan_in_progress"),
                    ),
                    PlanStatus::Pending => (
                        FullIconName::CircleDashed,
                        palette().faint,
                        text("session_data.plan_pending"),
                    ),
                };
                plan_list = plan_list.child(
                    h_flex()
                        .id(SharedString::from(format!("session-data-task-{index}")))
                        .test_support()
                        .role(Role::Group)
                        .aria_label(format!("{} {}", step.label(), metadata).trim().to_owned())
                        .w_full()
                        .min_w_0()
                        .items_start()
                        .gap_2()
                        .child(Icon::new(icon).size(px(14.)).text_color(rgb(color)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(ui_text_size(12.))
                                .whitespace_normal()
                                .text_color(rgb(if step.status == PlanStatus::Completed {
                                    palette().muted
                                } else {
                                    palette().text
                                }))
                                .child(step.label())
                                .when(!metadata.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .text_size(ui_text_size(11.))
                                            .text_color(rgb(palette().muted))
                                            .child(metadata),
                                    )
                                }),
                        )
                        .child(caption(status).flex_shrink_0()),
                );
            }
            timing = timing.child(plan_list).child(divider());
        }
        timing = timing.child(section(
            "session-data-timing-toggle",
            "session_data.timing",
            timing_summary,
            show_timing,
            &state,
            |state| state.timing = !state.timing,
            cx,
        ));
        if show_timing {
            timing = timing.child(
                disclosure_details()
                    .child(row(
                        "session_data.recorded_turns",
                        total.turns.to_string(),
                        cx,
                    ))
                    .child(row(
                        "session_data.active_time",
                        duration((total.turns > 0).then_some(total.elapsed_ms)),
                        cx,
                    ))
                    .child(row(
                        "session_data.average_speed",
                        usage
                            .average_output_speed()
                            .map_or_else(|| "—".into(), |value| format!("{value} tok/s")),
                        cx,
                    )),
            );
        }
        if !plan.is_empty()
            || total.turns > 0
            || stats.is_some_and(|stats| stats.count > 0)
            || !session.subagents.is_empty()
        {
            content = content.child(timing);
        }
        RightBody::Content(
            v_flex()
                .id("session-data-panel")
                .test_support()
                .debug_selector(|| "session-data-panel".into())
                .role(Role::Group)
                .aria_label(text("session_data.title"))
                .flex_1()
                .min_h_0()
                .w_full()
                .min_w_0()
                .child(
                    div()
                        .id("session-data-scroll")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_y_scroll()
                        .track_scroll(&scroll)
                        .child(content),
                )
                .child(
                    div()
                        .h(px(STATUS_BAR_HEIGHT))
                        .flex_shrink_0()
                        .border_t_1()
                        .border_color(rgb(palette().border)),
                )
                .into_any_element(),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApiQualityLevel, ApiRequestOutcome, Observation, api_quality, api_quality_samples,
        api_quality_score, consumption,
    };
    use crate::domain::Provider;
    use std::path::PathBuf;
    fn turn(
        key: &str,
        input: Option<u64>,
        cached: Option<u64>,
        output: Option<u64>,
    ) -> Observation {
        Observation {
            key: key.into(),
            session: 1,
            workspace: PathBuf::from("C:/test"),
            provider: Provider::Claude,
            service: "cli".into(),
            service_name: None,
            model: None,
            finished_at: 1,
            input,
            cached,
            cache_write: None,
            output,
            elapsed_ms: 1000,
            incomplete: false,
            api_first_response_ms: None,
            api_request_outcome: None,
        }
    }
    fn api_turn(key: &str, outcome: ApiRequestOutcome, response_ms: Option<u64>) -> Observation {
        let mut turn = turn(key, None, None, None);
        turn.api_request_outcome = Some(outcome);
        turn.api_first_response_ms = response_ms;
        turn
    }

    #[test]
    fn api_quality_maps_latency_and_failures_to_qualitative_states() {
        assert_eq!(
            api_quality([api_turn("fast", ApiRequestOutcome::Succeeded, Some(1_500))].iter()),
            ApiQualityLevel::Smooth,
        );
        assert_eq!(
            api_quality([api_turn("slow", ApiRequestOutcome::Succeeded, Some(6_000))].iter()),
            ApiQualityLevel::Slow,
        );
        assert_eq!(
            api_quality([api_turn("failed", ApiRequestOutcome::Failed, None)].iter()),
            ApiQualityLevel::Failed,
        );
        assert_eq!(
            api_quality(
                [
                    api_turn("old-failure", ApiRequestOutcome::Failed, None),
                    api_turn("recovered", ApiRequestOutcome::Succeeded, Some(1_000)),
                ]
                .iter()
            ),
            ApiQualityLevel::Unstable,
        );
    }

    #[test]
    fn api_quality_ignores_cancelled_requests_and_uses_only_the_latest_five() {
        let mut records = vec![api_turn("old-failure", ApiRequestOutcome::Failed, None)];
        records.extend((0..5).map(|index| {
            api_turn(
                &format!("success-{index}"),
                ApiRequestOutcome::Succeeded,
                Some(1_000),
            )
        }));
        records.push(api_turn("cancelled", ApiRequestOutcome::Cancelled, None));
        let quality = api_quality(records.iter());
        assert_eq!(quality, ApiQualityLevel::Smooth);
    }

    #[test]
    fn api_quality_history_orders_completed_samples_and_preserves_individual_quality() {
        let mut records = (0..7)
            .map(|index| {
                let mut record = api_turn(
                    &format!("request-{index}"),
                    ApiRequestOutcome::Succeeded,
                    Some(index * 4_000),
                );
                record.finished_at = index;
                record
            })
            .collect::<Vec<_>>();
        records[4].api_request_outcome = Some(ApiRequestOutcome::Failed);
        let mut cancelled = api_turn("cancelled", ApiRequestOutcome::Cancelled, None);
        cancelled.finished_at = 10;
        records.push(cancelled);
        records.push(turn("unmeasured", None, None, None));
        records.reverse();
        let samples = api_quality_samples(records.iter());
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.key.as_str())
                .collect::<Vec<_>>(),
            [
                "request-2",
                "request-3",
                "request-4",
                "request-5",
                "request-6"
            ],
        );
        assert!(api_quality_score(samples[0]) > api_quality_score(samples[1]));
        assert_eq!(api_quality_score(samples[2]), Some(0.));
        assert_eq!(api_quality_score(samples[4]), Some(0.));
        assert_eq!(
            api_quality_score(&api_turn("unknown", ApiRequestOutcome::Succeeded, None)),
            None,
        );
        assert!(api_quality_samples(std::iter::empty()).is_empty());
    }

    #[test]
    fn api_quality_includes_measured_live_responses_without_fabricating_an_outcome() {
        let mut live = turn("live", None, None, None);
        assert!(api_quality_samples(std::iter::once(&live)).is_empty());
        live.api_first_response_ms = Some(1_000);
        live.finished_at = 10;
        assert_eq!(api_quality(std::iter::once(&live)), ApiQualityLevel::Smooth);
        assert_eq!(api_quality_score(&live), Some(1.));
        assert_eq!(live.api_request_outcome, None);

        let completed = (0..5)
            .map(|index| {
                let mut record = api_turn(
                    &format!("completed-{index}"),
                    ApiRequestOutcome::Succeeded,
                    Some(3_000),
                );
                record.finished_at = index;
                record
            })
            .collect::<Vec<_>>();
        let samples = api_quality_samples(completed.iter().chain(std::iter::once(&live)));
        assert_eq!(samples.len(), 5);
        assert_eq!(samples[0].key, "completed-1");
        assert_eq!(samples[4].key, "live");

        live.api_request_outcome = Some(ApiRequestOutcome::Failed);
        assert_eq!(api_quality(std::iter::once(&live)), ApiQualityLevel::Failed);
        assert_eq!(api_quality_score(&live), Some(0.));
        live.api_request_outcome = Some(ApiRequestOutcome::Cancelled);
        assert!(api_quality_samples(std::iter::once(&live)).is_empty());
    }

    #[test]
    fn live_snapshot_replaces_its_record_and_unknowns_are_not_zero() {
        let records = [
            turn("old", Some(100), Some(80), Some(20)),
            turn("live", Some(10), Some(0), None),
        ];
        let current = turn("live", Some(200), Some(160), Some(40));
        let (total, latest) = consumption(records.iter(), Some(&current));
        assert_eq!(total.input, Some(300));
        assert_eq!(total.output, Some(60));
        assert_eq!(total.turns, 2);
        assert_eq!(total.cache_hit(), Some(80.));
        assert_eq!(latest.unwrap().key, "live");
        let unknown = [turn("unknown", None, None, None)];
        let (total, _) = consumption(unknown.iter(), None);
        assert_eq!(total.input, None);
        assert_eq!(total.output, None);
        assert_eq!(total.cache_hit(), None);
        assert!(total.incomplete);
    }

    #[test]
    fn cache_hit_uses_only_turns_with_known_input_and_cache() {
        let records = [
            turn("known", Some(100), Some(80), Some(20)),
            turn("uncached-unknown", Some(200), None, Some(40)),
            turn("input-unknown", None, Some(50), Some(10)),
        ];
        let (total, _) = consumption(records.iter(), None);
        assert_eq!(total.input, Some(300));
        assert_eq!(total.cached, Some(130));
        assert_eq!(total.output, Some(70));
        assert_eq!(total.cache_hit(), Some(80.));
    }

    #[test]
    fn cache_hit_excludes_invalid_turns() {
        let invalid = [turn("invalid", Some(100), Some(200), Some(20))];
        assert_eq!(consumption(invalid.iter(), None).0.cache_hit(), None);
        let known = turn("known", Some(100), Some(80), Some(20));
        let (total, _) = consumption(invalid.iter(), Some(&known));
        assert_eq!(total.input, Some(200));
        assert_eq!(total.cached, Some(280));
        assert_eq!(total.cache_hit(), Some(80.));
    }

    #[test]
    fn cache_writes_replace_live_snapshots_and_preserve_unknowns() {
        let mut old = turn("old", Some(100), Some(80), Some(20));
        old.cache_write = Some(10);
        let mut recorded_live = turn("live", Some(100), Some(80), Some(20));
        recorded_live.cache_write = Some(1);
        let mut current = recorded_live.clone();
        current.cache_write = Some(20);
        let (total, latest) = consumption([old, recorded_live].iter(), Some(&current));
        assert_eq!(total.cache_write, Some(30));
        assert_eq!(latest.unwrap().cache_write, Some(20));
        assert_eq!(total.cache_hit(), Some(80.));
        current.cache_write = None;
        assert_eq!(
            consumption(std::iter::empty(), Some(&current))
                .0
                .cache_write,
            None
        );
        current.cache_write = Some(0);
        assert_eq!(
            consumption(std::iter::empty(), Some(&current))
                .0
                .cache_write,
            Some(0)
        );
    }

    #[test]
    fn cache_hit_does_not_saturate_its_weighted_denominator() {
        let records = [
            turn("cached", Some(u64::MAX), Some(u64::MAX), Some(0)),
            turn("uncached", Some(u64::MAX), Some(0), Some(0)),
        ];
        assert_eq!(consumption(records.iter(), None).0.cache_hit(), Some(50.));
    }

    #[test]
    fn cache_hit_is_weighted_by_eligible_input() {
        let records = [
            turn("small", Some(100), Some(80), Some(20)),
            turn("large", Some(300), Some(60), Some(40)),
        ];
        assert_eq!(consumption(records.iter(), None).0.cache_hit(), Some(35.));
    }

    #[test]
    fn live_turn_without_usage_keeps_known_cache_hit() {
        let records = [turn("known", Some(100), Some(80), Some(20))];
        let current = turn("live", None, None, None);
        let (total, latest) = consumption(records.iter(), Some(&current));
        assert_eq!(total.input, Some(100));
        assert_eq!(total.cached, Some(80));
        assert_eq!(total.turns, 2);
        assert_eq!(total.cache_hit(), Some(80.));
        assert_eq!(latest.unwrap().key, "live");
    }

    #[test]
    fn cache_hit_distinguishes_zero_from_unknown() {
        let known_zero = [turn("zero", Some(100), Some(0), Some(20))];
        assert_eq!(consumption(known_zero.iter(), None).0.cache_hit(), Some(0.));
        let unknown = [turn("unknown", Some(100), None, Some(20))];
        assert_eq!(consumption(unknown.iter(), None).0.cache_hit(), None);
        let no_input = [turn("empty", Some(0), Some(0), Some(20))];
        assert_eq!(consumption(no_input.iter(), None).0.cache_hit(), None);
    }
}
