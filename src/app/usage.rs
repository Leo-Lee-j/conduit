//! Known usage and advisory token budgets; no inferred monetary billing.
use super::*;
use crate::usage_ledger::{self, Observation, Totals};

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

    pub(super) fn usage_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("usage-ledger")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::ChartColumn)
            .accessibility_label(label("usage.title"))
            .tooltip(label("usage.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| this.open_usage(window, cx)))
    }

    fn open_usage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
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
                cx.observe(&host, |_, _, cx| cx.notify()),
                cx.subscribe(&month, |this: &mut UsagePanel, _, _: &InputEvent, cx| {
                    this.shown = 60;
                    cx.notify();
                }),
            ];
            UsagePanel {
                host: host.downgrade(),
                workspace,
                month,
                budget,
                error: None,
                shown: 60,
                _subscriptions: subscriptions,
            }
        });
        window.open_dialog(cx, move |dialog, window, _| {
            let panel = panel.clone();
            dialog
                .title(label("usage.title"))
                .w(px(
                    (f32::from(window.viewport_size().width) - 48.).clamp(280., 900.)
                ))
                .content(move |content, _, _| content.child(panel.clone()))
        });
    }
}

struct UsagePanel {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    month: Entity<InputState>,
    budget: Entity<InputState>,
    error: Option<String>,
    shown: usize,
    _subscriptions: Vec<Subscription>,
}
impl UsagePanel {
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

fn summary(totals: &Totals) -> String {
    conduit::i18n::format(
        "usage.summary",
        &[
            ("turns", totals.turns.to_string()),
            ("input", totals.input.to_string()),
            ("cached", totals.cached.to_string()),
            ("output", totals.output.to_string()),
            ("incomplete", totals.incomplete.to_string()),
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
        let month = self.month.read(cx).value().to_string();
        let valid = month.len() == 7
            && chrono::NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d").is_ok();
        let mut body = v_flex()
            .id("usage-ledger-panel")
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 180.).max(160.)
            ))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .child(self.workspace.display().to_string()),
            )
            .child(label("usage.coverage"))
            .child(label("usage.month"))
            .child(Input::new(&self.month))
            .child(label("usage.budget_hint"))
            .child(Input::new(&self.budget))
            .child(
                Button::new("usage-save-budget")
                    .small()
                    .label(label("usage.save_budget"))
                    .disabled(!writable)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(error) = &app.persistence.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if !valid {
            return body.child(label("usage.invalid_month")).into_any_element();
        }
        if app.store.usage_ledger.truncated {
            body = body.child(
                div()
                    .text_color(rgb(palette().warning_text))
                    .child(label("usage.truncated")),
            );
        }
        let totals = app.store.usage_ledger.monthly(&self.workspace, &month);
        body = body.child(div().text_size(ui_text_size(13.)).child(summary(&totals)));
        if totals.elapsed_ms > 0 {
            body = body.child(conduit::i18n::format(
                "usage.speed",
                &[(
                    "rate",
                    format!(
                        "{:.1}",
                        totals.output as f64 * 1000. / totals.elapsed_ms as f64
                    ),
                )],
            ));
        }
        let mut groups: BTreeMap<(String, String, String), Totals> = BTreeMap::new();
        for record in app.store.usage_ledger.observations.iter().filter(|o| {
            o.workspace == self.workspace && usage_ledger::date(o.finished_at).starts_with(&month)
        }) {
            groups
                .entry((
                    usage_ledger::date(record.finished_at),
                    record.provider.label().into(),
                    record.service.clone(),
                ))
                .or_default()
                .add(record);
        }
        for ((day, engine, service), totals) in groups.iter().rev().take(self.shown) {
            body = body.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(format!("{day} · {engine} · {service}"))
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(summary(totals)),
                    ),
            );
        }
        if groups.len() > self.shown {
            body = body.child(
                Button::new("usage-more")
                    .small()
                    .label(label("usage.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shown += 60;
                        cx.notify();
                    })),
            );
        }
        body.into_any_element()
    }
}
