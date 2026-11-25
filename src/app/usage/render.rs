use super::insights::{Day, Insights, high_average};
use super::*;
use chrono::{Datelike, NaiveDate};
use gpui_kit::component::{chart::BarChart, plot::shape::BarAlignment};

pub(super) fn message(key: &str, args: &[(&str, String)]) -> String {
    conduit::i18n::format(key, args)
}

fn section(title: &'static str, hint: impl Into<SharedString>) -> Div {
    heading(title, hint)
        .font_semibold()
        .text_size(ui_text_size(16.))
}

pub(super) fn information(title: &'static str, note: impl Into<SharedString>) -> Popover {
    let note = note.into();
    Popover::new(SharedString::from(format!("usage-info-{title}")))
        .trigger(
            Button::new(SharedString::from(format!("usage-info-trigger-{title}")))
                .debug_selector(move || format!("usage-info-{title}"))
                .small()
                .compact()
                .ghost()
                .size(px(22.))
                .focus_ring(true)
                .icon(FullIconName::Info)
                .accessibility_label(message(
                    "usage.info_label",
                    &[("title", label(title).into())],
                ))
                .tooltip(note.clone()),
        )
        .content(move |_, window, _| {
            v_flex()
                .w(px(
                    (f32::from(window.viewport_size().width) - 48.).clamp(180., 320.)
                ))
                .gap_2()
                .text_size(ui_text_size(12.))
                .text_color(rgb(palette().text))
                .font_normal()
                .child(div().font_semibold().child(label(title)))
                .child(div().child(note.clone()))
        })
}

pub(super) fn heading(title: &'static str, hint: impl Into<SharedString>) -> Div {
    h_flex()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(div().min_w_0().child(label(title)))
        .child(information(title, hint))
}

fn reminder_state(
    tokens: u64,
    limit: u64,
    forecast: Option<&super::insights::Forecast>,
) -> (&'static str, u32) {
    if limit == 0 {
        ("usage.no_budget", palette().muted)
    } else if tokens >= limit {
        ("usage.budget_reached", palette().warning_text)
    } else if tokens >= limit - limit / 5 {
        ("usage.budget_near", palette().warning_text)
    } else if forecast.is_some_and(|forecast| forecast.tokens >= limit) {
        ("usage.projected_threshold", palette().warning_text)
    } else {
        ("usage.below_threshold", palette().text)
    }
}

pub(super) fn daily_chart(insights: &Insights) -> impl IntoElement + use<> {
    div()
        .id("usage-daily-chart-region")
        .debug_selector(|| "usage-daily-chart-region".into())
        .h(px(170.))
        .w_full()
        .role(Role::Group)
        .aria_label(label("usage.daily_chart_note"))
        .child(
            BarChart::new(insights.days.clone())
                .id("usage-daily-chart")
                .band(|day: &Day| SharedString::from(day.date.day().to_string()))
                .value(|day| day.tokens as f64)
                .fill(|day, _, _, _| {
                    rgb(if day.incomplete > 0 {
                        palette().warning_text
                    } else {
                        super::dashboard::chart_color()
                    })
                })
                .name(label("usage.total_tokens"))
                .value_axis(true)
                .value_tick_count(3)
                .value_tick_format(|value| format_tokens(value as u64))
                .band_tick_count(7)
                .grid_dashed(false)
                .corner_radii(px(2.))
                .appear(false)
                .tooltip_title(|day| day.date.format("%Y-%m-%d").to_string().into())
                .tooltip_value(|day, _| {
                    if !day.observed {
                        return label("usage.before_records").into();
                    }
                    message(
                        "usage.day_detail",
                        &[
                            ("tokens", format_tokens(day.tokens)),
                            ("turns", day.turns.to_string()),
                            ("incomplete", day.incomplete.to_string()),
                        ],
                    )
                    .into()
                }),
        )
}

