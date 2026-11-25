//! Keep ledger aggregation out of hover, scroll and pagination redraws.
use super::{insights::Insights, *};
use chrono::NaiveDate;
use std::rc::Rc;

#[derive(PartialEq)]
struct Key {
    revision: u64,
    records: usize,
    truncated: bool,
    month: String,
    today: NaiveDate,
    service: Option<String>,
    engine: Option<Provider>,
    status: Option<ApiRequestOutcome>,
    providers: Vec<(Provider, String, String)>,
}

pub(super) struct Snapshot {
    pub all_report: Report,
    pub all_insights: Insights,
    filtered_report: Option<Report>,
    filtered_insights: Option<Insights>,
    pub records: Vec<usize>,
    pub measured: usize,
}

impl Snapshot {
    pub fn row_count(&self, tab: dashboard::TableTab) -> usize {
        match tab {
            dashboard::TableTab::Records => self.records.len(),
            dashboard::TableTab::Providers => self.report().services.len(),
            dashboard::TableTab::Models => self.report().models.len(),
        }
    }

    pub fn report(&self) -> &Report {
        self.filtered_report.as_ref().unwrap_or(&self.all_report)
    }

    pub fn insights(&self) -> &Insights {
        self.filtered_insights
            .as_ref()
            .unwrap_or(&self.all_insights)
    }
}

#[derive(Default)]
pub(super) struct Cache {
    entry: Option<(Key, Rc<Snapshot>)>,
}

#[cfg(all(test, feature = "ui-tests"))]
impl Cache {
    pub fn snapshot(&self) -> &Rc<Snapshot> {
        &self.entry.as_ref().expect("dashboard rendered").1
    }
}

impl UsagePanel {
    pub(super) fn snapshot(
        &mut self,
        app: &ConduitApp,
        month: &str,
        first: NaiveDate,
        today: NaiveDate,
    ) -> Rc<Snapshot> {
        let ledger = &app.store.usage_ledger;
        let key = Key {
            revision: ledger.revision(),
            records: ledger.observations.len(),
            truncated: ledger.truncated,
            month: month.into(),
            today,
            service: self.selected_service.clone(),
            engine: self.selected_engine,
            status: self.log_status,
            providers: app
                .store
                .providers
                .iter()
                .map(|provider| (provider.engine, provider.id.clone(), provider.name.clone()))
                .collect(),
        };
        if let Some((previous, snapshot)) = &self.cache.entry
            && previous == &key
        {
            return snapshot.clone();
        }
        let all_report = Report::build(
            &ledger.observations,
            &self.workspace,
            month,
            &app.store.providers,
            None,
            None,
        );
        let filtered_report = (self.selected_service.is_some() || self.selected_engine.is_some())
            .then(|| {
                Report::build(
                    &ledger.observations,
                    &self.workspace,
                    month,
                    &app.store.providers,
                    self.selected_service.as_deref(),
                    self.selected_engine,
                )
            });
        let all_insights = Insights::build(
            ledger,
            &self.workspace,
            first,
            today,
            None,
            &all_report,
            None,
        );
        let filtered_insights = filtered_report.as_ref().map(|report| {
            Insights::build(
                ledger,
                &self.workspace,
                first,
                today,
                self.selected_service.as_deref(),
                report,
                self.selected_engine,
            )
        });
        let report = filtered_report.as_ref().unwrap_or(&all_report);
        let records: Vec<_> = report
            .records
            .iter()
            .copied()
            .filter(|index| {
                self.log_status.is_none_or(|status| {
                    ledger.observations[*index].api_request_outcome == Some(status)
                })
            })
            .collect();
        let measured = records
            .iter()
            .filter(|index| ledger.observations[**index].api_first_response_ms.is_some())
            .count();
        let snapshot = Rc::new(Snapshot {
            all_report,
            all_insights,
            filtered_report,
            filtered_insights,
            records,
            measured,
        });
        self.cache.entry = Some((key, snapshot.clone()));
        snapshot
    }
}
