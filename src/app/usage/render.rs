use super::insights::{Day, Insights, high_average};
use super::*;
use chrono::{Datelike, NaiveDate};
use gpui_kit::component::{chart::BarChart, plot::shape::BarAlignment};

fn message(key: &str, args: &[(&str, String)]) -> String {
    conduit::i18n::format(key, args)
}

fn section(title: &'static str, hint: impl Into<SharedString>) -> Div {
    heading(title, hint)
        .font_semibold()
        .text_size(ui_text_size(16.))
}

fn information(title: &'static str, note: impl Into<SharedString>) -> Popover {
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

fn heading(title: &'static str, hint: impl Into<SharedString>) -> Div {
    h_flex()
        .min_w_0()
        .items_center()
        .gap_1()
        .child(div().min_w_0().child(label(title)))
        .child(information(title, hint))
}

fn metric(title: &'static str, value: String, detail: impl IntoElement, note: String) -> Div {
    card()
        .flex_1()
        .min_w(px(160.))
        .p_3()
        .gap_2()
        .child(
            heading(title, note)
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().muted)),
        )
        .child(
            div()
                .text_size(ui_text_size(26.))
                .font_semibold()
                .child(value),
        )
        .child(detail)
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

fn legend(title: &'static str, color: u32) -> Div {
    h_flex()
        .gap_2()
        .items_center()
        .child(div().size(px(8.)).rounded_sm().bg(rgb(color)))
        .child(caption(label(title)))
}

