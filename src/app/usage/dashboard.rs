//! Compact usage overview and ledger tables. All values retain ledger semantics.
use super::insights::Insights;
use super::render::{daily_chart, heading, information, message};
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TableTab {
    Records,
    Providers,
    Models,
}

pub(super) fn chart_color() -> u32 {
    if is_dark_mode() { 0x38b9cf } else { 0x0891b2 }
}

fn count(value: usize) -> String {
    let digits = value.to_string();
    let mut result = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

fn token_value(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), format_tokens)
}

pub(super) fn first_response_value(value: Option<u64>) -> String {
    value.map_or_else(
        || label("usage.first_response_unrecorded").into(),
        |ms| format!("{:.1}s", ms as f64 / 1000.),
    )
}

fn status_label(status: Option<ApiRequestOutcome>) -> &'static str {
    label(match status {
        None => "usage.all_statuses",
        Some(ApiRequestOutcome::Succeeded) => "usage.status_succeeded",
        Some(ApiRequestOutcome::Failed) => "usage.status_failed",
        Some(ApiRequestOutcome::Cancelled) => "usage.status_cancelled",
    })
}

fn cell(value: impl Into<SharedString>, width: f32) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .truncate()
        .child(value.into())
}

fn numeric(value: impl Into<SharedString>, width: f32) -> Div {
    cell(value, width).text_right()
}

fn table_row() -> Div {
    h_flex()
        .w_full()
        .min_h(px(40.))
        .items_center()
        .gap_2()
        .px_4()
        .border_b_1()
        .border_color(rgb(palette().border))
        .text_size(ui_text_size(12.))
}

fn badge(value: usize) -> Div {
    div()
        .px_1()
        .rounded_sm()
        .bg(rgb(palette().surface_strong))
        .text_size(ui_text_size(10.))
        .text_color(rgb(palette().muted))
        .child(count(value))
}

fn overview_metric(title: &'static str, value: String, note: String) -> Div {
    v_flex()
        .debug_selector(move || format!("usage-metric-{title}"))
        .flex_1()
        .min_w(px(145.))
        .gap_1()
        .px_4()
        .py_3()
        .child(
            heading(title, note)
                .text_size(ui_text_size(11.))
                .text_color(rgb(palette().muted)),
        )
        .child(
            div()
                .text_size(ui_text_size(25.))
                .font_medium()
                .child(value),
        )
}

