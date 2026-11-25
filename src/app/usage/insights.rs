//! Comparisons use equal calendar ranges; projections describe observed usage only.
use super::report::{Report, service_key};
use crate::{
    domain::Provider,
    usage_ledger::{Ledger, Totals},
};
use chrono::{Datelike, Days, Months, NaiveDate};
use std::path::Path;

#[derive(Clone)]
pub(super) struct Day {
    pub date: NaiveDate,
    pub tokens: u64,
    pub turns: usize,
    pub incomplete: usize,
    pub observed: bool,
}

pub(super) struct Forecast {
    pub tokens: u64,
    pub daily_average: u64,
}

pub(super) struct Insights {
    pub days: Vec<Day>,
    pub first_seen: Option<NaiveDate>,
    pub elapsed_days: u32,
    pub comparison_days: u32,
    pub previous: Totals,
    pub comparable_current: Totals,
    pub comparable: bool,
    pub forecast: Option<Forecast>,
}

fn record_date(timestamp: u64) -> Option<NaiveDate> {
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|date| date.with_timezone(&chrono::Local).date_naive())
}

fn matches_service(provider: Provider, service: &str, selected: Option<&str>) -> bool {
    selected.is_none_or(|selected| service_key(provider, service) == selected)
}

impl Insights {
    pub fn build(
        ledger: &Ledger,
        workspace: &Path,
        first: NaiveDate,
        today: NaiveDate,
        selected: Option<&str>,
        report: &Report,
        engine: Option<Provider>,
    ) -> Self {
        let end = first.checked_add_months(Months::new(1)).unwrap() - Days::new(1);
        let elapsed_days = if first > today {
            0
        } else {
            today.min(end).day()
        };
        let previous_first = first.checked_sub_months(Months::new(1)).unwrap();
        let previous_days = (first - Days::new(1)).day();
        let comparison_days = elapsed_days.min(previous_days);
        let mut result = Self {
            days: Vec::new(),
            first_seen: None,
            elapsed_days,
            comparison_days,
            previous: Totals::default(),
            comparable_current: Totals::default(),
            comparable: false,
            forecast: None,
        };
        let window_end = today - Days::new(1);
        let window_start = today - Days::new(7);
        let mut recent = Totals::default();
        for record in ledger.observations.iter().filter(|record| {
            record.workspace == workspace
                && matches_service(record.provider, &record.service, selected)
                && engine.is_none_or(|engine| record.provider == engine)
        }) {
            let Some(date) = record_date(record.finished_at).filter(|date| *date <= today) else {
                continue;
            };
            result.first_seen = Some(result.first_seen.map_or(date, |first| first.min(date)));
            if date >= previous_first && date < first && date.day() <= comparison_days {
                result.previous.add(record);
            }
            if date >= first && date <= end && date.day() <= comparison_days {
                result.comparable_current.add(record);
            }
            if date >= window_start && date <= window_end {
                recent.add(record);
            }
        }
        result.comparable = comparison_days > 0
            && !ledger.truncated
            && result.first_seen.is_some_and(|date| date <= previous_first)
            && result.previous.turns > 0
            && result.previous.incomplete == 0
            && result.comparable_current.incomplete == 0;
        result.days = first
            .iter_days()
            .take(elapsed_days as usize)
            .map(|date| {
                let totals = report.daily.get(&date.format("%Y-%m-%d").to_string());
                Day {
                    date,
                    tokens: totals.map_or(0, Totals::tokens),
                    turns: totals.map_or(0, |totals| totals.turns),
                    incomplete: totals.map_or(0, |totals| totals.incomplete),
                    observed: result.first_seen.is_some_and(|first| first <= date),
                }
            })
            .collect();
        if today >= first
            && today <= end
            && !ledger.truncated
            && report.filtered.turns > 0
            && report.filtered.incomplete == 0
            && recent.incomplete == 0
            && result.first_seen.is_some_and(|date| date <= window_start)
        {
            let remaining = u128::from(end.day() - today.day());
            result.forecast = Some(Forecast {
                tokens: (u128::from(report.filtered.tokens())
                    + u128::from(recent.tokens()) * remaining / 7)
                    .min(u128::from(u64::MAX)) as u64,
                daily_average: recent.tokens() / 7,
            });
        }
        result
    }

