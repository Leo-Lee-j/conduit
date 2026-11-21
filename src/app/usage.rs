//! Known usage and advisory token budgets; no inferred monetary billing.
use super::*;
use crate::usage_ledger::{ApiRequestOutcome, Observation, Totals};
use chrono::{Datelike, Months};
use gpui_kit::component::{chart::BarChart, progress::Progress, tag::Tag};
mod report;
use report::{Report, service_name};

#[derive(Default)]
pub(super) struct UsageUi {
    pub(super) active: Option<u64>,
    pub(super) panels: HashMap<u64, Entity<UsagePanel>>,
    pub(super) roots: HashMap<u64, PathBuf>,
    next_id: u64,
}

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

impl ConduitApp {
    pub(super) fn turn_usage_observation(
        &self,
        id: u64,
        run: &ActiveRun,
        incomplete: bool,
    ) -> Option<Observation> {
        if matches!(run.intent, crate::commands::RequestIntent::Discover) {
            return None;
        }
        let session = self.session(id)?;
        let provider = session.provider;
        let workspace = self
            .store
            .workspaces
            .iter()
            .find(|w| w.sessions.iter().any(|s| s.id == id))
            .map(|w| w.path.clone())
            .unwrap_or_else(|| run.worktree.clone());
        let reported = run.usage.turn_elapsed_ms.is_some();
        let input = (reported && run.usage.input_tokens_known)
            .then(|| {
                if provider == Provider::Codex {
                    run.usage_input_baseline
                        .and_then(|base| run.usage.input_tokens.checked_sub(base))
                } else {
                    Some(run.usage.input_tokens)
                }
            })
            .flatten();
        let cached = (reported && run.usage.cached_input_tokens_known)
            .then(|| {
                if provider == Provider::Codex {
                    run.usage_cached_baseline
                        .and_then(|base| run.usage.cached_input_tokens.checked_sub(base))
                } else {
                    Some(run.usage.cached_input_tokens)
                }
            })
            .flatten();
        Some(Observation {
            key: run.usage_key.clone(),
            session: id,
            workspace: workspace.clone(),
            provider,
            service: run.usage_service.clone(),
            service_name: service_name(provider, &run.usage_service, &self.store.providers),
            model: run.model.clone(),
            finished_at: unix_now(),
            input,
            cached,
            output: (reported && run.usage.output_tokens_known)
                .then_some(run.usage.turn_output_tokens)
                .flatten(),
            elapsed_ms: run.turn_started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            incomplete,
            api_first_response_ms: run.api_first_response_ms,
            api_request_outcome: None,
        })
    }

    pub(super) fn record_turn_usage(
        &mut self,
        id: u64,
        run: &ActiveRun,
        incomplete: bool,
        api_outcome: ApiRequestOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(mut observation) = self.turn_usage_observation(id, run, incomplete) else {
            return;
        };
        if matches!(run.intent, crate::commands::RequestIntent::Message)
            && run.api_request_started.is_some()
        {
            observation.api_request_outcome = Some(api_outcome);
        }
        let workspace = observation.workspace.clone();
        self.store.usage_ledger.record(observation);
        self.check_usage_budget(&workspace, cx);
    }

    fn check_usage_budget(&mut self, workspace: &Path, cx: &mut Context<Self>) {
        let month = Local::now().format("%Y-%m").to_string();
        if let Some((total, limit)) = self.store.usage_ledger.take_alert(workspace, &month) {
            self.notify_user(
                Notice::new(
                    NoticeSource::App,
                    NoticeLevel::Warning,
                    label("usage.budget_reached"),
                    conduit::i18n::format(
                        "usage.budget_message",
                        &[
                            ("workspace", workspace.display().to_string()),
                            ("month", month.clone()),
                            ("total", total.to_string()),
                            ("limit", limit.to_string()),
                        ],
                    ),
                )
                .target(NoticeTarget::Workspace(workspace.to_path_buf()))
                .key(format!("usage/{}/{month}", workspace.display()), "budget")
                .presentation(NoticePresentation::Toast),
                cx,
            );
        }
    }