impl UsagePanel {
    pub(super) fn dashboard_header(&self, workspace: String, cx: &Context<Self>) -> Div {
        h_flex()
            .w_full()
            .items_center()
            .flex_wrap()
            .gap_2()
            .child(Icon::new(FullIconName::ChartColumn).small())
            .child(
                div()
                    .text_size(ui_text_size(18.))
                    .font_semibold()
                    .child(label("usage.title")),
            )
            .child(information("usage.title", label("usage.coverage")))
            .child(caption(workspace).min_w_0().truncate().flex_1())
            .child(
                Button::new("usage-analysis-toggle")
                    .small()
                    .outline()
                    .debug_selector(|| "usage-analysis-toggle".into())
                    .icon(FullIconName::Settings)
                    .label(label("usage.more_analysis"))
                    .selected(self.details_open)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.details_open = !this.details_open;
                        cx.notify();
                    })),
            )
    }

    pub(super) fn dashboard_filters(&self, report: &Report, cx: &Context<Self>) -> Div {
        let mut engines = h_flex()
            .items_center()
            .gap_1()
            .p_1()
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(palette().border))
            .child(
                Button::new("usage-engine-all")
                    .small()
                    .compact()
                    .ghost()
                    .debug_selector(|| "usage-engine-all".into())
                    .selected(self.selected_engine.is_none())
                    .label(label("usage.all_engines"))
                    .on_click(cx.listener(|this, _, _, cx| this.select_engine(None, cx))),
            );
        for engine in [
            Provider::Claude,
            Provider::Codex,
            Provider::OpenCode,
            Provider::Qoder,
            Provider::Gemini,
        ] {
            let id = engine.descriptor().id;
            engines = engines.child(
                Button::new(SharedString::from(format!("usage-engine-{id}")))
                    .debug_selector(move || format!("usage-engine-{id}"))
                    .small()
                    .compact()
                    .ghost()
                    .selected(self.selected_engine == Some(engine))
                    .accessibility_label(engine.label())
                    .tooltip(engine.label())
                    .child(provider_icon(engine))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.select_engine(Some(engine), cx)),
                    ),
            );
        }
        let scope = report
            .services
            .iter()
            .find(|service| self.selected_service.as_deref() == Some(&service.key))
            .map(service_title)
            .unwrap_or_else(|| label("usage.all_providers").into());
        let services: Vec<_> = report
            .services
            .iter()
            .map(|service| (service.key.clone(), service_title(service)))
            .collect();
        let owner = cx.entity().downgrade();
        h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .child(
                        Button::new("usage-previous-month")
                            .small()
                            .compact()
                            .ghost()
                            .icon(FullIconName::ChevronLeft)
                            .accessibility_label(label("usage.previous_month"))
                            .tooltip(label("usage.previous_month"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.shift_month(-1, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .w(px(106.))
                            .text_center()
                            .child(self.month.read(cx).value().to_string()),
                    )
                    .child(
                        Button::new("usage-next-month")
                            .small()
                            .compact()
                            .ghost()
                            .icon(FullIconName::ChevronRight)
                            .accessibility_label(label("usage.next_month"))
                            .tooltip(label("usage.next_month"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.shift_month(1, window, cx)),
                            ),
                    ),
            )
            .child(engines)
            .child(
                Button::new("usage-service-menu")
                    .small()
                    .outline()
                    .max_w(px(250.))
                    .label(scope)
                    .icon(FullIconName::ChevronDown)
                    .dropdown_menu(move |mut menu, _, _| {
                        let all_owner = owner.clone();
                        menu =
                            menu.item(PopupMenuItem::new(label("usage.all_providers")).on_click(
                                move |_, _, cx| {
                                    let _ = all_owner
                                        .update(cx, |this, cx| this.select_service(None, cx));
                                },
                            ));
                        for (key, name) in &services {
                            let owner = owner.clone();
                            let key = key.clone();
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.select_service(Some(key.clone()), cx)
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
    }

    fn select_engine(&mut self, engine: Option<Provider>, cx: &mut Context<Self>) {
        self.selected_engine = engine;
        self.select_service(None, cx);
    }

    fn select_service(&mut self, service: Option<String>, cx: &mut Context<Self>) {
        self.selected_service = service;
        self.shown = 20;
        self.model_details.clear();
        cx.notify();
    }

    pub(super) fn dashboard_overview(&self, report: &Report, truncated: bool) -> Div {
        let totals = &report.filtered;
        let mut coverage = message(
            "usage.coverage_count",
            &[
                ("complete", totals.complete_turns.to_string()),
                ("turns", totals.turns.to_string()),
            ],
        );
        if truncated {
            coverage.push_str(&format!("\n{}", label("usage.truncated")));
        }
        v_flex()
            .w_full()
            .min_w_0()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(
                h_flex()
                    .flex_wrap()
                    .child(overview_metric(
                        "usage.total_tokens",
                        format_tokens(totals.tokens()),
                        label("usage.coverage").into(),
                    ))
                    .child(overview_metric(
                        "usage.turns",
                        count(totals.turns),
                        label("usage.records_note").into(),
                    ))
                    .child(overview_metric(
                        "usage.cache_hit_rate",
                        report
                            .cache_hit_rate()
                            .map_or_else(|| "—".into(), |rate| format!("{rate:.1}%")),
                        label("usage.cache_hit_note").into(),
                    ))
                    .child(overview_metric(
                        "usage.data_completeness",
                        share(totals.complete_turns as u64, totals.turns as u64),
                        coverage,
                    )),
            )
    }

    pub(super) fn dashboard_daily(
        &self,
        report: &Report,
        insights: &Insights,
        truncated: bool,
        cx: &Context<Self>,
    ) -> Div {
        let totals = &report.filtered;
        let mut overview = card()
            .debug_selector(|| "usage-daily-card".into())
            .h(px(270.))
            .flex_1()
            .flex_basis(px(340.))
            .min_w_0();
        let mut chart_header = h_flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(
                heading("usage.trend", label("usage.daily_chart_note"))
                    .font_semibold()
                    .text_size(ui_text_size(16.)),
            )
            .child(div().flex_1());
        if totals.incomplete > 0 {
            chart_header = chart_header.child(
                caption(message(
                    "usage.incomplete_short",
                    &[("count", totals.incomplete.to_string())],
                ))
                .text_color(rgb(palette().warning_text)),
            );
        }
        if truncated {
            chart_header = chart_header.child(
                caption(label("usage.truncated_short")).text_color(rgb(palette().warning_text)),
            );
        }
        overview = overview.child(
            chart_header.child(
                Button::new("usage-chart-toggle")
                    .debug_selector(|| "usage-chart-toggle".into())
                    .small()
                    .compact()
                    .ghost()
                    .icon(if self.chart_open {
                        FullIconName::ChevronUp
                    } else {
                        FullIconName::ChevronDown
                    })
                    .accessibility_label(label(if self.chart_open {
                        "usage.hide_chart"
                    } else {
                        "usage.show_chart"
                    }))
                    .tooltip(label(if self.chart_open {
                        "usage.hide_chart"
                    } else {
                        "usage.show_chart"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.chart_open = !this.chart_open;
                        cx.notify();
                    })),
            ),
        );
        if self.chart_open {
            if let Some(peak) = insights.peak() {
                overview = overview.child(caption(message(
                    "usage.peak_day",
                    &[
                        ("date", peak.date.to_string()),
                        ("tokens", format_tokens(peak.tokens)),
                    ],
                )));
            }
            overview = overview.child(
                if insights.elapsed_days == 0 || insights.days.iter().all(|day| day.turns == 0) {
                    v_flex()
                        .h(px(150.))
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(caption(label("usage.no_matching_records")))
                        .child(caption(label("usage.empty_hint")))
                        .into_any_element()
                } else {
                    daily_chart(insights).into_any_element()
                },
            );
        }
        overview
    }

    pub(super) fn dashboard_table(
        &self,
        report: &Report,
        app: &ConduitApp,
        cx: &Context<Self>,
    ) -> Div {
        let mut tabs = h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap_1()
            .px_3()
            .border_b_1()
            .border_color(rgb(palette().border));
        for (tab, id, title, total) in [
            (
                TableTab::Records,
                "usage-group-records",
                "usage.records",
                report.records.len(),
            ),
            (
                TableTab::Providers,
                "usage-group-provider",
                "usage.providers",
                report.services.len(),
            ),
            (
                TableTab::Models,
                "usage-group-model",
                "usage.models",
                report.models.len(),
            ),
        ] {
            let active = self.table_tab == tab;
            tabs = tabs.child(
                div()
                    .py_1()
                    .border_b_2()
                    .border_color(rgb(if active {
                        palette().text
                    } else {
                        palette().surface
                    }))
                    .child(
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .small()
                            .ghost()
                            .selected(active)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(label(title))
                                    .child(badge(total)),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.table_tab = tab;
                                this.shown = 20;
                                cx.notify();
                            })),
                    ),
            );
        }
        tabs = tabs.child(div().flex_1());
        if self.table_tab == TableTab::Records {
            let owner = cx.entity().downgrade();
            tabs = tabs.child(
                Button::new("usage-status-menu")
                    .small()
                    .ghost()
                    .label(status_label(self.log_status))
                    .debug_selector(|| "usage-status-menu".into())
                    .icon(FullIconName::ChevronDown)
                    .tooltip(label("usage.status_filter_note"))
                    .dropdown_menu(move |mut menu, _, _| {
                        for status in [
                            None,
                            Some(ApiRequestOutcome::Succeeded),
                            Some(ApiRequestOutcome::Failed),
                            Some(ApiRequestOutcome::Cancelled),
                        ] {
                            let owner = owner.clone();
                            menu = menu.item(PopupMenuItem::new(status_label(status)).on_click(
                                move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.log_status = status;
                                        this.shown = 20;
                                        cx.notify();
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            );
        }
        let (body, total, measured) = match self.table_tab {
            TableTab::Records => {
                let (body, total, measured) = self.record_rows(report, app);
                (body, total, Some(measured))
            }
            TableTab::Providers | TableTab::Models => {
                let (body, total) = self.group_rows(report, cx);
                (body, total, None)
            }
        };
        let mut table = v_flex()
            .w_full()
            .min_w_0()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(tabs)
            .child(
                div()
                    .id("usage-table-scroll")
                    .debug_selector(|| "usage-table-viewport".into())
                    .w_full()
                    .overflow_x_scroll()
                    .child(body),
            );
        if total == 0 {
            table = table.child(
                v_flex()
                    .py_6()
                    .items_center()
                    .gap_2()
                    .child(caption(label("usage.no_matching_records")))
                    .child(caption(label("usage.try_filters"))),
            );
        }
        table.child(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_4()
                .py_2()
                .child(caption(message(
                    "usage.showing_records",
                    &[
                        ("shown", count(total.min(self.shown))),
                        ("total", count(total)),
                    ],
                )))
                .when_some(measured, |footer, measured| {
                    footer.child(
                        caption(message(
                            "usage.first_response_coverage",
                            &[("measured", count(measured)), ("total", count(total))],
                        ))
                        .id("usage-first-response-coverage")
                        .debug_selector(|| "usage-first-response-coverage".into())
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label(
                                "usage.first_response_note",
                            ))
                            .build(window, cx)
                        }),
                    )
                })
                .child(div().flex_1())
                .when(total > self.shown, |footer| {
                    footer.child(
                        Button::new("usage-more")
                            .small()
                            .ghost()
                            .label(label("usage.more"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.shown += 20;
                                cx.notify();
                            })),
                    )
                }),
        )
    }

    pub(super) fn record_rows(&self, report: &Report, app: &ConduitApp) -> (Div, usize, usize) {
        let rows: Vec<_> = report
            .records
            .iter()
            .map(|index| &app.store.usage_ledger.observations[*index])
            .filter(|record| {
                self.log_status
                    .is_none_or(|status| record.api_request_outcome == Some(status))
            })
            .collect();
        let mut body = v_flex()
            .debug_selector(|| "usage-records-table".into())
            .w_full()
            .min_w(px(940.))
            .child(
                table_row()
                    .min_h(px(34.))
                    .bg(rgb(palette().surface_strong))
                    .text_color(rgb(palette().muted))
                    .child(cell(label("usage.time"), 96.))
                    .child(cell(label("usage.engine"), 84.))
                    .child(cell(label("usage.provider"), 110.))
                    .child(div().flex_1().min_w(px(120.)).child(label("usage.model")))
                    .child(numeric(label("usage.new_input"), 66.))
                    .child(numeric(label("usage.output_short"), 60.))
                    .child(numeric(label("usage.cache_short"), 66.))
                    .child(
                        numeric(label("usage.first_response"), 80.)
                            .id("usage-first-response-header")
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(label(
                                    "usage.first_response_note",
                                ))
                                .build(window, cx)
                            }),
                    )
                    .child(numeric(label("usage.elapsed"), 60.))
                    .child(cell(label("usage.status"), 56.))
                    .child(div().w(px(28.))),
            );
        for (index, record) in rows.iter().take(self.shown).enumerate() {
            let time = chrono::DateTime::from_timestamp(
                i64::try_from(record.finished_at).unwrap_or(i64::MAX),
                0,
            )
            .map(|date| {
                date.with_timezone(&Local)
                    .format("%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| "—".into());
            let service_key = report::service_key(record.provider, &record.service);
            let service = report
                .services
                .iter()
                .find(|service| service.key == service_key)
                .map(service_title)
                .unwrap_or_else(|| label("usage.unavailable_provider").into());
            let uncached = record
                .input
                .zip(record.cached)
                .and_then(|(input, cached)| input.checked_sub(cached));
            let host = self.host.clone();
            let session_id = record.session;
            let session = app.session(session_id);
            let status = record
                .api_request_outcome
                .map(|status| status_label(Some(status)))
                .unwrap_or("—");
            let color = match record.api_request_outcome {
                Some(ApiRequestOutcome::Failed) => palette().danger_text,
                Some(ApiRequestOutcome::Cancelled) => palette().warning_text,
                _ => palette().muted,
            };
            body = body.child(
                table_row()
                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                    .child(cell(time, 96.))
                    .child(
                        h_flex()
                            .w(px(84.))
                            .flex_shrink_0()
                            .gap_1()
                            .items_center()
                            .child(provider_icon(record.provider))
                            .child(record.provider.label()),
                    )
                    .child(
                        div()
                            .id(("usage-record-service", index))
                            .w(px(110.))
                            .flex_shrink_0()
                            .truncate()
                            .tooltip({
                                let name = service.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(name.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(service),
                    )
                    .child(
                        div()
                            .id(("usage-record-model", index))
                            .flex_1()
                            .min_w(px(120.))
                            .truncate()
                            .tooltip({
                                let name = record
                                    .model
                                    .clone()
                                    .unwrap_or_else(|| label("usage.unknown_model").into());
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(name.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(record.model.clone().unwrap_or_else(|| "—".into())),
                    )
                    .child(numeric(token_value(uncached), 66.))
                    .child(numeric(token_value(record.output), 60.))
                    .child(numeric(token_value(record.cached), 66.))
                    .child(
                        numeric(first_response_value(record.api_first_response_ms), 80.)
                            .id(("usage-record-first-response", index))
                            .debug_selector(move || format!("usage-record-first-response-{index}"))
                            .when(record.api_first_response_ms.is_none(), |cell| {
                                cell.text_color(rgb(palette().muted))
                            })
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(label(
                                    "usage.first_response_note",
                                ))
                                .build(window, cx)
                            }),
                    )
                    .child(numeric(
                        format!("{:.1}s", record.elapsed_ms as f64 / 1000.),
                        60.,
                    ))
                    .child(cell(status, 56.).text_color(rgb(color)))
                    .child(
                        Button::new(("usage-record-open", index))
                            .debug_selector(move || format!("usage-record-open-{index}"))
                            .small()
                            .compact()
                            .ghost()
                            .w(px(28.))
                            .icon(FullIconName::ArrowUpRight)
                            .accessibility_label(label("usage.open_session"))
                            .tooltip(label("usage.open_session"))
                            .disabled(session.is_none_or(|session| session.archived))
                            .on_click(move |_, window, cx| {
                                let _ = host.update(cx, |app, cx| {
                                    app.select_document(
                                        files::DocumentTab::Chat(session_id),
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    ),
            );
        }
        let measured = rows
            .iter()
            .filter(|record| record.api_first_response_ms.is_some())
            .count();
        (body, rows.len(), measured)
    }

    fn group_rows(&self, report: &Report, cx: &Context<Self>) -> (Div, usize) {
        let providers = self.table_tab == TableTab::Providers;
        let mut body = v_flex().w_full().min_w(px(720.)).child(
            table_row()
                .min_h(px(34.))
                .bg(rgb(palette().surface_strong))
                .text_color(rgb(palette().muted))
                .child(div().flex_1().min_w(px(140.)).child(label(if providers {
                    "usage.provider"
                } else {
                    "usage.model"
                })))
                .child(numeric(label("usage.turns_short"), 58.))
                .child(numeric(label("usage.total_tokens"), 82.))
                .child(numeric(label("usage.input"), 75.))
                .child(numeric(label("usage.output_short"), 65.))
                .child(numeric(label("usage.cache_short"), 75.))
                .child(numeric(label("usage.usage_share"), 65.))
                .child(div().w(px(56.))),
        );
        let groups: Vec<_> = if providers {
            report
                .services
                .iter()
                .map(|service| (service.key.clone(), service_title(service), &service.totals))
                .collect()
        } else {
            report
                .models
                .iter()
                .map(|(model, totals)| {
                    (
                        format!("model:{}", model.as_deref().unwrap_or("")),
                        model
                            .clone()
                            .unwrap_or_else(|| label("usage.unknown_model").into()),
                        totals,
                    )
                })
                .collect()
        };
        let scope = if providers {
            report.services.iter().fold(0u64, |sum, service| {
                sum.saturating_add(service.totals.tokens())
            })
        } else {
            report.filtered.tokens()
        };
        for (index, (key, name, totals)) in groups.iter().take(self.shown).enumerate() {
            let selected = self.selected_service.as_ref() == Some(key);
            let expanded = self.model_details.contains(key);
            let key = key.clone();
            let action = Button::new(("usage-group-action", index))
                .small()
                .compact()
                .ghost()
                .w(px(56.))
                .debug_selector(move || format!("usage-provider-{index}"))
                .label(label(if providers {
                    if selected {
                        "usage.clear_filter"
                    } else {
                        "usage.filter"
                    }
                } else if expanded {
                    "usage.collapse_short"
                } else {
                    "usage.expand_short"
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if providers {
                        this.table_tab = TableTab::Models;
                        this.select_service(if selected { None } else { Some(key.clone()) }, cx);
                    } else {
                        if !this.model_details.remove(&key) {
                            this.model_details.insert(key.clone());
                        }
                        cx.notify();
                    }
                }));
            body = body.child(
                table_row()
                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                    .child(
                        div()
                            .id(("usage-group-name", index))
                            .flex_1()
                            .min_w(px(140.))
                            .truncate()
                            .tooltip({
                                let name = name.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(name.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(name.clone()),
                    )
                    .child(numeric(count(totals.turns), 58.))
                    .child(numeric(format_tokens(totals.tokens()), 82.).font_medium())
                    .child(numeric(
                        token_value((totals.input_records > 0).then_some(totals.input)),
                        75.,
                    ))
                    .child(numeric(
                        token_value((totals.output_records > 0).then_some(totals.output)),
                        65.,
                    ))
                    .child(numeric(
                        token_value((totals.cached_records > 0).then_some(totals.cached)),
                        75.,
                    ))
                    .child(numeric(share(totals.tokens(), scope), 65.))
                    .child(action),
            );
            if expanded && !providers {
                body = body.child(div().p_3().child(composition(totals)));
            }
        }
        (body, groups.len())
    }
}
