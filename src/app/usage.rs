//! Known usage and advisory token budgets; no inferred monetary billing.
use super::*;
use crate::usage_ledger::{ApiRequestOutcome, Observation, Totals};
use chrono::Months;
use gpui_kit::component::progress::Progress;
mod dashboard;
#[cfg(all(test, feature = "ui-tests"))]
mod dashboard_tests;
mod insights;
mod render;
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
            cache_write: reported
                .then_some(run.usage.cache_write_input_tokens)
                .flatten(),
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
                    label(if total >= limit {
                        "usage.budget_reached"
                    } else {
                        "usage.budget_near"
                    }),
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
                .key(
                    format!("usage/{}/{month}", workspace.display()),
                    if total >= limit {
                        "budget"
                    } else {
                        "budget-near"
                    },
                )
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
                selected_engine: None,
                log_status: None,
                table_tab: dashboard::TableTab::Records,
                chart_open: true,
                concentration_by_service: false,
                shown: 20,
                model_details: HashSet::new(),
                budget_open: false,
                details_open: false,
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
    selected_engine: Option<Provider>,
    log_status: Option<ApiRequestOutcome>,
    table_tab: dashboard::TableTab,
    chart_open: bool,
    concentration_by_service: bool,
    shown: usize,
    model_details: HashSet<String>,
    budget_open: bool,
    details_open: bool,
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
fn composition(total: &Totals) -> Div {
    let denominator = if total.incomplete == 0 && total.cached <= total.input {
        total.tokens()
    } else {
        0
    };
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(composition_metric(
            "usage.input",
            (total.input_records > 0).then_some(total.input),
            denominator,
        ))
        .child(composition_metric(
            "usage.output",
            (total.output_records > 0).then_some(total.output),
            denominator,
        ))
        .child(composition_metric(
            "usage.cached",
            (total.cached_records > 0).then_some(total.cached),
            denominator,
        ))
}
fn composition_metric(title: &'static str, value: Option<u64>, total: u64) -> Div {
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
                .child(value.map_or_else(|| "—".into(), format_tokens)),
        )
        .child(caption(
            value.map_or_else(|| "—".into(), |value| share(value, total)),
        ))
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
                    cache_write: None,
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