impl UsagePanel {
    fn concentration(&self, report: &Report, cx: &Context<Self>) -> Div {
        let mut result = card()
            .debug_selector(|| "usage-concentration-card".into())
            .h(px(270.))
            .flex_1()
            .flex_basis(px(340.))
            .min_w_0()
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(section(
                        "usage.breakdown_title",
                        label("usage.concentration_note"),
                    ))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("usage-concentration-model")
                                    .debug_selector(|| "usage-concentration-model".into())
                                    .small()
                                    .ghost()
                                    .selected(!self.concentration_by_service)
                                    .label(label("usage.by_model"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.concentration_by_service = false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("usage-concentration-provider")
                                    .debug_selector(|| "usage-concentration-provider".into())
                                    .small()
                                    .ghost()
                                    .selected(self.concentration_by_service)
                                    .label(label("usage.by_provider"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.concentration_by_service = true;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        let data: Vec<(String, u64)> = if self.concentration_by_service {
            report
                .services
                .iter()
                .filter(|service| {
                    self.selected_service
                        .as_ref()
                        .is_none_or(|selected| selected == &service.key)
                })
                .take(5)
                .map(|service| (service_title(service), service.totals.tokens()))
                .collect()
        } else {
            report
                .models
                .iter()
                .take(5)
                .map(|(model, totals)| {
                    (
                        model
                            .clone()
                            .unwrap_or_else(|| label("usage.unknown_model").into()),
                        totals.tokens(),
                    )
                })
                .collect()
        };
        if data.is_empty() {
            return result.child(
                v_flex()
                    .h(px(170.))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(caption(label("usage.no_matching_records")))
                    .child(caption(label("usage.empty_hint"))),
            );
        }
        // Rank prefixes keep categories distinct even when display names match.
        let data: Vec<_> = data
            .into_iter()
            .enumerate()
            .map(|(index, (title, tokens))| {
                let short: String = title.chars().take(18).collect();
                let suffix = if title.chars().count() > 18 {
                    "…"
                } else {
                    ""
                };
                (format!("{}. {short}{suffix}", index + 1), title, tokens)
            })
            .collect();
        let scope_tokens = report.filtered.tokens();
        result = result.child(
            div()
                .id("usage-ranking-chart-region")
                .debug_selector(|| "usage-ranking-chart-region".into())
                .h(px(180.))
                .w_full()
                .role(Role::Group)
                .aria_label(label("usage.ranking_chart_note"))
                .child(
                    BarChart::new(data)
                        .id("usage-ranking-chart")
                        .band(|row: &(String, String, u64)| SharedString::from(row.0.clone()))
                        .value(|row| row.2 as f64)
                        .alignment(BarAlignment::Left)
                        .label(|row| format_tokens(row.2))
                        .fill(|_, _, _, _| rgb(super::dashboard::chart_color()))
                        .grid(false)
                        .corner_radii(px(2.))
                        .appear(false)
                        .tooltip_title(|row| row.1.clone().into())
                        .tooltip_value(move |row, _| {
                            format!(
                                "{} Token · {}",
                                format_tokens(row.2),
                                share(row.2, scope_tokens)
                            )
                            .into()
                        }),
                ),
        );
        result
    }

    fn comparison(&self, insights: &Insights) -> Div {
        let range = message(
            "usage.comparison_range",
            &[("days", insights.comparison_days.to_string())],
        );
        let mut result = card().flex_1().min_w_0().child(section(
            "usage.period_comparison",
            if insights.comparable {
                range.clone()
            } else {
                format!("{range}\n\n{}", label("usage.comparison_unavailable"))
            },
        ));
        if !insights.comparable {
            return result.child(div().py_5().child(caption(label("usage.no_comparison"))));
        }
        result = result.child(caption(range));
        let data = vec![
            (
                label("usage.previous_period").to_owned(),
                insights.previous.tokens(),
                false,
            ),
            (
                label("usage.selected_period").to_owned(),
                insights.comparable_current.tokens(),
                true,
            ),
        ];
        result = result.child(
            div()
                .id("usage-comparison-chart-region")
                .h(px(150.))
                .w_full()
                .role(Role::Group)
                .aria_label(label("usage.period_comparison"))
                .child(
                    BarChart::new(data)
                        .id("usage-comparison-chart")
                        .band(|row: &(String, u64, bool)| SharedString::from(row.0.clone()))
                        .value(|row| row.1 as f64)
                        .alignment(BarAlignment::Left)
                        .fill(|row, _, _, _| {
                            rgb(if row.2 {
                                palette().activity_running
                            } else {
                                palette().muted
                            })
                        })
                        .label(|row| format_tokens(row.1))
                        .grid(false)
                        .corner_radii(px(2.))
                        .appear(false)
                        .tooltip_value(|row, _| format!("{} Token", format_tokens(row.1)).into()),
                ),
        );
        result.child(caption(message(
            "usage.comparison_values",
            &[
                (
                    "current",
                    format_tokens(insights.comparable_current.tokens()),
                ),
                ("previous", format_tokens(insights.previous.tokens())),
            ],
        )))
    }

    fn reminder(
        &self,
        report: &Report,
        insights: &Insights,
        limit: u64,
        writable: bool,
        cx: &Context<Self>,
    ) -> Div {
        let tokens = report.totals.tokens();
        let ratio = if limit > 0 {
            tokens as f64 * 100. / limit as f64
        } else {
            0.
        };
        let forecast = insights.forecast.as_ref();
        let (status, color) = reminder_state(tokens, limit, forecast);
        let mut reminder_note = format!(
            "{}\n\n{}",
            label("usage.workspace_reminder_scope"),
            label("usage.budget_hint"),
        );
        if limit == 0 {
            reminder_note.push_str(&format!("\n\n{}", label("usage.reminder_optional")));
        }
        if forecast.is_none() {
            reminder_note.push_str(&format!(
                "\n\n{}\n\n{}",
                label("usage.forecast_unavailable"),
                label("usage.forecast_note"),
            ));
        }
        let mut result = card()
            .debug_selector(|| "usage-budget-card".into())
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(section("usage.budget", reminder_note))
                    .child(
                        Button::new("usage-edit-reminder")
                            .debug_selector(|| "usage-edit-reminder".into())
                            .small()
                            .ghost()
                            .label(label(if self.budget_open {
                                "usage.close_settings"
                            } else {
                                "usage.edit_reminder"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.budget_open = !this.budget_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "usage-budget-status".into())
                    .font_semibold()
                    .text_color(rgb(color))
                    .child(label(status)),
            );
        if limit > 0 {
            result = result
                .child(
                    h_flex()
                        .flex_wrap()
                        .justify_between()
                        .gap_2()
                        .child(caption(format!(
                            "{ratio:.1}% · {} / {} Token",
                            format_tokens(tokens),
                            format_tokens(limit)
                        )))
                        .child(caption(message(
                            if tokens >= limit {
                                "usage.exceeded_by"
                            } else {
                                "usage.remaining_tokens"
                            },
                            &[("tokens", format_tokens(tokens.abs_diff(limit)))],
                        ))),
                )
                .child(
                    Progress::new("usage-budget-progress")
                        .value(ratio.min(100.) as f32)
                        .color(rgb(color))
                        .accessibility_label(label("usage.reminder_status")),
                );
        }
        if let Some(forecast) = forecast {
            result = result.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        caption(message(
                            "usage.observed_forecast",
                            &[
                                ("tokens", format_tokens(forecast.tokens)),
                                ("average", format_tokens(forecast.daily_average)),
                            ],
                        ))
                        .min_w_0()
                        .flex_1(),
                    )
                    .child(information(
                        "usage.forecast_title",
                        label("usage.forecast_note"),
                    )),
            );
        }
        if !self.budget_open {
            return result;
        }
        let changed = parse_budget(&self.budget.read(cx).value()) != Some(limit);
        result = result.child(
            div()
                .debug_selector(|| "usage-budget-settings".into())
                .border_t_1()
                .border_color(rgb(palette().border))
                .pt_3()
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            heading("usage.budget_amount", label("usage.budget_hint"))
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted)),
                        )
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(150.))
                                        .child(Input::new(&self.budget).disabled(!writable)),
                                )
                                .child(caption("M Token"))
                                .child(
                                    Button::new("usage-save-budget")
                                        .debug_selector(|| "usage-save-budget".into())
                                        .small()
                                        .label(label(if changed {
                                            "usage.save_budget"
                                        } else {
                                            "usage.saved_budget"
                                        }))
                                        .disabled(!writable || !changed)
                                        .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                                ),
                        ),
                ),
        );
        if let Some(forecast) = forecast.filter(|forecast| forecast.tokens > 0) {
            let suggested = forecast.tokens;
            result = result.child(
                Button::new("usage-use-forecast")
                    .small()
                    .outline()
                    .disabled(!writable)
                    .label(label("usage.use_forecast"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_budget(suggested, window, cx);
                        this.save(cx);
                    })),
            );
        }
        if let Some(error) = &self.error {
            result = result.child(caption(error.clone()).text_color(rgb(palette().danger_text)));
        }
        result
    }

    fn sessions(&self, report: &Report, app: &ConduitApp) -> Div {
        let mut ordered: Vec<_> = report.sessions.iter().collect();
        ordered.sort_by_key(|(_, totals)| {
            std::cmp::Reverse((
                totals.failed_requests > 0,
                high_average(totals, &report.filtered),
                totals.tokens(),
            ))
        });
        let mut result = card().child(section(
            "usage.sessions_title",
            label("usage.sessions_evidence_note"),
        ));
        for (id, totals) in ordered.into_iter().take(5) {
            let session = app.session(*id);
            let title = session
                .map(|session| session.title.clone())
                .filter(|title| !title.trim().is_empty())
                .unwrap_or_else(|| label("usage.unavailable_session").into());
            let evidence = if totals.failed_requests > 0 {
                message(
                    "usage.failed_request_evidence",
                    &[
                        ("failed", totals.failed_requests.to_string()),
                        ("requests", totals.requests.to_string()),
                    ],
                )
            } else if high_average(totals, &report.filtered) {
                message(
                    "usage.average_evidence",
                    &[(
                        "multiple",
                        format!(
                            "{:.1}",
                            totals.average().unwrap() as f64
                                / report.filtered.average().unwrap() as f64
                        ),
                    )],
                )
            } else {
                message(
                    "usage.concentration_evidence",
                    &[("share", share(totals.tokens(), report.filtered.tokens()))],
                )
            };
            let id = *id;
            let host = self.host.clone();
            result = result.child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .child(div().truncate().font_medium().child(title))
                            .child(caption(evidence).text_color(rgb(
                                if totals.failed_requests > 0 {
                                    palette().warning_text
                                } else {
                                    palette().muted
                                },
                            )))
                            .child(caption(format!(
                                "{} Token · {}",
                                format_tokens(totals.tokens()),
                                message(
                                    "usage.detail_record_count",
                                    &[
                                        ("turns", totals.turns.to_string()),
                                        ("incomplete", totals.incomplete.to_string())
                                    ]
                                )
                            ))),
                    )
                    .child(
                        Button::new(("usage-open-session", id))
                            .debug_selector(move || format!("usage-open-session-{id}"))
                            .small()
                            .ghost()
                            .label(label("usage.open_session"))
                            .disabled(session.is_none_or(|session| session.archived))
                            .on_click(move |_, window, cx| {
                                let _ = host.update(cx, |app, cx| {
                                    app.select_document(files::DocumentTab::Chat(id), window, cx)
                                });
                            }),
                    ),
            );
        }
        if report.sessions.is_empty() {
            result = result.child(caption(label("usage.empty")));
        }
        result
    }
}

