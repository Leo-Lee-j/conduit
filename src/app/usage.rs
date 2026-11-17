//! Known usage and advisory token budgets; no inferred monetary billing.
use super::*;
use crate::usage_ledger::{Observation, Totals};
use gpui_kit::component::{chart::BarChart, progress::Progress};
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
    pub(super) fn record_turn_usage(
        &mut self,
        id: u64,
        run: &ActiveRun,
        incomplete: bool,
        cx: &mut Context<Self>,
    ) {
        if matches!(run.intent, crate::commands::RequestIntent::Discover) {
            return;
        }
        let Some(session) = self.session(id) else {
            return;
        };
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
        self.store.usage_ledger.record(Observation {
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
        });
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
            let budget = cx.new(|cx| InputState::new(window, cx).default_value(budget.to_string()));
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
    model_details: HashSet<Option<String>>,
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
        let Ok(limit) = self.budget.read(cx).value().parse::<u64>() else {
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
fn metric(title: &'static str, value: String, note: impl Into<SharedString>) -> Div {
    card()
        .gap_2()
        .child(caption(label(title)))
        .child(
            div()
                .text_size(ui_text_size(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(value),
        )
        .child(caption(note))
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
fn breakdown_note(total: &Totals) -> String {
    conduit::i18n::format(
        "usage.model_note",
        &[
            ("input", format_tokens(total.input)),
            ("cached", format_tokens(total.cached)),
            ("output", format_tokens(total.output)),
            ("turns", total.turns.to_string()),
            (
                "average",
                format_tokens(total.tokens() / total.turns.max(1) as u64),
            ),
        ],
    )
}

impl Render for UsagePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let width = f32::from(window.viewport_size().width)
            - app.sidebar_width
            - 6.
            - if app.plugins.right_visible(false) {
                app.plugins.dock.width + 6.
            } else {
                0.
            };
        let columns = if width >= 880. {
            3
        } else if width >= 420. {
            2
        } else {
            1
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
        let changed = self.budget.read(cx).value().parse::<u64>().ok() != Some(limit);
        let mut content = v_flex().w_full().max_w(px(1120.)).gap_5().child(
            h_flex()
                .flex_wrap()
                .gap_4()
                .justify_between()
                .items_end()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(180.))
                        .gap_1()
                        .child(
                            div()
                                .text_size(ui_text_size(24.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label("usage.title")),
                        )
                        .child(caption(self.workspace.display().to_string()).truncate()),
                )
                .child(
                    v_flex()
                        .w(px(170.))
                        .gap_1()
                        .child(caption(label("usage.month")))
                        .child(Input::new(&self.month)),
                ),
        );
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
            let remaining = limit.saturating_sub(totals.tokens());
            let exceeded = totals.tokens().saturating_sub(limit);
            let leading = report.services.first();
            content = content.child(
                div()
                    .grid()
                    .grid_cols(columns)
                    .gap_3()
                    .child(metric(
                        "usage.month_usage",
                        format!("{} Token", format_tokens(totals.tokens())),
                        conduit::i18n::format(
                            "usage.observed_turns",
                            &[("count", totals.turns.to_string())],
                        ),
                    ))
                    .child(metric(
                        if limit > 0 && totals.tokens() > limit {
                            "usage.over_budget"
                        } else {
                            "usage.remaining_budget"
                        },
                        if limit == 0 {
                            label("usage.no_budget").into()
                        } else {
                            format!(
                                "{} Token",
                                format_tokens(if totals.tokens() > limit {
                                    exceeded
                                } else {
                                    remaining
                                })
                            )
                        },
                        if limit == 0 {
                            label("usage.set_budget_hint").into()
                        } else {
                            conduit::i18n::format(
                                "usage.limit_note",
                                &[("limit", format_tokens(limit))],
                            )
                        },
                    ))
                    .child(metric(
                        "usage.leading_provider",
                        leading.map(service_title).unwrap_or_else(|| "—".into()),
                        leading
                            .map(|s| {
                                conduit::i18n::format(
                                    "usage.share_note",
                                    &[("share", share(s.totals.tokens(), totals.tokens()))],
                                )
                            })
                            .unwrap_or_else(|| label("usage.empty").into()),
                    )),
            );
            if totals.incomplete > 0 {
                content = content.child(
                    caption(conduit::i18n::format(
                        "usage.partial_note",
                        &[("count", totals.incomplete.to_string())],
                    ))
                    .text_color(rgb(palette().warning_text)),
                );
            }
            let mut providers = card()
                .child(
                    div()
                        .text_size(ui_text_size(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label("usage.providers_title")),
                )
                .child(caption(label("usage.providers_hint")));
            if report.services.is_empty() {
                providers = providers
                    .child(div().py_5().child(label("usage.empty")))
                    .child(caption(label("usage.empty_hint")));
            }
            for (index, service) in report.services.iter().enumerate() {
                let key = service.key.clone();
                let selected = self.selected_service.as_ref() == Some(&key);
                let title = service_title(service);
                providers = providers.child(
                    v_flex()
                        .gap_2()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_3()
                                .justify_between()
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            div().font_weight(FontWeight::SEMIBOLD).child(format!(
                                                "{}  {}",
                                                index + 1,
                                                title
                                            )),
                                        )
                                        .child(caption(format!(
                                            "{} · {}",
                                            service.engine.label(),
                                            conduit::i18n::format(
                                                "usage.observed_turns",
                                                &[("count", service.totals.turns.to_string())]
                                            )
                                        ))),
                                )
                                .child(
                                    h_flex()
                                        .gap_3()
                                        .flex_wrap()
                                        .child(div().font_weight(FontWeight::MEDIUM).child(
                                            format!(
                                                "{} Token · {}",
                                                format_tokens(service.totals.tokens()),
                                                share(service.totals.tokens(), totals.tokens())
                                            ),
                                        ))
                                        .child(
                                            Button::new(("usage-provider", index))
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
                                                    this.selected_service = if selected {
                                                        None
                                                    } else {
                                                        Some(key.clone())
                                                    };
                                                    this.shown = 20;
                                                    this.model_details.clear();
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                        )
                        .child(
                            Progress::new(("usage-provider-share", index))
                                .small()
                                .value(if totals.tokens() == 0 {
                                    0.
                                } else {
                                    (service.totals.tokens() as f64 * 100. / totals.tokens() as f64)
                                        as f32
                                })
                                .accessibility_label(service_title(service)),
                        )
                        .when(service.name.is_none(), |row| {
                            row.child(caption(label("usage.unattributed_hint")))
                        }),
                );
            }
            content = content.child(providers);
            let scope = report
                .services
                .iter()
                .find(|s| Some(&s.key) == self.selected_service.as_ref())
                .map(service_title)
                .unwrap_or_else(|| label("usage.all_providers").into());
            let mut models = card()
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .justify_between()
                        .child(
                            div()
                                .text_size(ui_text_size(16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label("usage.models_title")),
                        )
                        .child(
                            caption(scope.clone()).debug_selector(|| "usage-filter-scope".into()),
                        )
                        .when(self.selected_service.is_some(), |row| {
                            row.child(
                                Button::new("usage-clear-filter")
                                    .small()
                                    .ghost()
                                    .label(label("usage.show_all"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.selected_service = None;
                                        this.shown = 20;
                                        this.model_details.clear();
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .child(caption(label("usage.models_hint")));
            if report.models.is_empty() {
                models = models.child(caption(label("usage.empty")));
            }
            for (index, (model, total)) in report.models.iter().take(self.shown).enumerate() {
                let expanded = self.model_details.contains(model);
                let model_key = model.clone();
                models = models.child(
                    v_flex()
                        .gap_3()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            h_flex()
                                .flex_wrap()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div().font_weight(FontWeight::SEMIBOLD).child(
                                        model
                                            .clone()
                                            .unwrap_or_else(|| label("usage.unknown_model").into()),
                                    ),
                                )
                                .child(caption(conduit::i18n::format(
                                    "usage.scope_share",
                                    &[("share", share(total.tokens(), report.filtered.tokens()))],
                                ))),
                        )
                        .child(
                            div()
                                .grid()
                                .grid_cols(if width >= 480. { 3 } else { 1 })
                                .gap_3()
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(caption(label("usage.total_tokens")))
                                        .child(format!("{} Token", format_tokens(total.tokens()))),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(caption(label("usage.per_turn")))
                                        .child(format!(
                                            "{} Token",
                                            format_tokens(
                                                total.tokens() / total.turns.max(1) as u64
                                            )
                                        )),
                                )
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(caption(label("usage.turns")))
                                        .child(total.turns.to_string()),
                                ),
                        )
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
                                    if !this.model_details.remove(&model_key) {
                                        this.model_details.insert(model_key.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                        .when(expanded, |row| row.child(caption(breakdown_note(total))))
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
                models = models.child(
                    Button::new("usage-more")
                        .small()
                        .label(label("usage.more"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shown += 20;
                            cx.notify();
                        })),
                );
            }
            content = content.child(models);
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
            let mut trend = card()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(label("usage.trend")),
                )
                .child(caption(format!("{} · Token", scope)));
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
                    div().h(px(180.)).w_full().child(
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
                            .tooltip_value(|d, _| format!("{} Token", d.1).into()),
                    ),
                );
            } else {
                trend = trend.child(caption(label("usage.empty")));
            }
            let ratio = if limit == 0 {
                0.
            } else {
                totals.tokens() as f64 / limit as f64 * 100.
            };
            let mut budget = card()
                .child(
                    h_flex()
                        .justify_between()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label("usage.budget")),
                        )
                        .child(caption(if limit == 0 {
                            label("usage.disabled").to_owned()
                        } else {
                            format!(
                                "{ratio:.1}% · {} / {} Token",
                                format_tokens(totals.tokens()),
                                format_tokens(limit)
                            )
                        })),
                )
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
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_end()
                        .gap_3()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(160.))
                                .gap_1()
                                .child(caption(label("usage.budget_amount")))
                                .child(Input::new(&self.budget).disabled(!writable)),
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
                .child(caption(label("usage.budget_hint")));
            if let Some(error) = &self.error {
                budget =
                    budget.child(caption(error.clone()).text_color(rgb(palette().danger_text)));
            }
            if ratio >= 100. {
                budget = budget.child(
                    caption(label("usage.budget_reached")).text_color(rgb(palette().warning_text)),
                );
            }
            content = content.child(
                div()
                    .grid()
                    .grid_cols(if width >= 880. { 2 } else { 1 })
                    .gap_4()
                    .child(trend)
                    .child(budget),
            );
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
                        .update(cx, |input, cx| input.set_value("12345", window, cx));
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
                assert_eq!(panel.read(cx).budget.read(cx).value().as_str(), "12345");
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
                });
                app.open_usage(window, cx);
            })
        });
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
