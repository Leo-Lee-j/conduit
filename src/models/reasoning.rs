//! Model-specific reasoning controls. The registry is a fallback, never a replacement
//! for an engine's published catalog or a translation between incompatible APIs.
use crate::{domain::Provider, providers::ReasoningEffort};
use serde::Deserialize;
use serde_json::Value;
use std::sync::LazyLock;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub levels: Vec<ReasoningEffort>,
    pub default: Option<ReasoningEffort>,
}

impl Capabilities {
    pub fn choices(&self) -> Vec<ReasoningEffort> {
        std::iter::once(ReasoningEffort::Default)
            .chain(self.levels.iter().copied())
            .collect()
    }

    pub fn constrain(&self, effort: ReasoningEffort) -> ReasoningEffort {
        if self.levels.contains(&effort) {
            effort
        } else {
            ReasoningEffort::Default
        }
    }

    /// Both Codex app-server model/list and its on-disk model catalog are supported.
    /// An empty published list is authoritative; absent metadata permits fallback.
    pub fn from_codex(value: &Value) -> Option<Self> {
        let entries = value
            .get("supportedReasoningEfforts")
            .or_else(|| value.get("supported_reasoning_levels"))?
            .as_array()?;
        let mut levels = vec![];
        for entry in entries {
            let raw = entry
                .get("reasoningEffort")
                .or_else(|| entry.get("effort"))
                .unwrap_or(entry);
            if let Some(level) = raw.as_str().and_then(ReasoningEffort::from_wire)
                && level != ReasoningEffort::Default
                && !levels.contains(&level)
            {
                levels.push(level);
            }
        }
        let default = value
            .get("defaultReasoningEffort")
            .or_else(|| value.get("default_reasoning_level"))
            .and_then(Value::as_str)
            .and_then(ReasoningEffort::from_wire)
            .filter(|value| levels.contains(value));
        Some(Self { levels, default })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    schema_version: u32,
    models: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    ids: Vec<String>,
    control: String,
    capabilities: Capabilities,
    /// Only engines whose current adapter can transmit this control unchanged.
    engines: Vec<Provider>,
    verified_at: String,
    sources: Vec<String>,
    notes: String,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    let registry: Registry =
        serde_json::from_str(include_str!("../../assets/model-reasoning.json"))
            .expect("invalid bundled reasoning registry");
    assert_eq!(registry.schema_version, 1);
    for entry in &registry.models {
        assert!(!entry.control.is_empty() && !entry.notes.is_empty());
        assert_eq!(entry.verified_at.len(), 10);
        assert!(!entry.sources.is_empty());
    }
    registry
});

fn normalized_id(id: &str) -> String {
    let id = crate::providers::claude::split_one_m(id).0;
    id.rsplit('/').next().unwrap_or(id).to_ascii_lowercase()
}

/// Exact IDs and dated snapshots only: never let `gpt-5` match `gpt-5-pro`,
/// or infer capabilities for a new version from an older model family.
fn matches_id(id: &str, known: &str) -> bool {
    if id == known {
        return true;
    }
    id.strip_prefix(known)
        .and_then(|suffix| suffix.strip_prefix('-'))
        .is_some_and(|date| {
            (date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()))
                || (date.len() == 10
                    && date.bytes().enumerate().all(|(i, b)| {
                        if i == 4 || i == 7 {
                            b == b'-'
                        } else {
                            b.is_ascii_digit()
                        }
                    }))
        })
}

