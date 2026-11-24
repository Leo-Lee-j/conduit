use super::{ApiRequestOutcome, Observation, Provider, Report, unix_now};
use gpui_kit::{Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size};

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        let _ = window.draw(cx);
        if window.simulate_next_frame(cx) > 0 {
            let _ = window.draw(cx);
        }
    });
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let center = cx.debug_bounds(selector).expect(selector).center();
    cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
    draw(cx);
}

#[gpui_kit::test]
fn dashboard_filters_reflow_and_chart_toggle_preserve_ledger_scope(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (host, cx, root) = super::super::queue::tests::fixture(cx, "usage-dashboard");
    cx.simulate_resize(size(px(1500.), px(900.)));
    cx.update(|window, cx| {
        host.update(cx, |app, cx| {
            app.startup = None;
            for (index, provider) in [Provider::Claude, Provider::Claude, Provider::Codex]
                .into_iter()
                .enumerate()
            {
                app.store.usage_ledger.record(Observation {
                    key: format!("dashboard-{index}"),
                    session: 1,
                    workspace: root.clone(),
                    provider,
                    service: "cli".into(),
                    service_name: None,
                    model: Some("model-a".into()),
                    finished_at: unix_now(),
                    input: Some(1000),
                    cached: Some(500),
                    cache_write: None,
                    output: Some(100),
                    elapsed_ms: 2000,
                    incomplete: false,
                    api_first_response_ms: (index != 0).then_some(6306),
                    api_request_outcome: Some(if index == 0 {
                        ApiRequestOutcome::Failed
                    } else {
                        ApiRequestOutcome::Succeeded
                    }),
                });
            }
            app.open_usage(window, cx);
        });
    });
    draw(cx);
    let panel = cx.update(|_, cx| {
        let app = host.read(cx);
        app.usage_ui.panels[&app.usage_ui.active.unwrap()].clone()
    });
    for selector in [
        "usage-daily-card",
        "usage-concentration-card",
        "usage-ranking-chart-region",
        "usage-budget-card",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "{selector}");
    }
    let daily_bounds = cx.debug_bounds("usage-daily-card").unwrap();
    let concentration_bounds = cx.debug_bounds("usage-concentration-card").unwrap();
    assert_eq!(daily_bounds.size.height, concentration_bounds.size.height);
    click(cx, "usage-concentration-provider");
    cx.update(|_, cx| {
        let panel = panel.read(cx);
        assert!(panel.concentration_by_service);
        assert!(matches!(
            panel.table_tab,
            super::dashboard::TableTab::Records
        ));
        assert!(panel.selected_service.is_none());
    });
    click(cx, "usage-concentration-model");
    click(cx, "usage-edit-reminder");
    assert!(cx.debug_bounds("usage-budget-settings").is_some());
    cx.update(|_, cx| assert!(!panel.read(cx).details_open));
    click(cx, "usage-edit-reminder");
    assert!(cx.debug_bounds("usage-budget-settings").is_none());
    assert!(cx.debug_bounds("usage-record-open-2").is_some());
    assert!(cx.debug_bounds("usage-record-first-response-2").is_some());
    assert!(cx.debug_bounds("usage-first-response-coverage").is_some());
    cx.update(|_, cx| {
        let app = host.read(cx);
        let panel = panel.read(cx);
        let month = panel.month.read(cx).value().to_string();
        let report = Report::build(
            &app.store.usage_ledger.observations,
            &root,
            &month,
            &[],
            None,
            None,
        );
        let (_, total, measured) = panel.record_rows(&report, app);
        assert_eq!((measured, total), (2, 3));
    });
    click(cx, "usage-group-provider");
    assert!(cx.debug_bounds("usage-first-response-coverage").is_none());
    click(cx, "usage-provider-0");
    cx.update(|_, cx| assert!(panel.read(cx).selected_service.is_some()));
    click(cx, "usage-engine-codex");
    cx.update(|_, cx| {
        assert_eq!(panel.read(cx).selected_engine, Some(Provider::Codex));
        assert!(panel.read(cx).selected_service.is_none());
    });
    click(cx, "usage-group-records");
    assert!(cx.debug_bounds("usage-record-open-0").is_some());
    assert!(cx.debug_bounds("usage-record-open-1").is_none());
    click(cx, "usage-chart-toggle");
    assert!(cx.debug_bounds("usage-daily-chart-region").is_none());
    assert_eq!(
        cx.debug_bounds("usage-daily-card").unwrap().size.height,
        cx.debug_bounds("usage-concentration-card")
            .unwrap()
            .size
            .height
    );
    click(cx, "usage-chart-toggle");
    assert!(cx.debug_bounds("usage-daily-chart-region").is_some());
    click(cx, "usage-engine-all");
    cx.update(|_, cx| {
        panel.update(cx, |panel, cx| {
            panel.log_status = Some(ApiRequestOutcome::Failed);
            cx.notify();
        })
    });
    draw(cx);
    assert!(cx.debug_bounds("usage-record-open-0").is_some());
    assert!(cx.debug_bounds("usage-record-open-1").is_none());
    for width in [1500., 1000., 820., 1500.] {
        cx.simulate_resize(size(px(width), px(900.)));
        draw(cx);
        let panel_bounds = cx.debug_bounds("usage-ledger-panel").unwrap();
        let viewport = cx.debug_bounds("usage-table-viewport").unwrap();
        assert!(viewport.origin.x >= panel_bounds.origin.x);
        assert!(viewport.right() <= panel_bounds.right());
        for name in [
            "usage-daily-card",
            "usage-concentration-card",
            "usage-budget-card",
            "usage-metric-usage.total_tokens",
            "usage-metric-usage.turns",
            "usage-metric-usage.cache_hit_rate",
            "usage-metric-usage.data_completeness",
        ] {
            let metric = cx.debug_bounds(name).unwrap();
            assert!(metric.origin.x >= panel_bounds.origin.x);
            assert!(metric.right() <= panel_bounds.right());
        }
        let table = cx.debug_bounds("usage-records-table").unwrap();
        let action = cx.debug_bounds("usage-record-open-0").unwrap();
        assert!(
            action.right() <= table.right(),
            "the last column must be inside the scrollable table"
        );
    }
    cx.update(|_, cx| {
        let app = host.read(cx);
        let month = panel.read(cx).month.read(cx).value().to_string();
        let report = Report::build(
            &app.store.usage_ledger.observations,
            &root,
            &month,
            &[],
            None,
            None,
        );
        assert_eq!(report.filtered.turns, 3, "status only filters the log");
        assert_eq!(report.cache_hit_rate(), Some(50.));
        let (_, total, measured) = panel.read(cx).record_rows(&report, app);
        assert_eq!(
            (measured, total),
            (0, 1),
            "first reply coverage follows the outcome filter"
        );
    });
    assert_eq!(super::dashboard::first_response_value(Some(6306)), "6.3s");
    assert_eq!(super::dashboard::first_response_value(Some(0)), "0.0s");
    assert_eq!(super::dashboard::first_response_value(None), "未记录");
}

