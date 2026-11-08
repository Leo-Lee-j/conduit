//! Shared localization for the desktop application and update helper.
//! Catalogs are embedded; changing language never performs filesystem I/O.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::LazyLock};

#[derive(Clone, Debug, Deserialize)]
pub struct Language {
    pub id: String,
    pub name: String,
}

pub fn languages() -> &'static [Language] {
    static LANGUAGES: LazyLock<Vec<Language>> = LazyLock::new(|| {
        serde_json::from_str(include_str!("../assets/languages.json"))
            .expect("valid embedded language registry")
    });
    &LANGUAGES
}

/// Unknown identifiers fall back to English without rejecting user settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub struct Locale(String);

impl Default for Locale {
    fn default() -> Self {
        Self("en".into())
    }
}
impl Locale {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn legacy() -> Self {
        Self("zh-CN".into())
    }
}
impl From<String> for Locale {
    fn from(value: String) -> Self {
        languages()
            .iter()
            .find(|lang| lang.id.eq_ignore_ascii_case(&value))
            .map(|lang| Self(lang.id.clone()))
            .unwrap_or_default()
    }
}
impl From<Locale> for String {
    fn from(value: Locale) -> Self {
        value.0
    }
}

pub fn set_locale(locale: &Locale) {
    rust_i18n::set_locale(locale.as_str());
}

pub fn locale() -> Locale {
    rust_i18n::locale().to_string().into()
}

pub fn text(key: &str) -> String {
    rust_i18n::t!(key).into_owned()
}

pub fn text_for(locale: &Locale, key: &str) -> String {
    rust_i18n::t!(key, locale = locale.as_str()).into_owned()
}

/// Parameters are substituted once, so user values resembling a marker remain
/// literal and cannot cause recursive substitution.
pub fn format(key: &str, args: &[(&str, String)]) -> String {
    substitute(&text(key), args)
}
pub fn format_for(locale: &Locale, key: &str, args: &[(&str, String)]) -> String {
    substitute(&text_for(locale, key), args)
}
fn substitute(template: &str, args: &[(&str, String)]) -> String {
    let patterns: Vec<_> = args.iter().map(|(name, _)| *name).collect();
    let values: Vec<_> = args.iter().map(|(_, value)| value.clone()).collect();
    rust_i18n::replace_patterns(template, &patterns, &values)
}

/// The source-language lookup is a migration adapter for existing static UI
/// descriptors. Keys in the catalogs remain stable semantic identifiers. It
/// never runs on user conversation/code/terminal content.
pub fn source(value: &str) -> &str {
    CATALOGS
        .sources
        .get(value)
        .and_then(|key| {
            CATALOGS
                .values
                .get(&*rust_i18n::locale())
                .and_then(|values| values.get(key))
        })
        .map_or(value, String::as_str)
}

struct Catalogs {
    values: HashMap<String, HashMap<String, String>>,
    sources: HashMap<String, String>,
    aliases: HashMap<String, String>,
}
static CATALOGS: LazyLock<Catalogs> = LazyLock::new(|| {
    let sources: HashMap<String, String> =
        serde_json::from_str(include_str!("../assets/i18n-sources.json"))
            .expect("valid embedded source aliases");
    let keys: HashMap<String, serde_json::Value> =
        serde_json::from_str(include_str!("../locales/en.json"))
            .expect("valid embedded English catalog");
    let values: HashMap<String, HashMap<String, String>> = languages()
        .iter()
        .map(|language| {
            let values = keys
                .iter()
                .filter(|(_, value)| value.is_string())
                .map(|(key, _)| {
                    (
                        key.clone(),
                        rust_i18n::t!(key.as_str(), locale = language.id.as_str()).into_owned(),
                    )
                })
                .collect();
            (language.id.clone(), values)
        })
        .collect();
    let mut aliases = sources.clone();
    for language in languages() {
        let mut entries: Vec<_> = values[&language.id].iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        for (key, value) in entries {
            if !value.contains("%{") {
                aliases.entry(value.clone()).or_insert_with(|| key.clone());
            }
        }
    }
    Catalogs {
        values,
        sources,
        aliases,
    }
});

/// Borrowed translations avoid allocating for static labels on every frame.
pub fn text_static(key: &'static str) -> &'static str {
    CATALOGS
        .values
        .get(&*rust_i18n::locale())
        .or_else(|| CATALOGS.values.get("en"))
        .and_then(|values| values.get(key))
        .map_or(key, String::as_str)
}

pub fn source_for<'a>(locale: &Locale, value: &'a str) -> &'a str {
    CATALOGS
        .sources
        .get(value)
        .and_then(|key| {
            CATALOGS
                .values
                .get(locale.as_str())
                .and_then(|values| values.get(key))
        })
        .map_or(value, String::as_str)
}

