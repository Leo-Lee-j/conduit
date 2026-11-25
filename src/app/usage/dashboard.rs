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

pub(super) fn count(value: usize) -> String {
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

pub(super) fn token_value(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), format_tokens)
}

pub(super) fn first_response_value(value: Option<u64>) -> String {
    value.map_or_else(
        || label("usage.first_response_unrecorded").into(),
        |ms| format!("{:.1}s", ms as f64 / 1000.),
    )
}

pub(super) fn status_label(status: Option<ApiRequestOutcome>) -> &'static str {
    label(match status {
        None => "usage.all_statuses",
        Some(ApiRequestOutcome::Succeeded) => "usage.status_succeeded",
        Some(ApiRequestOutcome::Failed) => "usage.status_failed",
        Some(ApiRequestOutcome::Cancelled) => "usage.status_cancelled",
    })
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

    pub(super) fn select_service(&mut self, service: Option<String>, cx: &mut Context<Self>) {
        self.selected_service = service;
        self.page = 0;
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
        &mut self,
        snapshot: &cache::Snapshot,
        rows: Vec<table::Row>,
        cx: &mut Context<Self>,
    ) -> Div {
        let report = snapshot.report();
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
                                this.page = 0;
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
                                        this.page = 0;
                                        cx.notify();
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            );
        }
        let total = snapshot.row_count(self.table_tab);
        let measured = (self.table_tab == TableTab::Records).then_some(snapshot.measured);
        let page = table::Page::new(self.page, self.page_size, total);
        self.page = page.index;
        let tab = self.table_tab;
        let locale = conduit::i18n::locale();
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            let columns_changed = delegate.tab != tab || delegate.locale != locale;
            let page_changed = columns_changed || delegate.page != page.index;
            let rows_changed = delegate.rows != rows;
            if columns_changed || rows_changed || page_changed {
                delegate.tab = tab;
                delegate.page = page.index;
                delegate.locale = locale;
                delegate.rows = rows;
                if columns_changed {
                    table.refresh(cx);
                }
                if page_changed || rows_changed {
                    table.scroll_to_row(0, cx);
                }
                cx.notify();
            }
        });
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
                    .h(px(34. + 36. * page.range.len().clamp(2, 10) as f32 + 12.))
                    .child(
                        div()
                            .debug_selector(|| "usage-records-table".into())
                            .size_full()
                            .child(
                                gpui_kit::component::table::DataTable::new(&self.table)
                                    .with_size(px(36.))
                                    .header_row_height(px(34.))
                                    .bordered(false),
                            ),
                    ),
            );
        if self.table_tab == TableTab::Models {
            for index in page.range.clone() {
                let (model, totals) = &report.models[index];
                let key = format!("model:{}", model.as_deref().unwrap_or(""));
                if self.model_details.contains(&key) {
                    table = table.child(
                        v_flex()
                            .id(("usage-model-detail", index))
                            .debug_selector(move || format!("usage-model-detail-{index}"))
                            .p_3()
                            .gap_2()
                            .child(caption(
                                model
                                    .clone()
                                    .unwrap_or_else(|| label("usage.unknown_model").into()),
                            ))
                            .child(composition(totals)),
                    );
                }
            }
        }
        table.child(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .px_4()
                .py_2()
                .child(caption(message(
                    "usage.page_range",
                    &[
                        (
                            "start",
                            count(if total == 0 { 0 } else { page.range.start + 1 }),
                        ),
                        ("end", count(page.range.end)),
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
                .child({
                    let owner = cx.entity().downgrade();
                    Button::new("usage-page-size")
                        .debug_selector(|| "usage-page-size".into())
                        .small()
                        .ghost()
                        .icon(FullIconName::ChevronDown)
                        .label(message(
                            "usage.page_size",
                            &[("count", count(self.page_size))],
                        ))
                        .dropdown_menu(move |mut menu, _, _| {
                            for size in [20, 50, 100] {
                                let owner = owner.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(message(
                                        "usage.page_size",
                                        &[("count", count(size))],
                                    ))
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.page_size = size;
                                                this.page = 0;
                                                cx.notify();
                                            });
                                        },
                                    ),
                                );
                            }
                            menu
                        })
                })
                .child(caption(message(
                    "usage.page_number",
                    &[
                        ("page", count(page.index + 1)),
                        ("pages", count(page.count)),
                    ],
                )))
                .children(
                    [
                        (
                            "usage-first-page",
                            "usage.first_page",
                            FullIconName::ChevronsLeft,
                            0,
                            page.index == 0,
                        ),
                        (
                            "usage-prev-page",
                            "usage.previous_page",
                            FullIconName::ChevronLeft,
                            page.index.saturating_sub(1),
                            page.index == 0,
                        ),
                        (
                            "usage-next-page",
                            "usage.next_page",
                            FullIconName::ChevronRight,
                            (page.index + 1).min(page.count - 1),
                            page.index + 1 == page.count,
                        ),
                        (
                            "usage-last-page",
                            "usage.last_page",
                            FullIconName::ChevronsRight,
                            page.count - 1,
                            page.index + 1 == page.count,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, title, icon, target, disabled)| {
                        Button::new(id)
                            .debug_selector(move || id.into())
                            .small()
                            .compact()
                            .ghost()
                            .icon(icon)
                            .accessibility_label(label(title))
                            .tooltip(label(title))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.page = target;
                                cx.notify();
                            }))
                    }),
                ),
        )
    }
}
