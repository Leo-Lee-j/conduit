//! A bounded page of display values, rendered independently from the dashboard.
use super::{
    dashboard::{TableTab, count, first_response_value, status_label, token_value},
    *,
};
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use std::ops::Range;

pub(super) struct Page {
    pub index: usize,
    pub count: usize,
    pub range: Range<usize>,
}

impl Page {
    pub fn new(index: usize, size: usize, total: usize) -> Self {
        let count = total.div_ceil(size).max(1);
        let index = index.min(count - 1);
        let start = index * size;
        Self {
            index,
            count,
            range: start..start.saturating_add(size).min(total),
        }
    }
}

#[derive(PartialEq)]
enum Action {
    Session {
        id: u64,
        available: bool,
        provider: Provider,
        outcome: Option<ApiRequestOutcome>,
    },
    Group {
        key: String,
        selected: bool,
        expanded: bool,
    },
}

#[derive(PartialEq)]
pub(super) struct Row {
    index: usize,
    cells: Vec<SharedString>,
    action: Action,
}

pub(super) struct UsageTable {
    owner: WeakEntity<UsagePanel>,
    host: WeakEntity<ConduitApp>,
    pub tab: TableTab,
    pub rows: Vec<Row>,
    pub page: usize,
    pub locale: conduit::i18n::Locale,
}

impl UsageTable {
    pub fn new(owner: WeakEntity<UsagePanel>, host: WeakEntity<ConduitApp>) -> Self {
        Self {
            owner,
            host,
            tab: TableTab::Records,
            rows: Vec::new(),
            page: 0,
            locale: conduit::i18n::locale(),
        }
    }

    fn columns(&self) -> &[(&'static str, f32, bool)] {
        match self.tab {
            TableTab::Records => &[
                ("usage.time", 116., false),
                ("usage.engine", 100., false),
                ("usage.provider", 130., false),
                ("usage.model", 180., false),
                ("usage.new_input", 86., true),
                ("usage.output_short", 80., true),
                ("usage.cache_short", 86., true),
                ("usage.first_response", 100., true),
                ("usage.elapsed", 80., true),
                ("usage.status", 76., false),
                ("usage.open_session", 48., false),
            ],
            TableTab::Providers | TableTab::Models => &[
                ("usage.model", 240., false),
                ("usage.turns_short", 78., true),
                ("usage.total_tokens", 102., true),
                ("usage.input", 95., true),
                ("usage.output_short", 85., true),
                ("usage.cache_short", 95., true),
                ("usage.usage_share", 85., true),
                ("usage.filter", 76., false),
            ],
        }
    }
}

impl TableDelegate for UsageTable {
    fn render_last_empty_col(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        h_flex().h_full().flex_grow_1()
    }

    fn columns_count(&self, _: &App) -> usize {
        self.columns().len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col: usize, _: &App) -> Column {
        let (mut key, width, numeric) = self.columns()[col];
        if self.tab == TableTab::Providers && col == 0 {
            key = "usage.provider";
        }
        Column::new(
            key,
            if col + 1 == self.columns().len() {
                ""
            } else {
                label(key)
            },
        )
        .width(px(width))
        .min_width(px(48.))
        .movable(false)
        .when(numeric, |column| column.text_right())
    }