pub fn resolve(
    engine: Provider,
    model: Option<&str>,
    published: Option<&Capabilities>,
) -> Capabilities {
    if let Some(published) = published {
        return published.clone();
    }
    let Some(model) = model else {
        return Capabilities::default();
    };
    let id = normalized_id(model);
    REGISTRY
        .models
        .iter()
        .find(|entry| {
            entry.engines.contains(&engine) && entry.ids.iter().any(|known| matches_id(&id, known))
        })
        .map(|entry| entry.capabilities.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ReasoningEffort::{Default, High, Low, Max, Medium, Ultra, Xhigh};

    #[test]
    fn registry_is_reviewable_and_does_not_invent_cross_protocol_levels() {
        let mut controls = std::collections::HashSet::new();
        let mut engine_ids = std::collections::HashSet::new();
        for entry in &REGISTRY.models {
            assert!(!entry.ids.is_empty());
            assert!(!entry.control.is_empty() && !entry.notes.is_empty());
            assert_eq!(entry.verified_at.len(), 10);
            assert!(
                !entry.sources.is_empty()
                    && entry.sources.iter().all(|s| s.starts_with("https://"))
            );
            for id in &entry.ids {
                assert!(
                    controls.insert((id, &entry.control)),
                    "duplicate model/control: {id} / {}",
                    entry.control
                );
                for engine in &entry.engines {
                    assert!(
                        engine_ids.insert((id, engine)),
                        "ambiguous model/engine: {id} / {engine:?}"
                    );
                }
            }
            let mut levels = vec![];
            for level in &entry.capabilities.levels {
                assert_ne!(*level, Default);
                assert!(!levels.contains(level));
                levels.push(*level);
            }
            if let Some(default) = entry.capabilities.default {
                assert!(levels.contains(&default));
            }
        }
        assert_eq!(
            resolve(
                Provider::Claude,
                Some("anthropic/claude-sonnet-4-6[1M]"),
                None
            )
            .levels,
            [Low, Medium, High, Max]
        );
        assert_eq!(
            resolve(Provider::Claude, Some("claude-opus-4-7"), None).levels,
            [Low, Medium, High, Xhigh, Max]
        );
        assert!(
            resolve(Provider::Codex, Some("claude-opus-4-7"), None)
                .levels
                .is_empty()
        );
        assert!(
            resolve(Provider::Gemini, Some("gemini-4-neon"), None)
                .levels
                .is_empty()
        );
        assert_eq!(
            resolve(Provider::Claude, Some("deepseek-flash"), None).levels,
            [Low, High, Max]
        );
        assert_eq!(
            resolve(Provider::Codex, Some("deepseek-flash"), None).levels,
            [ReasoningEffort::None, Low, High, Max]
        );
        for id in ["gpt-5-pro", "gpt-5-custom", "gpt-99", "sonnet", "my-gpt-5"] {
            assert!(
                resolve(Provider::Codex, Some(id), None).levels.is_empty(),
                "{id}"
            );
        }
        assert_eq!(
            resolve(Provider::Codex, Some("router/gpt-5-2025-08-07"), None).levels,
            resolve(Provider::Codex, Some("gpt-5"), None).levels
        );
    }

    #[test]
    fn published_catalog_wins_including_empty_lists_and_native_max_ultra() {
        let native = Capabilities::from_codex(&serde_json::json!({
            "supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"max"},{"reasoningEffort":"ultra"}],
            "defaultReasoningEffort":"max"
        })).unwrap();
        assert_eq!(native.levels, [Low, Max, Ultra]);
        assert_eq!(native.default, Some(Max));
        assert_eq!(
            resolve(Provider::Codex, Some("gpt-5"), Some(&native)),
            native
        );
        assert_eq!(native.constrain(High), Default);
        assert_eq!(native.constrain(Ultra), Ultra);
        let empty = Capabilities::from_codex(&serde_json::json!({"supported_reasoning_levels":[]}))
            .unwrap();
        assert_eq!(
            resolve(Provider::Codex, Some("gpt-5"), Some(&empty)).choices(),
            [Default]
        );
        assert!(Capabilities::from_codex(&serde_json::json!({})).is_none());
    }

    #[test]
    fn all_wire_levels_roundtrip_without_confusing_none_and_follow_default() {
        for level in ReasoningEffort::ALL {
            let value = serde_json::to_value(level).unwrap();
            assert_eq!(
                serde_json::from_value::<ReasoningEffort>(value).unwrap(),
                level
            );
            if let Some(wire) = level.wire() {
                assert_eq!(ReasoningEffort::from_wire(wire), Some(level));
            }
        }
        assert_eq!(Default.wire(), None);
        assert_eq!(ReasoningEffort::None.wire(), Some("none"));
    }
}
