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
    pub model: Option<String>,
    pub finished_at: u64,
    pub input: Option<u64>,
    pub cached: Option<u64>,
    pub output: Option<u64>,
    pub elapsed_ms: u64,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Budget {
    pub workspace: PathBuf,
    pub monthly_tokens: u64,
    pub notified_month: Option<String>,
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
}
impl Totals {
    pub fn add(&mut self, observation: &Observation) {
        self.turns += 1;
        self.input = self.input.saturating_add(observation.input.unwrap_or(0));
        self.cached = self.cached.saturating_add(observation.cached.unwrap_or(0));
        self.output = self.output.saturating_add(observation.output.unwrap_or(0));
        self.incomplete += usize::from(
            observation.incomplete || observation.input.is_none() || observation.output.is_none(),
        );
        if observation.output.is_some() {
            self.elapsed_ms = self.elapsed_ms.saturating_add(observation.elapsed_ms);
        }
    }
    pub fn tokens(&self) -> u64 {
        self.input.saturating_add(self.output)
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
            }
            budget.monthly_tokens = monthly_tokens;
        } else {
            self.budgets.push(Budget {
                workspace,
                monthly_tokens,
                notified_month: None,
            });
        }
    }
    pub fn take_alert(&mut self, workspace: &Path, month: &str) -> Option<(u64, u64)> {
        let total = self.monthly(workspace, month).tokens();
        let budget = self.budgets.iter_mut().find(|b| b.workspace == workspace)?;
        if budget.monthly_tokens == 0
            || total < budget.monthly_tokens
            || budget.notified_month.as_deref() == Some(month)
        {
            return None;
        }
        budget.notified_month = Some(month.to_owned());
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
            model: None,
            finished_at: timestamp,
            input: Some(100),
            cached: Some(80),
            output: Some(50),
            elapsed_ms: 1_000,
            incomplete: false,
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
        ledger.record(unknown);
        let totals = ledger.monthly(&workspace, &month);
        assert_eq!(totals.turns, 2);
        assert_eq!(totals.incomplete, 1);
        assert_eq!(totals.tokens(), 150);
        let restored: Ledger =
            serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
        assert_eq!(restored.monthly(&workspace, &month).incomplete, 1);
    }
}