    pub fn change(&self) -> Option<f64> {
        (self.comparable && self.previous.tokens() > 0).then(|| {
            (self.comparable_current.tokens() as f64 / self.previous.tokens() as f64 - 1.) * 100.
        })
    }

    pub fn peak(&self) -> Option<&Day> {
        self.days
            .iter()
            .filter(|day| day.tokens > 0)
            .max_by_key(|day| day.tokens)
    }
}

pub(super) fn high_average(total: &Totals, scope: &Totals) -> bool {
    total.complete_turns >= 3
        && scope.complete_turns >= 5
        && total
            .average()
            .zip(scope.average())
            .is_some_and(|(average, baseline)| {
                baseline > 0 && u128::from(average) >= u128::from(baseline) * 2
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_ledger::{ApiRequestOutcome, Observation};
    use chrono::{Local, TimeZone};

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }
    fn record(date: NaiveDate, tokens: u64) -> Observation {
        Observation {
            key: date.to_string(),
            session: 1,
            workspace: "C:/usage".into(),
            provider: Provider::Claude,
            service: "cli".into(),
            service_name: None,
            model: Some("model-a".into()),
            finished_at: Local
                .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
                .unwrap()
                .timestamp() as u64,
            input: Some(tokens),
            cached: None,
            cache_write: None,
            output: Some(0),
            elapsed_ms: 1,
            incomplete: false,
            api_first_response_ms: None,
            api_request_outcome: Some(ApiRequestOutcome::Succeeded),
        }
    }
    fn build(ledger: &Ledger, first: NaiveDate, today: NaiveDate) -> Insights {
        let report = Report::build(
            &ledger.observations,
            Path::new("C:/usage"),
            &first.format("%Y-%m").to_string(),
            &[],
            None,
            None,
        );
        Insights::build(
            ledger,
            Path::new("C:/usage"),
            first,
            today,
            None,
            &report,
            None,
        )
    }

    #[test]
    fn comparison_aligns_short_months_and_rejects_missing_or_partial_baselines() {
        let mut ledger = Ledger::default();
        for (date, tokens) in [
            (day(2, 1), 100),
            (day(2, 28), 100),
            (day(3, 1), 200),
            (day(3, 28), 200),
            (day(3, 31), 900),
        ] {
            ledger.record(record(date, tokens));
        }
        let insights = build(&ledger, day(3, 1), day(4, 1));
        assert_eq!(insights.comparison_days, 28);
        assert_eq!(insights.previous.tokens(), 200);
        assert_eq!(insights.comparable_current.tokens(), 400);
        assert_eq!(insights.change(), Some(100.));
        ledger.observations[0].input = None;
        assert!(!build(&ledger, day(3, 1), day(4, 1)).comparable);
        ledger.observations.remove(0);
        assert!(!build(&ledger, day(3, 1), day(4, 1)).comparable);
    }

    #[test]
    fn projection_uses_seven_complete_days_and_never_zero_fills_future_dates() {
        let mut ledger = Ledger::default();
        for date in day(11, 16).iter_days().take(8) {
            ledger.record(record(date, 70));
        }
        let insights = build(&ledger, day(11, 1), day(11, 23));
        assert_eq!(insights.days.len(), 23);
        assert!(!insights.days[0].observed);
        let forecast = insights.forecast.unwrap();
        assert_eq!(forecast.daily_average, 70);
        assert_eq!(forecast.tokens, 1_050); // 560 observed + seven remaining days at 70.
        assert!(build(&ledger, day(11, 1), day(11, 22)).forecast.is_none());
        ledger.observations[0].output = None;
        assert!(build(&ledger, day(11, 1), day(11, 23)).forecast.is_none());
        ledger.observations[0].output = Some(0);
        ledger.truncated = true;
        assert!(build(&ledger, day(11, 1), day(11, 23)).forecast.is_none());
        assert!(build(&ledger, day(12, 1), day(11, 23)).days.is_empty());
    }
}