    fn render_th(
        &mut self,
        col: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .id(("usage-header", col))
            .w_full()
            .truncate()
            .child(self.column(col, cx).name)
            .when(self.tab == TableTab::Records && col == 7, |cell| {
                cell.tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(label("usage.first_response_note"))
                        .build(window, cx)
                })
            })
    }

    fn render_tr(
        &mut self,
        row: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> gpui_kit::Stateful<Div> {
        let index = self.rows[row].index;
        div()
            .id(("usage-row", index))
            .debug_selector(move || format!("usage-table-row-{index}"))
            .text_size(ui_text_size(12.))
    }

    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row];
        let index = row.index;
        let value = row.cells[col].clone();
        match &row.action {
            Action::Session { provider, .. } if col == 1 => {
                return h_flex()
                    .gap_1()
                    .items_center()
                    .child(provider_icon(*provider))
                    .child(value)
                    .into_any_element();
            }
            Action::Session { id, available, .. } if col == 10 => {
                let host = self.host.clone();
                let id = *id;
                return Button::new(("usage-record-open", index))
                    .debug_selector(move || format!("usage-record-open-{index}"))
                    .small()
                    .compact()
                    .ghost()
                    .icon(FullIconName::ArrowUpRight)
                    .accessibility_label(label("usage.open_session"))
                    .tooltip(label("usage.open_session"))
                    .disabled(!available)
                    .on_click(move |_, window, cx| {
                        let _ = host.update(cx, |app, cx| {
                            app.select_document(files::DocumentTab::Chat(id), window, cx)
                        });
                    })
                    .into_any_element();
            }
            Action::Group {
                key,
                selected,
                expanded,
            } if col == 7 => {
                let owner = self.owner.clone();
                let key = key.clone();
                let selected = *selected;
                let providers = self.tab == TableTab::Providers;
                return Button::new(("usage-group-action", index))
                    .debug_selector(move || format!("usage-provider-{index}"))
                    .small()
                    .compact()
                    .ghost()
                    .label(label(if providers {
                        if selected {
                            "usage.clear_filter"
                        } else {
                            "usage.filter"
                        }
                    } else if *expanded {
                        "usage.collapse_short"
                    } else {
                        "usage.expand_short"
                    }))
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            if providers {
                                this.table_tab = TableTab::Models;
                                this.select_service(
                                    if selected { None } else { Some(key.clone()) },
                                    cx,
                                );
                            } else {
                                if !this.model_details.remove(&key) {
                                    this.model_details.insert(key.clone());
                                }
                                cx.notify();
                            }
                        });
                    })
                    .into_any_element();
            }
            _ => {}
        }
        let mut cell = div()
            .id(("usage-cell", index * 11 + col))
            .w_full()
            .truncate()
            .child(value.clone());
        if let Action::Session { outcome, .. } = row.action {
            if col == 9 {
                cell = cell.text_color(rgb(match outcome {
                    Some(ApiRequestOutcome::Failed) => palette().danger_text,
                    Some(ApiRequestOutcome::Cancelled) => palette().warning_text,
                    _ => palette().muted,
                }));
            }
            if col == 7 {
                return cell
                    .debug_selector(move || format!("usage-record-first-response-{index}"))
                    .tooltip(|window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(label(
                            "usage.first_response_note",
                        ))
                        .build(window, cx)
                    })
                    .into_any_element();
            }
        }
        cell.tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(value.clone()).build(window, cx)
        })
        .into_any_element()
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(caption(label("usage.no_matching_records")))
            .child(caption(label("usage.try_filters")))
    }
}

impl UsagePanel {
    pub(super) fn table_rows(
        &self,
        snapshot: &cache::Snapshot,
        app: &ConduitApp,
        range: Range<usize>,
    ) -> Vec<Row> {
        let report = snapshot.report();
        range
            .map(|index| {
                if self.table_tab == TableTab::Records {
                    let record = &app.store.usage_ledger.observations[snapshot.records[index]];
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
                    let key = report::service_key(record.provider, &record.service);
                    let service = report
                        .services
                        .iter()
                        .find(|service| service.key == key)
                        .map(service_title)
                        .unwrap_or_else(|| label("usage.unavailable_provider").into());
                    let uncached = record
                        .input
                        .zip(record.cached)
                        .and_then(|(input, cached)| input.checked_sub(cached));
                    Row {
                        index,
                        cells: vec![
                            time,
                            record.provider.label().into(),
                            service,
                            record.model.clone().unwrap_or_else(|| "—".into()),
                            token_value(uncached),
                            token_value(record.output),
                            token_value(record.cached),
                            first_response_value(record.api_first_response_ms),
                            format!("{:.1}s", record.elapsed_ms as f64 / 1000.),
                            record
                                .api_request_outcome
                                .map(|status| status_label(Some(status)))
                                .unwrap_or("—")
                                .into(),
                            String::new(),
                        ]
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                        action: Action::Session {
                            id: record.session,
                            available: app
                                .session(record.session)
                                .is_some_and(|session| !session.archived),
                            provider: record.provider,
                            outcome: record.api_request_outcome,
                        },
                    }
                } else {
                    let (key, name, totals, scope) = if self.table_tab == TableTab::Providers {
                        let service = &report.services[index];
                        (
                            service.key.clone(),
                            service_title(service),
                            &service.totals,
                            report.services.iter().fold(0u64, |sum, service| {
                                sum.saturating_add(service.totals.tokens())
                            }),
                        )
                    } else {
                        let (model, totals) = &report.models[index];
                        (
                            format!("model:{}", model.as_deref().unwrap_or("")),
                            model
                                .clone()
                                .unwrap_or_else(|| label("usage.unknown_model").into()),
                            totals,
                            report.filtered.tokens(),
                        )
                    };
                    Row {
                        index,
                        cells: vec![
                            name,
                            count(totals.turns),
                            format_tokens(totals.tokens()),
                            token_value((totals.input_records > 0).then_some(totals.input)),
                            token_value((totals.output_records > 0).then_some(totals.output)),
                            token_value((totals.cached_records > 0).then_some(totals.cached)),
                            share(totals.tokens(), scope),
                            String::new(),
                        ]
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                        action: Action::Group {
                            selected: self.selected_service.as_ref() == Some(&key),
                            expanded: self.model_details.contains(&key),
                            key,
                        },
                    }
                }
            })
            .collect()
    }
}
