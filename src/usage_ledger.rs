//! Per-turn observations, distinct from imported cumulative session counters.
use crate::domain::Provider;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub key: String,
    pub session: u64,
    pub workspace: PathBuf,
    pub provider: Provider,
    pub service: String,
    /// Display name captured with the observation; survives provider removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    pub model: Option<String>,
    pub finished_at: u64,
    pub input: Option<u64>,
    pub cached: Option<u64>,
    pub output: Option<u64>,
    pub elapsed_ms: u64,
    pub incomplete: bool,
    /// End-to-end time to the first visible assistant output, not pure network latency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_first_response_ms: Option<u64>,
    /// Outcome for a dispatched chat request; absent on older records/non-chat operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_request_outcome: Option<ApiRequestOutcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiRequestOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budget {
    pub workspace: PathBuf,
    pub monthly_tokens: u64,
    pub notified_month: Option<String>,
    #[serde(default)]
    pub near_notified_month: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledger {
    pub observations: Vec<Observation>,
    pub budgets: Vec<Budget>,
    pub truncated: bool,
}

#[derive(Default)]
pub struct Totals {
    pub turns: usize,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub incomplete: usize,
    pub elapsed_ms: u64,
    pub complete_turns: usize,
    pub complete_tokens: u64,
    pub input_records: usize,
    pub cached_records: usize,
    pub output_records: usize,
    pub requests: usize,
    pub failed_requests: usize,
}
impl Totals {
    pub fn add(&mut self, observation: &Observation) {
        self.turns += 1;
        self.input = self.input.saturating_add(observation.input.unwrap_or(0));
        self.cached = self.cached.saturating_add(observation.cached.unwrap_or(0));
        self.output = self.output.saturating_add(observation.output.unwrap_or(0));
        self.input_records += usize::from(observation.input.is_some());
        self.cached_records += usize::from(observation.cached.is_some());
        self.output_records += usize::from(observation.output.is_some());
        let incomplete =
            observation.incomplete || observation.input.is_none() || observation.output.is_none();
        self.incomplete += usize::from(incomplete);
        if !incomplete {
            self.complete_turns += 1;
            self.complete_tokens = self.complete_tokens.saturating_add(
                observation
                    .input
                    .unwrap_or(0)
                    .saturating_add(observation.output.unwrap_or(0)),
            );
        }
        if let Some(outcome) = observation.api_request_outcome
            && outcome != ApiRequestOutcome::Cancelled
        {
            self.requests += 1;
            self.failed_requests += usize::from(outcome == ApiRequestOutcome::Failed);
        }
        if observation.output.is_some() {
            self.elapsed_ms = self.elapsed_ms.saturating_add(observation.elapsed_ms);
        }
    }
    pub fn tokens(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
    pub fn average(&self) -> Option<u64> {
        (self.complete_turns > 0).then(|| self.complete_tokens / self.complete_turns as u64)
    }
}

pub fn date(timestamp: u64) -> String {
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default()
}

impl Ledger {
    pub fn record(&mut self, observation: Observation) {
        if let Some(existing) = self
            .observations
            .iter_mut()
            .find(|o| o.key == observation.key)
        {
            *existing = observation;
            return;
        }
        if self.observations.len() >= 10_000 {
            self.observations.remove(0);
            self.truncated = true;
        }
        self.observations.push(observation);
    }
    pub fn monthly(&self, workspace: &Path, month: &str) -> Totals {
        let mut totals = Totals::default();
        for record in self
            .observations
            .iter()
            .filter(|o| o.workspace == workspace && date(o.finished_at).starts_with(month))
        {
            totals.add(record);
        }
        totals
    }
    pub fn save_budget(&mut self, workspace: PathBuf, monthly_tokens: u64) {
        if let Some(budget) = self.budgets.iter_mut().find(|b| b.workspace == workspace) {
            if budget.monthly_tokens != monthly_tokens {
                budget.notified_month = None;
                budget.near_notified_month = None;
            }
            budget.monthly_tokens = monthly_tokens;
        } else {
            self.budgets.push(Budget {
                workspace,
                monthly_tokens,
                notified_month: None,
                near_notified_month: None,
            });
        }
    }
    pub fn take_alert(&mut self, workspace: &Path, month: &str) -> Option<(u64, u64)> {
        let total = self.monthly(workspace, month).tokens();
        let budget = self.budgets.iter_mut().find(|b| b.workspace == workspace)?;
        if budget.monthly_tokens == 0 || budget.notified_month.as_deref() == Some(month) {
            return None;
        }
        if total >= budget.monthly_tokens {
            budget.notified_month = Some(month.to_owned());
            budget.near_notified_month = Some(month.to_owned());
        } else if total >= budget.monthly_tokens - budget.monthly_tokens / 5
            && budget.near_notified_month.as_deref() != Some(month)
        {
            budget.near_notified_month = Some(month.to_owned());
        } else {
            return None;
        }
        Some((total, budget.monthly_tokens))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observations_deduplicate_unknowns_stay_incomplete_and_alerts_are_month_scoped() {
        let workspace = PathBuf::from("C:/project");
        let timestamp = 1_783_000_000;
        let month = date(timestamp)[..7].to_owned();
        let mut ledger = Ledger::default();
        let record = Observation {
            key: "turn-1".into(),
            session: 1,
            workspace: workspace.clone(),
            provider: Provider::Codex,
            service: "cli".into(),
            service_name: None,
            model: None,
            finished_at: timestamp,
            input: Some(100),
            cached: Some(80),
            output: Some(50),
            elapsed_ms: 1_000,
            incomplete: false,
            api_first_response_ms: Some(250),
            api_request_outcome: Some(ApiRequestOutcome::Succeeded),
        };
        ledger.record(record.clone());
        ledger.record(record.clone());
        assert_eq!(ledger.monthly(&workspace, &month).tokens(), 150);
        ledger.save_budget(workspace.clone(), 120);
        assert_eq!(ledger.take_alert(&workspace, &month), Some((150, 120)));
        assert!(ledger.take_alert(&workspace, &month).is_none());
        let mut unknown = record;
        unknown.key = "turn-2".into();
        unknown.input = None;
        unknown.output = None;
        unknown.api_first_response_ms = None;
        unknown.api_request_outcome = None;
        let legacy = serde_json::to_string(&unknown).unwrap();
        assert!(!legacy.contains("api_first_response_ms"));
        assert!(!legacy.contains("api_request_outcome"));
        ledger.record(serde_json::from_str(&legacy).unwrap());
        let totals = ledger.monthly(&workspace, &month);
        assert_eq!(totals.turns, 2);
        assert_eq!(totals.incomplete, 1);
        assert_eq!(totals.tokens(), 150);
        assert_eq!(totals.average(), Some(150)); // Unknown turns do not dilute a measured average.
        assert_eq!(totals.complete_turns, 1);
        assert_eq!(totals.requests, 1);
        let restored: Ledger =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert_eq!(restored.monthly(&workspace, &month).incomplete, 1);
        assert_eq!(restored.observations[0].api_first_response_ms, Some(250));
        assert_eq!(
            restored.observations[0].api_request_outcome,
            Some(ApiRequestOutcome::Succeeded)
        );
        assert_eq!(restored.observations[1].api_first_response_ms, None);
        assert_eq!(restored.observations[1].api_request_outcome, None);
    }

    #[test]
    fn usage_reminders_fire_once_at_eighty_and_one_hundred_percent_and_survive_reload() {
        let workspace = PathBuf::from("C:/usage-alerts");
        let mut observation: Observation = serde_json::from_value(serde_json::json!({
            "key": "turn", "session": 1, "workspace": workspace, "provider": "Claude",
            "service": "cli", "model": null, "finished_at": 1783000000u64,
            "input": 79, "cached": null, "output": 0, "elapsed_ms": 0, "incomplete": false
        }))
        .unwrap();
        let month = date(observation.finished_at)[..7].to_owned();
        let mut ledger = Ledger::default();
        ledger.save_budget(workspace.clone(), 100);
        ledger.record(observation.clone());
        assert!(ledger.take_alert(&workspace, &month).is_none());
        observation.input = Some(80);
        ledger.record(observation.clone());
        assert_eq!(ledger.take_alert(&workspace, &month), Some((80, 100)));
        let mut restored: Ledger =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert!(restored.take_alert(&workspace, &month).is_none());
        observation.input = Some(100);
        restored.record(observation.clone());
        assert_eq!(restored.take_alert(&workspace, &month), Some((100, 100)));
        assert!(restored.take_alert(&workspace, &month).is_none());
        assert_eq!(restored.take_alert(&workspace, "2026-08"), None);
        restored.save_budget(workspace.clone(), 0);
        assert!(restored.take_alert(&workspace, &month).is_none());
        // Older profiles have no early-reminder field.
        let mut legacy: Budget = serde_json::from_value(serde_json::json!({
            "workspace": workspace, "monthly_tokens": 100, "notified_month": null
        }))
        .unwrap();
        assert!(legacy.near_notified_month.take().is_none());
        restored.save_budget(workspace.clone(), u64::MAX);
        observation.input = Some(u64::MAX - u64::MAX / 5);
        restored.record(observation);
        assert!(restored.take_alert(&workspace, &month).is_some());
    }
}
