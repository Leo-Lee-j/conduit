//! Session inspector. Consumption comes from deduplicated turn observations;
//! current context always remains a separate provider snapshot.
use super::right::{RightBody, RightView};
use super::*;
use crate::app::state::{PlanStatus, plan_from_messages};
use crate::subagents::{Agent, Status as AgentStatus};
use crate::usage_ledger::Observation;
use conduit::plugins::{context::ScopeKey, state::ViewInstanceKey};
use gpui_kit::component::{
    empty::{Empty, EmptyDescription, EmptyHeader, EmptyMedia},
    shimmer::ShimmerText,
    tooltip::Tooltip,
};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub(super) struct Sections {
    tokens: bool,
    tools: bool,
    timing: bool,
    scroll: ScrollHandle,
    tool_stats: RefCell<Option<(ToolStatsKey, Rc<ToolStats>)>>,
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
    output: Option<u64>,
    elapsed_ms: u64,
    turns: usize,
    incomplete: bool,
    cache_incomplete: bool,
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
        add_known(&mut self.output, turn.output);
        self.elapsed_ms = self.elapsed_ms.saturating_add(turn.elapsed_ms);
        self.turns += 1;
        self.incomplete |= turn.incomplete || turn.input.is_none() || turn.output.is_none();
        self.cache_incomplete |= match (turn.input, turn.cached) {
            (Some(input), Some(cached)) => cached > input,
            _ => true,
        };
    }
    fn cache_hit(self) -> Option<f64> {
        if self.cache_incomplete {
            return None;
        }
        match (self.cached, self.input) {
            (Some(cached), Some(input)) if input > 0 && cached <= input => {
                Some(cached as f64 * 100. / input as f64)
            }
            _ => None,
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
fn caption(value: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(11.))
        .text_color(rgb(palette().muted))
        .child(value.into())
}
fn metric(label: &'static str, value: String, detail: impl IntoElement) -> Div {
    card()
        .flex_1()
        .min_w_0()
        .gap_1()
        .p_3()
        .child(caption(text(label)).truncate())
        .child(
            div()
                .text_size(ui_text_size(23.))
                .font_semibold()
                .truncate()
                .child(value),
        )
        .child(detail)
}
fn usage_row(label: &'static str, input: Option<u64>, output: Option<u64>, cx: &App) -> Div {
    let value = |value| {
        div()
            .w(px(58.))
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
        .child(value(input))
        .child(value(output))
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
                            div()
                                .text_size(ui_text_size(13.))
                                .font_semibold()
                                .child(text(label)),
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
    let tooltip = SharedString::from(
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
        .role(Role::Group)
        .aria_label(tooltip.clone())
        .w_full()
        .min_w_0()
        .min_h(px(40.))
        .p_2()
        .gap_2()
        .items_start()
        .rounded(px(8.))
        .bg(rgb(colors.surface_strong))
        .hover(|row| row.bg(rgb(colors.surface_hover)))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
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
                    tokens: true,
                    tools: true,
                    timing: true,
                    ..Sections::default()
                })
            })
            .clone();
        let Some(session) = self.session(*id) else {
            return RightBody::Loading.into();
        };
        let run = self.session_control.runs.get(id);
        let plan = self
            .session_control
            .plans
            .get(id)
            .filter(|steps| !steps.is_empty())
            .cloned()
            .unwrap_or_else(|| {
                if run.is_none() && session.transcript.loaded {
                    plan_from_messages(session)
                } else {
                    Vec::new()
                }
            });
        let expanded = state.read(cx);
        let scroll = expanded.scroll.clone();
        let show_tokens = expanded.tokens;
        let show_tools = expanded.tools;
        let show_timing = expanded.timing;
        let usage = run.map_or(session.usage, |run| run.usage);
        let current = run.and_then(|run| self.turn_usage_observation(*id, run, false));
        let (total, latest) = consumption(
            self.store
                .usage_ledger
                .observations
                .iter()
                .filter(|turn| turn.session == *id && turn.workspace == workspace.path()),
            current.as_ref(),
        );
        let model = match run {
            Some(run) => run.model.as_deref(),
            None => session.model.as_deref(),
        }
        .unwrap_or(text("session_data.cli_default"));
        let effort = run.map_or(session.reasoning_effort, |run| run.reasoning_effort);
        let variant = run
            .and_then(|run| run.acp_variant.as_deref())
            .or_else(|| session.opencode.effective_variant());
        let status = if let Some(run) = run {
            if run.runtime.is_cancelled() {
                text("session_data.stopping")
            } else if !run.approvals.is_empty() {
                run.waiting_status()
            } else {
                text("session_data.running")
            }
        } else if session.interrupted {
            text("session_data.interrupted")
        } else {
            text("session_data.idle")
        };
        let color = if status == text("session_data.running") {
            palette().activity_completed
        } else if run.is_some() || session.interrupted {
            palette().warning_text
        } else {
            palette().muted
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
                            .child(
                                div()
                                    .size(px(6.))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(color)),
                            )
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
                    .child(caption(text("session_data.context")))
                    .when(
                        usage.context_ratio().is_some() && usage.context_window_estimated,
                        |header| header.child(caption(text("session_data.estimated"))),
                    ),
            );
        if let Some(ratio) = usage.context_ratio() {
            let color = if ratio >= 0.9 {
                palette().error_text
            } else if ratio >= 0.75 {
                palette().warning_text
            } else {
                palette().highlight_text
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
                            v_flex()
                                .items_end()
                                .gap_0()
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
                                )
                                .child(caption(text("session_data.tokens_unit"))),
                        ),
                )
                .child(
                    progress::Progress::new("session-context-progress")
                        .value((ratio * 100.).clamp(0., 100.) as f32)
                        .color(rgb(color))
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
                        .when(ratio >= 0.9, |row| {
                            row.child(
                                caption(text("session_data.context_high")).text_color(rgb(color)),
                            )
                        }),
                );
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
        if total.turns == 0 {
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
            let mut summary = card()
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
                        .child(caption(text("session_data.usage")).flex_1())
                        .child(caption(text("session_data.input")).w(px(58.)).text_right())
                        .child(caption(text("session_data.output")).w(px(58.)).text_right()),
                )
                .child(usage_row(
                    if run.is_some() {
                        "session_data.current_turn"
                    } else {
                        "session_data.latest_turn"
                    },
                    turn.and_then(|turn| turn.input),
                    turn.and_then(|turn| turn.output),
                    cx,
                ))
                .child(divider())
                .child(usage_row(
                    "session_data.recorded_total",
                    total.input,
                    total.output,
                    cx,
                ));
            if total.incomplete {
                summary = summary.child(
                    caption(text("session_data.partial_note"))
                        .text_color(rgb(palette().warning_text)),
                );
            }
            content = content.child(summary);
        }

        let token_summary = conduit::i18n::format(
            "session_data.cache_read_summary",
            &[("count", tokens(total.cached))],
        );
        let mut details = card().child(section(
            "session-data-token-toggle",
            "session_data.token_details",
            token_summary,
            show_tokens,
            &state,
            |state| state.tokens = !state.tokens,
            cx,
        ));
        if show_tokens {
            details = details.child(
                disclosure_details()
                    .id("session-data-token-details")
                    .test_support()
                    .debug_selector(|| "session-data-token-details".into())
                    .child(row("session_data.input", tokens(total.input), cx))
                    .child(row("session_data.cache_read", tokens(total.cached), cx))
                    .child(row("session_data.output", tokens(total.output), cx))
                    .child(caption(text("session_data.cache_subset")))
                    .child(caption(text("session_data.recorded_note"))),
            );
        }

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
                    ("agents", active_agents.to_string()),
                ],
            )
        } else {
            text("session_data.loading_history").into()
        };
        let mut tools = details.child(divider()).child(section(
            "session-data-tool-toggle",
            "session_data.tools_tasks",
            tool_summary,
            show_tools,
            &state,
            |state| state.tools = !state.tools,
            cx,
        ));
        if show_tools {
            let mut tool_details = disclosure_details();
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
            let mut agents = v_flex().px_3().pb_3().gap_2().child(row(
                "session_data.active_agents",
                active_agents.to_string(),
                cx,
            ));
            for agent in session
                .subagents
                .iter()
                .filter(|agent| {
                    run.is_some_and(|run| agent.run == run.prefix) && agent.status.active()
                })
                .take(5)
            {
                agents = agents.child(agent_row(agent));
            }
            tools = tools.child(tool_details);
            if active_agents > 0 {
                tools = tools.child(agents);
            }
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
            for step in &plan {
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
                                .truncate()
                                .text_color(rgb(if step.status == PlanStatus::Completed {
                                    palette().muted
                                } else {
                                    palette().text
                                }))
                                .child(step.text.clone()),
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
                    ))
                    .child(caption(text("session_data.speed_note"))),
            );
        }
        if !plan.is_empty()
            || total.turns > 0
            || stats.is_some_and(|stats| stats.count > 0)
            || active_agents > 0
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
    use super::{Observation, consumption};
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
            output,
            elapsed_ms: 1000,
            incomplete: false,
        }
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
    fn cache_hit_requires_matching_input_and_cache_coverage() {
        let records = [
            turn("known", Some(100), Some(80), Some(20)),
            turn("uncached-unknown", Some(200), None, Some(40)),
        ];
        let (total, _) = consumption(records.iter(), None);
        assert_eq!(total.input, Some(300));
        assert_eq!(total.cache_hit(), None);
        let invalid = [turn("invalid", Some(100), Some(200), Some(20))];
        assert_eq!(consumption(invalid.iter(), None).0.cache_hit(), None);
    }
}