impl Render for UsagePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let month = self.month.read(cx).value().to_string();
        let first = (month.len() == 7)
            .then(|| NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").ok())
            .flatten();
        let workspace = app
            .store
            .workspaces
            .iter()
            .find(|workspace| workspace.path == self.workspace)
            .map(Workspace::name)
            .unwrap_or_else(|| self.workspace.display().to_string());
        let limit = app
            .store
            .usage_ledger
            .budgets
            .iter()
            .find(|budget| budget.workspace == self.workspace)
            .map_or(0, |budget| budget.monthly_tokens);
        let mut content = v_flex().w_full().max_w(px(1440.)).gap_3();
        if let Some(first) = first {
            let ledger = &app.store.usage_ledger;
            let today = Local::now().date_naive();
            let snapshot = self.snapshot(app, &month, first, today);
            let all_report = &snapshot.all_report;
            let all_insights = &snapshot.all_insights;
            let report = snapshot.report();
            let insights = snapshot.insights();
            content = content
                .child(self.dashboard_header(workspace, cx))
                .child(self.dashboard_filters(report, cx));
            if let Some(error) = app.persistence.error.as_ref() {
                content = content.child(
                    div()
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                );
            }
            content = content.child(self.dashboard_overview(report, ledger.truncated));
            content = content
                .child(
                    h_flex()
                        .w_full()
                        .items_start()
                        .flex_wrap()
                        .gap_3()
                        .child(self.dashboard_daily(report, insights, ledger.truncated, cx))
                        .child(self.concentration(report, cx)),
                )
                .child(self.reminder(all_report, all_insights, limit, writable, cx));
            if self.details_open {
                let change = insights.change().map_or_else(
                    || label("usage.no_comparison").into(),
                    |value| format!("{value:+.1}%"),
                );
                let mut notes = h_flex().flex_wrap().gap_3().child(caption(format!(
                    "{} · {change}",
                    label("usage.period_change")
                )));
                if let Some(peak) = insights.peak() {
                    notes = notes.child(caption(message(
                        "usage.peak_day",
                        &[
                            ("date", peak.date.to_string()),
                            ("tokens", format_tokens(peak.tokens)),
                        ],
                    )));
                }
                content = content
                    .child(notes)
                    .child(self.comparison(insights))
                    .child(self.sessions(report, app));
            }
            let page = table::Page::new(
                self.page,
                self.page_size,
                snapshot.row_count(self.table_tab),
            );
            let rows = self.table_rows(&snapshot, app, page.range);
            content = content.child(
                div()
                    .pt_2()
                    .child(self.dashboard_table(&snapshot, rows, cx)),
            );
        } else {
            content = content
                .child(self.dashboard_header(workspace, cx))
                .child(self.dashboard_filters(&Report::default(), cx))
                .child(
                    caption(label("usage.invalid_month")).text_color(rgb(palette().danger_text)),
                );
        }
        v_flex()
            .id("usage-ledger-panel")
            .debug_selector(|| "usage-ledger-panel".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .p_4()
            .items_center()
            .bg(rgb(palette().surface))
            .text_color(rgb(palette().text))
            .text_size(ui_text_size(13.))
            .child(content)
            .into_any_element()
    }
}
