//! Aggregation keys are stable identities, never editable display names.
use crate::{
    domain::Provider,
    providers::ModelProvider,
    usage_ledger::{self, Observation, Totals},
};
use std::{collections::BTreeMap, path::Path};

pub(super) fn service_key(engine: Provider, service: &str) -> String {
    let id = if let Some(id) = service.strip_prefix("provider:") {
        format!("provider:{id}")
    } else if let Some(id) = service.strip_prefix("ccs:") {
        format!("provider:{}", ModelProvider::ccs_id(engine, id))
    } else {
        service.to_owned()
    };
    format!("{}:{id}", engine.descriptor().id)
}

pub(super) fn service_name(
    engine: Provider,
    service: &str,
    providers: &[ModelProvider],
) -> Option<String> {
    let key = service_key(engine, service);
    providers
        .iter()
        .find(|p| p.engine == engine && service_key(engine, &format!("provider:{}", p.id)) == key)
        .map(|p| p.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

pub(super) struct Service {
    pub key: String,
    pub name: Option<String>,
    pub engine: Provider,
    pub cli: bool,
    pub totals: Totals,
}

#[derive(Default)]
pub(super) struct Report {
    pub totals: Totals,
    pub filtered: Totals,
    pub services: Vec<Service>,
    pub models: Vec<(Option<String>, Totals)>,
    pub sessions: Vec<(u64, Totals)>,
    pub daily: BTreeMap<String, Totals>,
    /// Indices into the source ledger, newest first; avoids copying observations per frame.
    pub records: Vec<usize>,
    cache_input: u128,
    cache_hits: u128,
}

impl Report {
    pub fn build(
        observations: &[Observation],
        workspace: &Path,
        month: &str,
        providers: &[ModelProvider],
        selected: Option<&str>,
        engine: Option<Provider>,
    ) -> Self {
        let mut report = Self::default();
        let mut services = BTreeMap::<String, Service>::new();
        let mut models = BTreeMap::<Option<String>, Totals>::new();
        let mut sessions = BTreeMap::<u64, Totals>::new();
        for (index, row) in observations.iter().enumerate().filter(|(_, o)| {
            o.workspace == workspace && usage_ledger::date(o.finished_at).starts_with(month)
        }) {
            report.totals.add(row);
            if engine.is_some_and(|engine| engine != row.provider) {
                continue;
            }
            let key = service_key(row.provider, &row.service);
            let service = services.entry(key.clone()).or_insert_with(|| Service {
                key: key.clone(),
                name: row
                    .service_name
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| service_name(row.provider, &row.service, providers)),
                engine: row.provider,
                cli: row.service == "cli",
                totals: Totals::default(),
            });
            if service.name.is_none() {
                service.name = row.service_name.clone().filter(|s| !s.trim().is_empty());
            }
            service.totals.add(row);
            if selected.is_some_and(|selected| selected != key) {
                continue;
            }
            report.filtered.add(row);
            report.records.push(index);
            if let (Some(input), Some(cached)) = (row.input, row.cached)
                && cached <= input
            {
                report.cache_input += u128::from(input);
                report.cache_hits += u128::from(cached);
            }
            let model = row
                .model
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            models.entry(model).or_default().add(row);
            sessions.entry(row.session).or_default().add(row);
            report
                .daily
                .entry(usage_ledger::date(row.finished_at))
                .or_default()
                .add(row);
        }
        report.services = services.into_values().collect();
        report
            .services
            .sort_by_key(|s| std::cmp::Reverse(s.totals.tokens()));
        report.models = models.into_iter().collect();
        report
            .models
            .sort_by_key(|(_, t)| std::cmp::Reverse(t.tokens()));
        report.sessions = sessions.into_iter().collect();
        report
            .records
            .sort_by_key(|index| std::cmp::Reverse((observations[*index].finished_at, *index)));
        report
            .sessions
            .sort_by_key(|(_, t)| std::cmp::Reverse(t.tokens()));
        report
    }

    pub fn cache_hit_rate(&self) -> Option<f64> {
        (self.cache_input > 0).then(|| self.cache_hits as f64 * 100. / self.cache_input as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_breakdown_preserves_identity_models_unknowns_and_filter_scope() {
        let root = Path::new("C:/usage");
        let record = Observation {
            key: "1".into(),
            session: 1,
            workspace: root.into(),
            provider: Provider::Codex,
            service: "ccs:one".into(),
            service_name: Some("Gateway".into()),
            model: Some("model-a".into()),
            finished_at: 1_783_000_000,
            input: Some(100),
            cached: Some(80),
            cache_write: None,
            output: Some(50),
            elapsed_ms: 1000,
            incomplete: false,
            api_first_response_ms: None,
            api_request_outcome: None,
        };
        let month = usage_ledger::date(record.finished_at)[..7].to_owned();
        let mut modern = record.clone();
        modern.service = "provider:ccs:codex:one".into();
        modern.model = Some("model-b".into());
        modern.session = 2;
        let mut other = record.clone();
        other.service = "provider:other".into();
        other.input = None;
        other.output = None;
        other.model = None;
        let mut outside = record.clone();
        outside.workspace = "C:/other".into();
        let records = [record, modern, other, outside];
        let report = Report::build(&records, root, &month, &[], None, None);
        assert_eq!(report.totals.tokens(), 300); // Cached input is not counted twice.
        assert_eq!(report.services.len(), 2); // Same names do not merge different identities.
        assert_eq!(report.services[0].totals.turns, 2); // Legacy/new CCS identities merge.
        assert_eq!(report.models.len(), 3);
        assert_eq!(report.totals.incomplete, 1);
        let selected = &report.services[0].key;
        let filtered = Report::build(&records, root, &month, &[], Some(selected), None);
        assert_eq!(filtered.totals.turns, 3);
        assert_eq!(filtered.filtered.turns, 2);
        assert_eq!(filtered.models.len(), 2);
        assert_eq!(filtered.sessions.len(), 2);
        assert_eq!(
            filtered.daily.values().map(Totals::tokens).sum::<u64>(),
            300
        );
        assert_eq!(filtered.services[0].name.as_deref(), Some("Gateway")); // Saved names survive removal.
        assert!(service_name(Provider::Codex, "cli", &[]).is_none());
        assert_ne!(
            service_key(Provider::Codex, "cli"),
            service_key(Provider::Codex, "provider:cli")
        );
    }

    #[test]
    fn engine_filters_share_the_ledger_scope_and_cache_uses_paired_counters() {
        let root = Path::new("C:/usage");
        let record = Observation {
            key: "first".into(),
            session: 1,
            workspace: root.into(),
            provider: Provider::Claude,
            service: "cli".into(),
            service_name: None,
            model: Some("model-a".into()),
            finished_at: 1_783_000_000,
            input: Some(100),
            cached: Some(80),
            cache_write: None,
            output: Some(10),
            elapsed_ms: 1000,
            incomplete: false,
            api_first_response_ms: None,
            api_request_outcome: None,
        };
        let month = usage_ledger::date(record.finished_at)[..7].to_owned();
        let mut records = vec![record; 5];
        records[1].cached = None;
        records[1].finished_at += 30;
        records[2].cached = Some(150);
        records[2].finished_at += 10;
        records[3].provider = Provider::Codex;
        records[3].service = "provider:other".into();
        records[3].finished_at += 40;
        records[4].input = Some(50);
        records[4].cached = Some(0);
        records[4].finished_at += 20;
        let report = Report::build(&records, root, &month, &[], None, Some(Provider::Claude));
        assert_eq!(report.totals.turns, 5); // Reminder scope stays workspace-wide.
        assert_eq!(report.filtered.turns, 4);
        assert_eq!(report.services.len(), 1);
        assert_eq!(report.records, vec![1, 4, 2, 0]);
        assert!((report.cache_hit_rate().unwrap() - 80. / 150. * 100.).abs() < 0.001);
        assert_eq!(
            report
                .daily
                .values()
                .map(|total| total.turns)
                .sum::<usize>(),
            4
        );
        let incompatible = Report::build(
            &records,
            root,
            &month,
            &[],
            Some(&service_key(Provider::Codex, "provider:other")),
            Some(Provider::Claude),
        );
        assert!(incompatible.records.is_empty());
        assert!(incompatible.cache_hit_rate().is_none());
        let unknown = Report::build(&records[1..2], root, &month, &[], None, None);
        assert!(unknown.cache_hit_rate().is_none());
    }

    #[test]
    fn usage_provider_names_resolve_legacy_routes_without_guessing_cli() {
        let provider: ModelProvider = serde_json::from_value(serde_json::json!({
            "id": "ccs:codex:one", "name": "My gateway", "engine": "Codex",
            "base_url": "https://example.invalid/v1", "api_key_env": "", "models": []
        }))
        .unwrap();
        let providers = [provider];
        for route in ["ccs:one", "provider:ccs:codex:one"] {
            assert_eq!(
                service_name(Provider::Codex, route, &providers).as_deref(),
                Some("My gateway")
            );
        }
        assert!(service_name(Provider::Claude, "ccs:one", &providers).is_none());
        assert!(service_name(Provider::Codex, "cli", &providers).is_none());
        let legacy: Observation = serde_json::from_value(serde_json::json!({
            "key":"legacy", "session":1, "workspace":"C:/usage", "provider":"Codex", "service":"cli",
            "model":null, "finished_at":1783000000u64, "input":100, "cached":null, "output":50,
            "elapsed_ms":1000, "incomplete":false
        })).unwrap();
        assert!(legacy.service_name.is_none());
        let mut saved = legacy;
        saved.service_name = Some("Deleted gateway".into());
        let restored: Observation =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.service_name.as_deref(), Some("Deleted gateway"));
    }
}