    pub(super) fn usage_button(
        &self,
        cx: &mut Context<Self>,
        popover: Option<WeakEntity<gpui_kit::component::popover::PopoverState>>,
    ) -> Button {
        Button::new("usage-ledger")
            .cursor_pointer()
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::ChartColumn)
            .accessibility_label(label("usage.title"))
            .tooltip(label("usage.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(popover) = &popover {
                    let _ = popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                }
                this.open_usage(window, cx);
            }))
    }

    pub(super) fn open_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        if let Some(id) = self
            .usage_ui
            .roots
            .iter()
            .find(|(_, root)| **root == workspace)
            .map(|(id, _)| *id)
        {
            self.select_document(files::DocumentTab::Usage(id), window, cx);
            return;
        }
        let budget = self
            .store
            .usage_ledger
            .budgets
            .iter()
            .find(|b| b.workspace == workspace)
            .map_or(0, |b| b.monthly_tokens);
        let host = cx.entity();
        let panel = cx.new(|cx| {
            let month = cx.new(|cx| {
                InputState::new(window, cx).default_value(Local::now().format("%Y-%m").to_string())
            });
            let budget =
                cx.new(|cx| InputState::new(window, cx).default_value(format_budget(budget)));
            let subscriptions = vec![
                cx.subscribe(
                    &budget,
                    |this: &mut UsagePanel, _, event: &InputEvent, cx| {
                        if !matches!(event, InputEvent::Change) {
                            return;
                        }
                        this.error = None;
                        cx.notify();
                    },
                ),
                cx.observe(&host, |_, _, cx| cx.notify()),
                cx.subscribe(
                    &month,
                    |this: &mut UsagePanel, _, event: &InputEvent, cx| {
                        if !matches!(event, InputEvent::Change) {
                            return;
                        }
                        this.selected_service = None;
                        this.shown = 20;
                        this.model_details.clear();
                        cx.notify();
                    },
                ),
            ];
            UsagePanel {
                host: host.downgrade(),
                workspace: workspace.clone(),
                month,
                budget,
                error: None,
                selected_service: None,
                shown: 20,
                by_service: false,
                model_details: HashSet::new(),
                _subscriptions: subscriptions,
            }
        });
        self.usage_ui.next_id += 1;
        let id = self.usage_ui.next_id;
        self.usage_ui.panels.insert(id, panel);
        self.usage_ui.roots.insert(id, workspace);
        self.open_tabs.push(files::DocumentTab::Usage(id));
        self.select_document(files::DocumentTab::Usage(id), window, cx);
    }
}

pub(super) struct UsagePanel {
    host: WeakEntity<ConduitApp>,
    pub(super) workspace: PathBuf,
    month: Entity<InputState>,
    budget: Entity<InputState>,
    error: Option<String>,
    selected_service: Option<String>,
    shown: usize,
    by_service: bool,
    model_details: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}
impl UsagePanel {
    pub(super) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.month.read(cx).focus_handle(cx).focus(window, cx);
    }
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let Some(limit) = parse_budget(&self.budget.read(cx).value()) else {
            self.error = Some(label("usage.invalid_budget").into());
            cx.notify();
            return;
        };
        host.update(cx, |app, cx| {
            if !app.writable || app.closing {
                return;
            }
            app.store
                .usage_ledger
                .save_budget(self.workspace.clone(), limit);
            app.check_usage_budget(&self.workspace, cx);
            app.persist();
            cx.notify();
        });
        self.error = None;
        cx.notify();
    }

    fn shift_month(&mut self, amount: i32, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.month.read(cx).value().to_string();
        let Some(date) =
            chrono::NaiveDate::parse_from_str(&format!("{current}-01"), "%Y-%m-%d").ok()
        else {
            return;
        };
        let months = Months::new(amount.unsigned_abs());
        let date = if amount < 0 {
            date.checked_sub_months(months)
        } else {
            date.checked_add_months(months)
        };
        let Some(date) = date else { return };
        self.month.update(cx, |input, cx| {
            input.set_value(date.format("%Y-%m").to_string(), window, cx)
        });
        self.selected_service = None;
        self.shown = 20;
        self.model_details.clear();
        cx.notify();
    }

    fn set_budget(&mut self, value: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.budget.update(cx, |input, cx| {
            input.set_value(format_budget(value), window, cx)
        });
        self.error = None;
        cx.notify();
    }
}