#[gpui_kit::test]
fn budget_settings_alert_for_all_engines_and_can_be_disabled(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (host, cx, root) = super::super::queue::tests::fixture(cx, "usage-budget-settings");
    cx.simulate_resize(size(px(1500.), px(1100.)));
    cx.update(|window, cx| {
        host.update(cx, |app, cx| {
            app.startup = None;
            for (index, provider) in [Provider::Claude, Provider::Codex].into_iter().enumerate() {
                app.store.usage_ledger.record(Observation {
                    key: format!("budget-{index}"),
                    session: 1,
                    workspace: root.clone(),
                    provider,
                    service: "cli".into(),
                    service_name: None,
                    model: Some("model-a".into()),
                    finished_at: unix_now(),
                    input: Some(41),
                    cached: None,
                    cache_write: None,
                    output: Some(0),
                    elapsed_ms: 1000,
                    incomplete: false,
                    api_first_response_ms: None,
                    api_request_outcome: None,
                });
            }
            app.open_usage(window, cx);
        });
    });
    draw(cx);
    let panel = cx.update(|_, cx| {
        let app = host.read(cx);
        app.usage_ui.panels[&app.usage_ui.active.unwrap()].clone()
    });
    click(cx, "usage-engine-codex");
    click(cx, "usage-edit-reminder");
    cx.update(|window, cx| {
        panel.update(cx, |panel, cx| {
            panel
                .budget
                .update(cx, |input, cx| input.set_value("invalid", window, cx));
        });
    });
    draw(cx);
    click(cx, "usage-save-budget");
    cx.update(|_, cx| {
        assert!(panel.read(cx).error.is_some());
        assert!(host.read(cx).store.usage_ledger.budgets.is_empty());
    });
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.set_budget(100, window, cx)));
    draw(cx);
    click(cx, "usage-save-budget");
    let month = chrono::Local::now().format("%Y-%m").to_string();
    cx.update(|_, cx| {
        assert!(panel.read(cx).error.is_none());
        let app = host.read(cx);
        let budget = &app.store.usage_ledger.budgets[0];
        assert_eq!(budget.monthly_tokens, 100);
        assert_eq!(budget.near_notified_month.as_deref(), Some(month.as_str()));
        assert!(budget.notified_month.is_none());
        assert_eq!(app.store.usage_ledger.monthly(&root, &month).tokens(), 82);
    });
    cx.update(|_, cx| {
        host.update(cx, |app, cx| {
            let mut observation = app.store.usage_ledger.observations[0].clone();
            observation.input = Some(59);
            app.store.usage_ledger.record(observation);
            app.check_usage_budget(&root, cx);
            assert_eq!(
                app.store.usage_ledger.budgets[0].notified_month.as_deref(),
                Some(month.as_str())
            );
            assert!(app.store.usage_ledger.take_alert(&root, &month).is_none());
        });
    });
    draw(cx);
    cx.update(|window, cx| panel.update(cx, |panel, cx| panel.set_budget(0, window, cx)));
    draw(cx);
    click(cx, "usage-save-budget");
    cx.update(|_, cx| {
        host.update(cx, |app, _| {
            assert_eq!(app.store.usage_ledger.budgets[0].monthly_tokens, 0);
            assert!(app.store.usage_ledger.take_alert(&root, &month).is_none());
        });
    });
}