fn daily_chart(insights: &Insights) -> impl IntoElement + use<> {
    div()
        .id("usage-daily-chart-region")
        .h(px(210.))
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
                        palette().activity_running
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
    fn header(&self, workspace: String, scope: String, cx: &Context<Self>) -> Div {
        h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(24.))
                            .font_semibold()
                            .child(label("usage.title")),
                    )
                    .child(caption(format!("{workspace} · {scope}"))),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("usage-previous-month")
                            .small()
                            .ghost()
                            .label("‹")
                            .accessibility_label(label("usage.previous_month"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.shift_month(-1, window, cx)),
                            ),
                    )
                    .child(div().w(px(145.)).child(Input::new(&self.month)))
                    .child(
                        Button::new("usage-next-month")
                            .small()
                            .ghost()
                            .label("›")
                            .accessibility_label(label("usage.next_month"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.shift_month(1, window, cx)),
                            ),
                    ),
            )
    }

    fn summary(
        &self,
        report: &Report,
        insights: &Insights,
        truncated: bool,
        cx: &Context<Self>,
    ) -> Div {
        let totals = &report.filtered;
        let completeness = share(totals.complete_turns as u64, totals.turns as u64);
        let comparison = insights
            .change()
            .map_or_else(|| "—".into(), |change| format!("{change:+.1}%"));
        let comparison_hint = if insights.comparable {
            message(
                "usage.comparison_range",
                &[("days", insights.comparison_days.to_string())],
            )
        } else {
            label("usage.no_comparison").into()
        };
        let mut total_note = label("usage.coverage").to_owned();
        if let Some(first) = insights.first_seen {
            total_note.push_str(&format!(
                "\n\n{}",
                message("usage.recording_since", &[("date", first.to_string())]),
            ));
        }
        let total_detail = if let Some(average) = totals.average() {
            total_note.push_str(&format!(
                "\n\n{}",
                message(
                    "usage.complete_average_note",
                    &[("count", totals.complete_turns.to_string())],
                ),
            ));
            message("usage.average_short", &[("tokens", format_tokens(average))])
        } else {
            label("usage.total_hint").into()
        };
        let coverage = message(
            "usage.coverage_count",
            &[
                ("complete", totals.complete_turns.to_string()),
                ("turns", totals.turns.to_string()),
            ],
        );
        let mut completeness_note = coverage.clone();
        let mut completeness_detail = h_flex().flex_wrap().gap_2();
        if totals.incomplete > 0 {
            completeness_note.push_str(&format!(
                "\n\n{}",
                message(
                    "usage.partial_note",
                    &[("count", totals.incomplete.to_string())]
                ),
            ));
            completeness_detail = completeness_detail.child(
                caption(message(
                    "usage.incomplete_short",
                    &[("count", totals.incomplete.to_string())],
                ))
                .text_color(rgb(palette().warning_text)),
            );
        } else {
            completeness_detail = completeness_detail.child(caption(coverage));
        }
        if truncated {
            completeness_note.push_str(&format!("\n\n{}", label("usage.truncated")));
            completeness_detail = completeness_detail.child(
                caption(label("usage.truncated_short")).text_color(rgb(palette().warning_text)),
            );
        }
        let mut result = v_flex().gap_3().child(
            h_flex()
                .flex_wrap()
                .gap_3()
                .child(metric(
                    "usage.total_tokens",
                    format_tokens(totals.tokens()),
                    caption(total_detail),
                    total_note,
                ))
                .child(metric(
                    "usage.period_change",
                    comparison,
                    caption(comparison_hint),
                    if insights.comparable {
                        message(
                            "usage.comparison_range",
                            &[("days", insights.comparison_days.to_string())],
                        )
                    } else {
                        label("usage.comparison_unavailable").into()
                    },
                ))
                .child(metric(
                    "usage.data_completeness",
                    completeness,
                    completeness_detail,
                    completeness_note,
                )),
        );
        if self.selected_service.is_some() {
            result = result.child(
                Button::new("usage-clear-scope")
                    .small()
                    .ghost()
                    .label(label("usage.show_all"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected_service = None;
                        this.model_details.clear();
                        cx.notify();
                    })),
            );
        }
        result
    }

    fn trend(&self, insights: &Insights) -> Div {
        let mut result = card()
            .flex_1()
            .min_w_0()
            .child(section("usage.trend", label("usage.daily_chart_note")))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_4()
                    .child(legend("usage.known_daily", palette().activity_running))
                    .child(legend("usage.partial_daily", palette().warning_text)),
            );
        if let Some(peak) = insights.peak() {
            result = result.child(caption(message(
                "usage.peak_day",
                &[
                    ("date", peak.date.to_string()),
                    ("tokens", format_tokens(peak.tokens)),
                ],
            )));
        }
        if insights.elapsed_days == 0 || insights.days.iter().all(|day| day.turns == 0) {
            return result.child(
                div().py_6().child(
                    v_flex()
                        .gap_1()
                        .child(div().font_semibold().child(label("usage.empty")))
                        .child(caption(label("usage.empty_hint"))),
                ),
            );
        }
        result.child(daily_chart(insights))
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
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(section("usage.reminder_status", reminder_note))
                    .child(
                        Button::new("usage-edit-reminder")
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

    fn breakdown(&self, report: &Report, cx: &Context<Self>) -> Div {
        let mut result = card().child(
            h_flex()
                .flex_wrap()
                .justify_between()
                .gap_2()
                .child(section(
                    "usage.breakdown_title",
                    label("usage.breakdown_note"),
                ))
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("usage-group-model")
                                .small()
                                .selected(!self.by_service)
                                .label(label("usage.by_model"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.by_service = false;
                                    this.model_details.clear();
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
                                    this.model_details.clear();
                                    cx.notify();
                                })),
                        ),
                ),
        );
        // Keys stay distinct even if two providers share a display name.
        let data: Vec<(String, String, u64)> = if self.by_service {
            report
                .services
                .iter()
                .take(5)
                .map(|service| {
                    (
                        service.key.clone(),
                        service_title(service),
                        service.totals.tokens(),
                    )
                })
                .collect()
        } else {
            report
                .models
                .iter()
                .take(5)
                .enumerate()
                .map(|(index, (model, totals))| {
                    (
                        index.to_string(),
                        model
                            .clone()
                            .unwrap_or_else(|| label("usage.unknown_model").into()),
                        totals.tokens(),
                    )
                })
                .collect()
        };
        if data.is_empty() {
            return result.child(caption(label("usage.empty")));
        }
        let data: Vec<_> = data
            .into_iter()
            .enumerate()
            .map(|(index, (_, title, tokens))| {
                let short: String = title.chars().take(18).collect();
                let short = if title.chars().count() > 18 {
                    format!("{short}…")
                } else {
                    short
                };
                (format!("{}. {short}", index + 1), title, tokens)
            })
            .collect();
        let height = (data.len() as f32 * 30. + 30.).max(75.);
        result = result
            .child(
                div()
                    .id("usage-ranking-chart-region")
                    .h(px(height))
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
                            .fill(|_, _, _, _| rgb(palette().activity_running))
                            .grid(false)
                            .corner_radii(px(2.))
                            .appear(false)
                            .tooltip_title(|row| row.1.clone().into())
                            .tooltip_value(|row, _| {
                                format!("{} Token", format_tokens(row.2)).into()
                            }),
                    ),
            )
            .child(
                Button::new("usage-toggle-details")
                    .small()
                    .ghost()
                    .label(label(if self.details_open {
                        "usage.collapse_details"
                    } else {
                        "usage.expand_details"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.details_open = !this.details_open;
                        cx.notify();
                    })),
            );
        // Provider filters remain reachable without opening the detailed table.
        if self.by_service || self.details_open {
            result = result.child(self.breakdown_rows(report, cx));
        }
        result
    }

    fn breakdown_rows(&self, report: &Report, cx: &Context<Self>) -> Div {
        let mut result = v_flex().gap_2();
        if self.by_service {
            for (index, service) in report.services.iter().enumerate() {
                let key = service.key.clone();
                let selected = self.selected_service.as_deref() == Some(key.as_str());
                let mut row = self.detail_row(
                    format!("provider:{key}"),
                    service_title(service),
                    &service.totals,
                    report.totals.tokens(),
                    cx,
                );
                row = row.child(
                    Button::new(("usage-provider-filter", index))
                        .debug_selector(move || format!("usage-provider-{index}"))
                        .small()
                        .ghost()
                        .selected(selected)
                        .label(label(if selected {
                            "usage.show_all"
                        } else {
                            "usage.inspect_provider"
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_service = if selected { None } else { Some(key.clone()) };
                            this.by_service = false;
                            this.shown = 20;
                            this.model_details.clear();
                            cx.notify();
                        })),
                );
                result = result.child(row);
            }
        } else {
            for (model, totals) in report.models.iter().take(self.shown) {
                result = result.child(
                    self.detail_row(
                        format!("model:{}", model.as_deref().unwrap_or("")),
                        model
                            .clone()
                            .unwrap_or_else(|| label("usage.unknown_model").into()),
                        totals,
                        report.filtered.tokens(),
                        cx,
                    ),
                );
            }
            if report.models.len() > self.shown {
                result = result.child(
                    Button::new("usage-more")
                        .small()
                        .ghost()
                        .label(label("usage.more"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shown += 20;
                            cx.notify();
                        })),
                );
            }
        }
        result
    }

    fn detail_row(
        &self,
        key: String,
        title: String,
        totals: &Totals,
        scope_tokens: u64,
        cx: &Context<Self>,
    ) -> Div {
        let expanded = self.model_details.contains(&key);
        let mut row = v_flex()
            .gap_2()
            .py_3()
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .font_medium()
                            .child(title),
                    )
                    .child(caption(format!(
                        "{} Token · {}",
                        format_tokens(totals.tokens()),
                        share(totals.tokens(), scope_tokens)
                    ))),
            );
        row = row.child(
            h_flex()
                .flex_wrap()
                .gap_3()
                .child(caption(message(
                    "usage.detail_record_count",
                    &[
                        ("turns", totals.turns.to_string()),
                        ("incomplete", totals.incomplete.to_string()),
                    ],
                )))
                .child(caption(message(
                    "usage.detail_average",
                    &[(
                        "tokens",
                        totals.average().map_or_else(|| "—".into(), format_tokens),
                    )],
                ))),
        );
        if self.details_open {
            row = row.child(
                Button::new(SharedString::from(format!("usage-composition-{key}")))
                    .small()
                    .ghost()
                    .label(label(if expanded {
                        "usage.hide_composition"
                    } else {
                        "usage.show_composition"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.model_details.remove(&key) {
                            this.model_details.insert(key.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        row.when(expanded, |row| row.child(composition(totals)))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let mut content = v_flex().w_full().max_w(px(1200.)).gap_4();
        if let Some(first) = first {
            let ledger = &app.store.usage_ledger;
            let today = Local::now().date_naive();
            let all_report = Report::build(
                &ledger.observations,
                &self.workspace,
                &month,
                &app.store.providers,
                None,
            );
            let filtered_report = self.selected_service.as_ref().map(|selected| {
                Report::build(
                    &ledger.observations,
                    &self.workspace,
                    &month,
                    &app.store.providers,
                    Some(selected),
                )
            });
            let report = filtered_report.as_ref().unwrap_or(&all_report);
            let scope = report
                .services
                .iter()
                .find(|service| Some(&service.key) == self.selected_service.as_ref())
                .map(service_title)
                .unwrap_or_else(|| label("usage.all_providers").into());
            let all_insights =
                Insights::build(ledger, &self.workspace, first, today, None, &all_report);
            let filtered_insights = self.selected_service.as_deref().map(|selected| {
                Insights::build(
                    ledger,
                    &self.workspace,
                    first,
                    today,
                    Some(selected),
                    report,
                )
            });
            let insights = filtered_insights.as_ref().unwrap_or(&all_insights);
            content = content.child(self.header(workspace, scope, cx));
            if let Some(error) = app.persistence.error.as_ref() {
                content = content.child(
                    div()
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                );
            }
            content = content.child(self.summary(report, insights, ledger.truncated, cx));
            let comparison = self.comparison(insights);
            let trend = self.trend(insights);
            let width = f32::from(window.viewport_size().width)
                - app.sidebar_width
                - 44.
                - if window.viewport_size().width >= px(1100.) && app.right_view_visible() {
                    app.plugins.dock.width
                } else {
                    0.
                };
            content = content
                .child(if width >= 780. {
                    h_flex()
                        .gap_4()
                        .items_start()
                        .child(trend.flex_1())
                        .child(comparison.w(px(310.)).flex_none())
                        .into_any_element()
                } else {
                    v_flex()
                        .gap_4()
                        .child(trend)
                        .child(comparison)
                        .into_any_element()
                })
                .child(self.breakdown(report, cx))
                .child(self.sessions(report, app))
                .child(self.reminder(&all_report, &all_insights, limit, writable, cx));
        } else {
            content = content
                .child(self.header(workspace, label("usage.all_providers").into(), cx))
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
            .p_5()
            .items_center()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .text_size(ui_text_size(13.))
            .child(content)
            .into_any_element()
    }
}