fn format_tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.)
    } else {
        value.to_string()
    }
}
fn format_budget(value: u64) -> String {
    if value == 0 {
        String::new()
    } else {
        format!("{:.6}", value as f64 / 1_000_000.)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}
fn parse_budget(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return Some(0);
    }
    let millions = value.parse::<f64>().ok()?;
    let tokens = millions * 1_000_000.;
    (millions.is_finite() && millions >= 0. && tokens <= u64::MAX as f64)
        .then(|| tokens.round() as u64)
}

fn card() -> Div {
    v_flex()
        .min_w_0()
        .gap_3()
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
}
fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(11.))
        .text_color(rgb(palette().muted))
        .child(text.into())
}
fn usage_stat(title: &'static str, value: String) -> Div {
    v_flex().gap_1().child(caption(label(title))).child(
        div()
            .text_size(ui_text_size(16.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(value),
    )
}
fn composition(total: &Totals) -> Div {
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(composition_metric(
            "usage.input",
            total.input.saturating_sub(total.cached),
            total.tokens(),
        ))
        .child(composition_metric(
            "usage.output",
            total.output,
            total.tokens(),
        ))
        .child(composition_metric(
            "usage.cached",
            total.cached,
            total.tokens(),
        ))
}
fn composition_metric(title: &'static str, value: u64, total: u64) -> Div {
    v_flex()
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(palette().border))
        .child(caption(label(title)))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format_tokens(value)),
        )
        .child(caption(share(value, total)))
}
fn budget_preset(value: u64, current: u64, disabled: bool) -> Button {
    let millions = value / 1_000_000;
    Button::new(("usage-budget-preset", millions))
        .small()
        .outline()
        .selected(current == value)
        .disabled(disabled)
        .label(if millions == 30 {
            format!("{millions}M · {}", label("usage.recommended"))
        } else {
            format!("{millions}M")
        })
}
fn share(value: u64, total: u64) -> String {
    if total == 0 {
        "—".into()
    } else {
        format!("{:.1}%", value as f64 * 100. / total as f64)
    }
}
fn service_title(service: &report::Service) -> String {
    service.name.clone().unwrap_or_else(|| {
        format!(
            "{} · {}",
            service.engine.label(),
            label(if service.cli {
                "usage.unattributed_cli"
            } else {
                "usage.unavailable_provider"
            })
        )
    })
}
impl Render for UsagePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let workspace_name = app
            .store
            .workspaces
            .iter()
            .find(|w| w.path == self.workspace)
            .map(Workspace::name);
        let width = f32::from(window.viewport_size().width)
            - app.sidebar_width
            - 6.
            - if window.viewport_size().width >= px(1100.) && app.right_view_visible() {
                app.plugins.dock.width
            } else {
                0.
            };
        let month = self.month.read(cx).value().to_string();
        let first = (month.len() == 7)
            .then(|| chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").ok())
            .flatten();
        let limit = app
            .store
            .usage_ledger
            .budgets
            .iter()
            .find(|b| b.workspace == self.workspace)
            .map_or(0, |b| b.monthly_tokens);
        let changed = parse_budget(&self.budget.read(cx).value()) != Some(limit);
        let previous = Button::new("usage-previous-month")
            .small()
            .ghost()
            .label("‹")
            .accessibility_label(label("usage.previous_month"))
            .on_click(cx.listener(|this, _, window, cx| this.shift_month(-1, window, cx)));
        let next = Button::new("usage-next-month")
            .small()
            .ghost()
            .label("›")
            .accessibility_label(label("usage.next_month"))
            .on_click(cx.listener(|this, _, window, cx| this.shift_month(1, window, cx)));
        let header = h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(220.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label("usage.title")),
                    )
                    .when_some(workspace_name, |title, name| {
                        title.child(
                            h_flex().min_w_0().child(
                                Tag::secondary()
                                    .small()
                                    .max_w_full()
                                    .min_w_0()
                                    .text_size(ui_text_size(11.))
                                    .child(div().min_w_0().truncate().child(name)),
                            ),
                        )
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .p_1()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .child(previous)
                    .child(div().w(px(150.)).child(Input::new(&self.month)))
                    .child(next),
            );
        let mut content = v_flex().w_full().max_w(px(1200.)).gap_4().child(header);
        if let Some(error) = app.persistence.error.as_ref() {
            content = content.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if app.store.usage_ledger.truncated {
            content = content
                .child(caption(label("usage.truncated")).text_color(rgb(palette().warning_text)));
        }
        if let Some(first) = first {
            let report = Report::build(
                &app.store.usage_ledger.observations,
                &self.workspace,
                &month,
                &app.store.providers,
                self.selected_service.as_deref(),
            );
            let totals = &report.totals;
            let now = Local::now();
            let days_in_month = first
                .checked_add_months(Months::new(1))
                .map(|next| (next - chrono::Days::new(1)).day())
                .unwrap_or(30);
            let today = now.date_naive();
            let (status_date, days_elapsed) = if month == now.format("%Y-%m").to_string() {
                (today, now.day().min(days_in_month))
            } else if first < today {
                (
                    first + chrono::Days::new(u64::from(days_in_month - 1)),
                    days_in_month,
                )
            } else {
                (first, 0)
            };
            let daily_average = totals.tokens() / u64::from(days_elapsed.max(1));
            let forecast = if month == now.format("%Y-%m").to_string() {
                daily_average.saturating_mul(u64::from(days_in_month))
            } else if first < today {
                totals.tokens()
            } else {
                0
            };
            let month_status = if days_elapsed == 0 {
                label("usage.month_not_started").to_owned()
            } else {
                conduit::i18n::format(
                    "usage.month_progress",
                    &[
                        ("date", status_date.format("%Y-%m-%d").to_string()),
                        ("day", days_elapsed.to_string()),
                        ("days", days_in_month.to_string()),
                    ],
                )
            };
            let (summary_value, summary_unit) = if totals.tokens() >= 1_000_000 {
                (
                    format!("{:.1}", totals.tokens() as f64 / 1_000_000.),
                    "M Token",
                )
            } else if totals.tokens() >= 1_000 {
                (format!("{:.1}", totals.tokens() as f64 / 1_000.), "K Token")
            } else {
                (totals.tokens().to_string(), "Token")
            };
            let mut summary = v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .justify_between()
                        .flex_wrap()
                        .gap_2()
                        .child(caption(label("usage.month_usage")))
                        .child(caption(month_status)),
                )
                .child(
                    h_flex()
                        .items_end()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(40.))
                                .font_weight(FontWeight::BOLD)
                                .child(summary_value),
                        )
                        .child(
                            div()
                                .pb_1()
                                .text_color(rgb(palette().muted))
                                .child(summary_unit),
                        ),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_6()
                        .child(usage_stat(label("usage.turns"), totals.turns.to_string()))
                        .child(usage_stat(
                            label("usage.average_per_turn"),
                            format_tokens(totals.tokens() / totals.turns.max(1) as u64),
                        ))
                        .child(usage_stat(
                            label("usage.average_per_day"),
                            format_tokens(daily_average),
                        )),
                );
            if totals.incomplete > 0 {
                summary = summary.child(
                    caption(conduit::i18n::format(
                        "usage.partial_note",
                        &[("count", totals.incomplete.to_string())],
                    ))
                    .text_color(rgb(palette().warning_text)),
                );
            }
            let scope = report
                .services
                .iter()
                .find(|service| Some(&service.key) == self.selected_service.as_ref())
                .map(service_title)
                .unwrap_or_else(|| label("usage.all_providers").into());
            let mut trend = card().flex_1().min_w_0().child(summary).child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(label("usage.trend")),
                    )
                    .child(caption(format!("{} · Token", scope))),
            );
            if report.filtered.turns > 0 {
                let data: Vec<_> = first
                    .iter_days()
                    .take_while(|day| day.format("%Y-%m").to_string() == month)
                    .map(|day| {
                        let day = day.format("%Y-%m-%d").to_string();
                        let tokens = report.daily.get(&day).map_or(0, Totals::tokens);
                        (day, tokens)
                    })
                    .collect();
                trend = trend.child(
                    div().h(px(190.)).w_full().child(
                        BarChart::new(data)
                            .id("usage-daily-chart")
                            .band(|d: &(String, u64)| SharedString::from(d.0[8..].to_owned()))
                            .value(|d| d.1 as f64)
                            .name(label("usage.total_tokens"))
                            .fill(|_, _, _, _| rgb(palette().activity_running))
                            .value_axis(true)
                            .value_tick_count(3)
                            .value_tick_format(|v| format_tokens(v as u64))
                            .band_tick_count(7)
                            .grid_dashed(false)
                            .appear(false)
                            .tooltip_title(|d| d.0.clone().into())
                            .tooltip_value(|d, _| format!("{} Token", format_tokens(d.1)).into()),
                    ),
                );
            } else {
                trend = trend.child(
                    div()
                        .h(px(190.))
                        .items_center()
                        .justify_center()
                        .child(label("usage.empty")),
                );
            }
            let ratio = if limit == 0 {
                0.
            } else {
                totals.tokens() as f64 / limit as f64 * 100.
            };
            let mut budget =
                card()
                    .w(px(320.))
                    .min_w(px(260.))
                    .justify_between()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(caption(label("usage.budget")))
                            .child(
                                caption(if limit == 0 {
                                    label("usage.disabled").to_owned()
                                } else {
                                    label("usage.budget_set").to_owned()
                                })
                                .p_1()
                                .rounded_full()
                                .bg(rgb(palette().surface_hover)),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(ui_text_size(18.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(label("usage.budget_title")),
                            )
                            .child(caption(label("usage.budget_description"))),
                    )
                    .child(caption(if limit == 0 {
                        label("usage.budget_not_set").to_owned()
                    } else {
                        format!(
                            "{ratio:.1}% · {} / {} Token",
                            format_tokens(totals.tokens()),
                            format_tokens(limit)
                        )
                    }))
                    .child(
                        Progress::new("usage-budget-progress")
                            .value(ratio as f32)
                            .accessibility_label(label("usage.budget"))
                            .color(rgb(if ratio >= 100. {
                                palette().warning_text
                            } else {
                                palette().activity_running
                            })),
                    )
                    .child(caption(label("usage.quick_budget")))
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(budget_preset(20_000_000, limit, !writable).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.set_budget(20_000_000, window, cx);
                                    this.save(cx);
                                }),
                            ))
                            .child(budget_preset(30_000_000, limit, !writable).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.set_budget(30_000_000, window, cx);
                                    this.save(cx);
                                }),
                            ))
                            .child(budget_preset(50_000_000, limit, !writable).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.set_budget(50_000_000, window, cx);
                                    this.save(cx);
                                }),
                            )),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(caption(label("usage.custom_budget")))
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2()
                                            .child(Input::new(&self.budget).disabled(!writable))
                                            .child(caption("M Token")),
                                    ),
                            )
                            .child(
                                Button::new("usage-save-budget")
                                    .label(label(if changed {
                                        "usage.save_budget"
                                    } else {
                                        "usage.saved_budget"
                                    }))
                                    .disabled(!writable || !changed)
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .p_3()
                            .rounded_lg()
                            .bg(rgb(palette().surface_hover))
                            .child(caption(conduit::i18n::format(
                                "usage.daily_average_note",
                                &[("average", format_tokens(daily_average))],
                            )))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(
                                conduit::i18n::format(
                                    "usage.budget_forecast",
                                    &[("forecast", format_tokens(forecast))],
                                ),
                            ))
                            .child(caption(label("usage.budget_hint"))),
                    );
            if let Some(error) = &self.error {
                budget =
                    budget.child(caption(error.clone()).text_color(rgb(palette().danger_text)));
            }
            if ratio >= 100. {
                budget = budget.child(
                    caption(label("usage.budget_reached")).text_color(rgb(palette().warning_text)),
                );
            }
            let hero = if width >= 700. {
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(trend.flex_1())
                    .child(budget)
            } else {
                v_flex().w_full().gap_4().child(trend).child(budget)
            };
            content = content.child(hero);
            let mut breakdown = card().child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(ui_text_size(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(label("usage.breakdown_title")),
                            )
                            .child(caption(if self.by_service {
                                label("usage.providers_hint")
                            } else {
                                label("usage.models_hint")
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .p_1()
                            .rounded_lg()
                            .bg(rgb(palette().surface_hover))
                            .child(
                                Button::new("usage-group-model")
                                    .small()
                                    .selected(!self.by_service)
                                    .label(label("usage.by_model"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.by_service = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("usage-group-provider")
                                    .debug_selector(|| "usage-group-provider".into())
                                    .small()
                                    .selected(self.by_service)
                                    .label(label("usage.by_provider"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.by_service = true;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
            if self.by_service {
                if report.services.is_empty() {
                    breakdown = breakdown.child(div().py_5().child(label("usage.empty")));
                }
                for (index, service) in report.services.iter().enumerate() {
                    let key = service.key.clone();
                    let detail_key = format!("provider:{key}");
                    let expanded = self.model_details.contains(&detail_key);
                    let selected = self.selected_service.as_deref() == Some(key.as_str());
                    let title = service_title(service);
                    breakdown = breakdown.child(
                        v_flex()
                            .gap_2()
                            .py_3()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w(px(170.))
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .size(px(9.))
                                                    .rounded_full()
                                                    .bg(rgb(palette().activity_running)),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(title.clone()),
                                            ),
                                    )
                                    .child(caption(format!(
                                        "{} · {}",
                                        share(service.totals.tokens(), totals.tokens()),
                                        format_tokens(service.totals.tokens())
                                    )))
                                    .child(caption(format!(
                                        "{} {}",
                                        format_tokens(
                                            service.totals.tokens()
                                                / service.totals.turns.max(1) as u64
                                        ),
                                        label("usage.per_turn")
                                    )))
                                    .child(caption(format!(
                                        "{} {}",
                                        service.totals.turns,
                                        label("usage.turns")
                                    )))
                                    .child(
                                        Button::new(("usage-provider-detail", index))
                                            .small()
                                            .ghost()
                                            .label(label(if expanded {
                                                "usage.hide_composition"
                                            } else {
                                                "usage.show_composition"
                                            }))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.model_details.remove(&detail_key) {
                                                    this.model_details.insert(detail_key.clone());
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("usage-provider-filter", index))
                                            .debug_selector(move || {
                                                format!("usage-provider-{index}")
                                            })
                                            .small()
                                            .ghost()
                                            .selected(selected)
                                            .label(label(if selected {
                                                "usage.show_all"
                                            } else {
                                                "usage.inspect_provider"
                                            }))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.selected_service =
                                                    if selected { None } else { Some(key.clone()) };
                                                this.shown = 20;
                                                this.model_details.clear();
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Progress::new(("usage-provider-share", index))
                                    .small()
                                    .value(if totals.tokens() == 0 {
                                        0.
                                    } else {
                                        (service.totals.tokens() as f64 * 100.
                                            / totals.tokens() as f64)
                                            as f32
                                    })
                                    .accessibility_label(title),
                            )
                            .when(expanded, |row| row.child(composition(&service.totals)))
                            .when(service.totals.incomplete > 0, |row| {
                                row.child(
                                    caption(conduit::i18n::format(
                                        "usage.incomplete",
                                        &[("count", service.totals.incomplete.to_string())],
                                    ))
                                    .text_color(rgb(palette().warning_text)),
                                )
                            }),
                    );
                }
            } else {
                if report.models.is_empty() {
                    breakdown = breakdown.child(div().py_5().child(label("usage.empty")));
                }
                for (index, (model, total)) in report.models.iter().take(self.shown).enumerate() {
                    let model_label = model
                        .clone()
                        .unwrap_or_else(|| label("usage.unknown_model").into());
                    let detail_key = format!("model:{}", model.as_deref().unwrap_or(""));
                    let expanded = self.model_details.contains(&detail_key);
                    let share_value = if report.filtered.tokens() == 0 {
                        0.
                    } else {
                        total.tokens() as f64 * 100. / report.filtered.tokens() as f64
                    };
                    breakdown = breakdown.child(
                        v_flex()
                            .gap_2()
                            .py_3()
                            .border_t_1()
                            .border_color(rgb(palette().border))
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w(px(180.))
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .size(px(9.))
                                                    .rounded_full()
                                                    .bg(rgb(palette().activity_running)),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(model_label),
                                            ),
                                    )
                                    .child(
                                        div().w(px(150.)).child(
                                            Progress::new(("usage-model-share", index))
                                                .small()
                                                .value(share_value as f32)
                                                .accessibility_label(label("usage.usage_share")),
                                        ),
                                    )
                                    .child(caption(format!("{share_value:.1}%")))
                                    .child(
                                        div()
                                            .w(px(82.))
                                            .text_right()
                                            .font_semibold()
                                            .child(format_tokens(total.tokens())),
                                    )
                                    .child(caption(format!(
                                        "{} {}",
                                        format_tokens(total.tokens() / total.turns.max(1) as u64),
                                        label("usage.per_turn")
                                    )))
                                    .child(caption(format!(
                                        "{} {}",
                                        total.turns,
                                        label("usage.turns")
                                    )))
                                    .child(
                                        Button::new(("usage-model-details", index))
                                            .small()
                                            .ghost()
                                            .label(label(if expanded {
                                                "usage.hide_composition"
                                            } else {
                                                "usage.show_composition"
                                            }))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if !this.model_details.remove(&detail_key) {
                                                    this.model_details.insert(detail_key.clone());
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .when(expanded, |row| row.child(composition(total)))
                            .when(total.incomplete > 0, |row| {
                                row.child(
                                    caption(conduit::i18n::format(
                                        "usage.incomplete",
                                        &[("count", total.incomplete.to_string())],
                                    ))
                                    .text_color(rgb(palette().warning_text)),
                                )
                            }),
                    );
                }
                if report.models.len() > self.shown {
                    breakdown = breakdown.child(
                        Button::new("usage-more")
                            .small()
                            .label(label("usage.more"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.shown += 20;
                                cx.notify();
                            })),
                    );
                }
            }
            content = content.child(breakdown);
            let mut sessions = card()
                .child(
                    div()
                        .text_size(ui_text_size(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label("usage.sessions_title")),
                )
                .child(caption(conduit::i18n::format(
                    "usage.sessions_hint",
                    &[("scope", scope.clone())],
                )));
            for (id, total) in report.sessions.iter().take(5) {
                let session = app.session(*id);
                let title = session
                    .map(|s| s.title.clone())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| label("usage.unavailable_session").into());
                let id = *id;
                let host = self.host.clone();
                sessions = sessions.child(
                    h_flex()
                        .flex_wrap()
                        .justify_between()
                        .gap_3()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .gap_1()
                                .child(div().truncate().child(title))
                                .child(caption(format!(
                                    "{} Token · {}",
                                    format_tokens(total.tokens()),
                                    share(total.tokens(), report.filtered.tokens())
                                ))),
                        )
                        .child(
                            Button::new(("usage-open-session", id))
                                .debug_selector(move || format!("usage-open-session-{id}"))
                                .small()
                                .ghost()
                                .label(label("usage.open_session"))
                                .disabled(session.is_none_or(|s| s.archived))
                                .on_click(move |_, window, cx| {
                                    let _ = host.update(cx, |app, cx| {
                                        app.select_document(
                                            files::DocumentTab::Chat(id),
                                            window,
                                            cx,
                                        )
                                    });
                                }),
                        ),
                );
            }
            if report.sessions.is_empty() {
                sessions = sessions.child(caption(label("usage.empty")));
            }
            content = content.child(sessions);
        } else {
            content = content.child(
                card().child(
                    div()
                        .text_color(rgb(palette().danger_text))
                        .child(label("usage.invalid_month")),
                ),
            );
        }
        content = content.child(caption(label("usage.coverage")));
        v_flex()
            .id("usage-ledger-panel")
            .debug_selector(|| "usage-ledger-panel".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .p_5()
            .items_center()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .text_size(ui_text_size(13.))
            .child(content)
            .into_any_element()
    }
}

#[cfg(test)]
mod budget_tests {
    use super::{format_budget, parse_budget};

    #[test]
    fn budget_input_uses_millions_and_round_trips_token_counts() {
        for tokens in [0, 1, 12_345, 1_000_000, 30_000_000, u64::MAX] {
            assert_eq!(parse_budget(&format_budget(tokens)), Some(tokens));
        }
        assert_eq!(parse_budget(""), Some(0));
        assert_eq!(parse_budget("0.5"), Some(500_000));
        assert_eq!(parse_budget("-1"), None);
        assert_eq!(parse_budget("NaN"), None);
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{Observation, Provider, files, unix_now};
    use gpui_kit::{Modifiers, MouseButton, TestAppContext};

    #[gpui_kit::test]
    fn usage_tabs_reuse_workspace_preserve_inputs_and_close(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (host, cx, _) = super::super::queue::tests::fixture(cx, "usage-tabs");
        cx.update(|window, cx| {
            host.update(cx, |app, cx| {
                app.startup = None;
                app.open_usage(window, cx);
                let first = app.usage_ui.active.unwrap();
                let panel = app.usage_ui.panels[&first].clone();
                panel.update(cx, |panel, cx| {
                    panel
                        .month
                        .update(cx, |input, cx| input.set_value("2026-01", window, cx));
                    panel
                        .budget
                        .update(cx, |input, cx| input.set_value("0.012345", window, cx));
                });
                app.open_usage(window, cx);
                assert_eq!(app.usage_ui.panels.len(), 1);
                assert_eq!(
                    app.active_document(),
                    Some(files::DocumentTab::Usage(first))
                );
                app.select_document(files::DocumentTab::Chat(2), window, cx);
                assert!(app.usage_ui.active.is_none());
                app.open_usage(window, cx);
                let second = app.usage_ui.active.unwrap();
                assert_ne!(first, second);
                app.select_document(files::DocumentTab::Usage(first), window, cx);
                assert_eq!(app.selected_workspace, Some(0));
                assert_eq!(panel.read(cx).month.read(cx).value().as_str(), "2026-01");
                assert_eq!(panel.read(cx).budget.read(cx).value().as_str(), "0.012345");
                app.select_document(files::DocumentTab::Settings, window, cx);
                assert_eq!(app.active_document(), Some(files::DocumentTab::Settings));
                app.close_documents(&[files::DocumentTab::Settings], window, cx);
                assert_eq!(
                    app.active_document(),
                    Some(files::DocumentTab::Usage(first))
                );
                app.close_documents(&[files::DocumentTab::Usage(first)], window, cx);
                assert!(!app.usage_ui.panels.contains_key(&first));
                assert_eq!(app.active_document(), Some(files::DocumentTab::Chat(1)));
                app.select_document(files::DocumentTab::Usage(second), window, cx);
                let root = app.usage_ui.roots[&second].clone();
                app.forget_workspace_files(&root);
                assert!(app.usage_ui.active.is_none());
                assert!(app.usage_ui.panels.is_empty());
            });
        });
    }

    #[gpui_kit::test]
    fn changing_usage_month_clears_the_selected_service(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (host, cx, root) = super::super::queue::tests::fixture(cx, "usage-filter");
        cx.update(|window, cx| {
            host.update(cx, |app, cx| {
                app.startup = None;
                app.store.usage_ledger.record(Observation {
                    key: "usage-filter-turn".into(),
                    session: 1,
                    workspace: root.clone(),
                    provider: Provider::Claude,
                    service: "local".into(),
                    service_name: None,
                    model: None,
                    finished_at: unix_now(),
                    input: Some(1200),
                    cached: Some(800),
                    output: Some(400),
                    elapsed_ms: 1000,
                    incomplete: false,
                    api_first_response_ms: None,
                    api_request_outcome: None,
                });
                app.open_usage(window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let center = cx.debug_bounds("usage-group-provider").unwrap().center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let center = cx.debug_bounds("usage-provider-0").unwrap().center();
        cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        let panel = cx.update(|_, cx| {
            let app = host.read(cx);
            app.usage_ui.panels[&app.usage_ui.active.unwrap()].clone()
        });
        cx.update(|window, cx| {
            assert!(panel.read(cx).selected_service.is_some());
            panel.update(cx, |panel, cx| {
                panel.month.update(cx, |input, cx| {
                    input.set_value("2020-01", window, cx);
                    cx.emit(super::InputEvent::Change);
                })
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| assert!(panel.read(cx).selected_service.is_none()));
    }
}
