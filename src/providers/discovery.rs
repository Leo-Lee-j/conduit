//! Explicit, bounded model catalog request. Does not make a generation request.
use super::*;
use anyhow::ensure;

pub fn endpoint(base: &str) -> Result<url::Url> {
    let url = url::Url::parse(base).map_err(|_| {
        anyhow::anyhow!(conduit::i18n::text(
            "providers.discovery.base_url_is_not_a_valid_address"
        ))
    })?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !base.chars().any(char::is_whitespace)
            && !base.contains(['\\', '"']),
        conduit::i18n::text("providers.discovery.base_url_must_use_http_s_and")
    );
    Ok(url)
}

fn models_url(base: &str, engine: Provider) -> Result<url::Url> {
    let mut url = endpoint(base)?;
    let path = url.path().trim_end_matches('/');
    let path = if engine == Provider::Claude && !path.ends_with("/v1") {
        format!("{path}/v1/models")
    } else {
        format!("{path}/models")
    };
    url.set_path(&path);
    Ok(url)
}

fn model_ids(value: &Value) -> Result<Vec<String>> {
    let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
        anyhow::anyhow!(conduit::i18n::text(
            "providers.discovery.unsupported_model_catalog_format_you_can_enter"
        ))
    })?;
    let mut models = vec![];
    for item in data.iter().take(2000) {
        if let Some(id) = item["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
            && !models.iter().any(|m| m == id)
        {
            models.push(id.to_string());
        }
    }
    ensure!(
        !models.is_empty(),
        conduit::i18n::text("providers.discovery.service_returned_no_models_you_can_enter")
    );
    Ok(models)
}

pub fn fetch(engine: Provider, base: &str, key: Option<&str>) -> Result<Vec<String>> {
    let url = models_url(base, engine)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut request = agent.get(url.as_str());
    if engine == Provider::Claude {
        request = request.header("anthropic-version", "2023-06-01");
    }
    if let Some(key) = key.filter(|key| !key.is_empty()) {
        ensure!(
            !key.chars().any(char::is_control),
            conduit::i18n::text("providers.discovery.api_key_contains_invalid_characters")
        );
        request = if engine == Provider::Claude {
            request.header("x-api-key", key)
        } else {
            request.header("Authorization", format!("Bearer {key}"))
        };
    }
    let mut response = request.call().map_err(|error| match error {
        ureq::Error::StatusCode(code) => anyhow::anyhow!(conduit::i18n::format(
            "providers.discovery.model_catalog_request_failed_http_check_the",
            &[("code", format!("{}", code))]
        )),
        _ => anyhow::anyhow!(conduit::i18n::text(
            "providers.discovery.model_catalog_request_incomplete_check_network_certificates"
        )),
    })?;
    let value: Value = response
        .body_mut()
        .with_config()
        .limit(2 * 1024 * 1024)
        .read_json()
        .map_err(|_| {
            anyhow::anyhow!(conduit::i18n::text(
                "providers.discovery.model_catalog_is_invalid_or_exceeds_2"
            ))
        })?;
    model_ids(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_urls_and_untrusted_responses_are_bounded() {
        let _locale = crate::test_support::chinese();
        assert_eq!(
            models_url("https://example.com/v1/", Provider::Codex)
                .unwrap()
                .path(),
            "/v1/models"
        );
        assert_eq!(
            models_url("https://example.com/anthropic", Provider::Claude)
                .unwrap()
                .path(),
            "/anthropic/v1/models"
        );
        for bad in [
            "file:///tmp",
            "https://key@example.com",
            "https://example.com?key=abc",
            "https://example.com/#abc",
        ] {
            assert!(endpoint(bad).is_err());
        }
        assert_eq!(
            model_ids(&json!({"data":[{"id":"a"},{"id":"a"},{"id":"b\n"},{"id":"c"}]})).unwrap(),
            vec!["a", "c"]
        );
        assert!(model_ids(&json!({"error":"secret"})).is_err());
    }
}