/// Only for application-owned retained labels and historical application notices.
/// Never apply this to user text, model responses, filenames, or terminal output.
pub fn retained(value: &str) -> &str {
    CATALOGS
        .aliases
        .get(value)
        .and_then(|key| {
            CATALOGS
                .values
                .get(&*rust_i18n::locale())
                .and_then(|values| values.get(key))
        })
        .map_or(value, String::as_str)
}

/// Serializable presentation metadata. Arguments are data, never translated.
/// A fallback keeps records readable if a future catalog removes the key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    key: String,
    args: Vec<(String, String)>,
    fallback: String,
}
impl Message {
    pub fn new(key: &str, args: &[(&str, String)]) -> Self {
        Self {
            key: key.into(),
            args: args
                .iter()
                .map(|(key, value)| ((*key).into(), value.clone()))
                .collect(),
            fallback: format(key, args),
        }
    }
    pub fn capture_label(value: &str) -> Option<Self> {
        let key = CATALOGS.aliases.get(value)?;
        let template = CATALOGS.values.get("en")?.get(key)?;
        if template.contains("%{") {
            return None;
        }
        Some(Self {
            key: key.clone(),
            args: vec![],
            fallback: value.into(),
        })
    }
    pub fn render(&self) -> String {
        if !CATALOGS.values["en"].contains_key(&self.key) {
            return self.fallback.clone();
        }
        format(
            &self.key,
            &self
                .args
                .iter()
                .map(|(key, value)| (key.as_str(), value.clone()))
                .collect::<Vec<_>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_defaults_and_unknown_values_are_safe() {
        assert_eq!(Locale::default().as_str(), "en");
        assert_eq!(Locale::legacy().as_str(), "zh-CN");
        assert_eq!(Locale::from("ZH-cn".to_string()).as_str(), "zh-CN");
        assert_eq!(Locale::from("future-locale".to_string()).as_str(), "en");
        assert_eq!(
            text_for(&Locale::default(), "language.title"),
            "Application language"
        );
        assert_eq!(text_for(&Locale::legacy(), "language.title"), "应用语言");
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use std::collections::BTreeSet;

    fn parameters(text: &str) -> BTreeSet<String> {
        text.split("%{")
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_string()))
            .collect()
    }

    #[test]
    fn every_registered_language_has_matching_keys_and_parameters() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let baseline: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_str(include_str!("../locales/en.json")).unwrap();
        let mut ids = BTreeSet::new();
        for language in languages() {
            assert!(
                ids.insert(&language.id),
                "duplicate language {}",
                language.id
            );
            assert!(!language.name.trim().is_empty());
            let translated: std::collections::BTreeMap<String, serde_json::Value> =
                serde_json::from_str(
                    &std::fs::read_to_string(root.join(format!("{}.json", language.id))).unwrap(),
                )
                .unwrap();
            assert_eq!(
                baseline.keys().collect::<Vec<_>>(),
                translated.keys().collect::<Vec<_>>(),
                "{} keys",
                language.id
            );
            for (key, value) in &baseline {
                if let Some(value) = value.as_str() {
                    let actual = translated[key].as_str().unwrap();
                    assert!(!actual.is_empty(), "{}: {key}", language.id);
                    assert_eq!(
                        parameters(value),
                        parameters(actual),
                        "{}: {key}",
                        language.id
                    );
                }
            }
        }
        for key in CATALOGS.sources.values() {
            assert!(baseline.contains_key(key), "unknown alias target: {key}");
        }
    }

    #[test]
    fn interpolation_preserves_parameter_text_and_localized_history() {
        let _locale = crate::test_support::scoped("en");
        let message = Message::new(
            "chat.reasoning_effort",
            &[("effort", "%{effort} 中文 / {literal}".into())],
        );
        assert_eq!(
            message.render(),
            "Reasoning effort: %{effort} 中文 / {literal}"
        );
        let encoded = serde_json::to_string(&message).unwrap();
        set_locale(&Locale::legacy());
        let restored: Message = serde_json::from_str(&encoded).unwrap();
        assert_eq!(restored.render(), "思考强度：%{effort} 中文 / {literal}");
        assert_eq!(retained("Settings"), "设置");
        assert_eq!(retained("user owned code 🚀"), "user owned code 🚀");
        set_locale(&Locale::default());
        assert_eq!(source("设置"), "Settings");
        assert_eq!(text_static("missing.future.key"), "missing.future.key");
    }
}
