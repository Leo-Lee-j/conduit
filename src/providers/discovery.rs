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

/// CLI clients append different resource paths: Claude adds `/v1/messages`,
/// while Codex adds `/responses`. Normalize once for runtime and discovery.
pub fn runtime_base_url(base: &str, engine: Provider) -> String {
    let base = base.trim_end_matches('/');
    match engine {
        Provider::Claude => base.strip_suffix("/v1").unwrap_or(base).to_owned(),
        Provider::Codex => {
            let path = url::Url::parse(base)
                .ok()
                .map(|url| url.path().to_owned())
                .unwrap_or_default();
            let last = path.rsplit('/').next().unwrap_or_default();
            let versioned = last
                .strip_prefix('v')
                .is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit()));
            if versioned {
                base.to_owned()
            } else {
                format!("{base}/v1")
            }
        }
        _ => base.to_owned(),
    }
}

fn models_url(base: &str, engine: Provider) -> Result<url::Url> {
    endpoint(base)?;
    let mut url = endpoint(&runtime_base_url(base, engine))?;
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
    fn optional_v1_is_consistent_for_discovery_and_runtime() {
        for prefix in ["", "/proxy", "/gateway/anthropic"] {
            for suffix in ["", "/", "/v1", "/v1/"] {
                let base = format!("https://example.com{prefix}{suffix}");
                for engine in [Provider::Claude, Provider::Codex] {
                    assert_eq!(
                        models_url(&base, engine).unwrap().path(),
                        format!("{prefix}/v1/models")
                    );
                    let route = RuntimeRoute::endpoint(engine, "test", &base, None);
                    if engine == Provider::Claude {
                        assert_eq!(
                            route.env["ANTHROPIC_BASE_URL"],
                            format!("https://example.com{prefix}")
                        );
                    } else {
                        assert_eq!(
                            route.config["model_providers"]["conduit"]["base_url"],
                            format!("https://example.com{prefix}/v1")
                        );
                    }
                }
            }
        }
        for version in ["v2", "v1beta", "v1beta1"] {
            let base = format!("https://example.com/gateway/{version}/");
            assert_eq!(
                models_url(&base, Provider::Codex).unwrap().path(),
                format!("/gateway/{version}/models")
            );
        }
    }

    #[test]
    fn catalog_fetch_sends_one_version_and_engine_specific_auth() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        for engine in [Provider::Claude, Provider::Codex] {
            for suffix in ["", "/v1/"] {
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let address = listener.local_addr().unwrap();
                let server = std::thread::spawn(move || {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        socket.read_exact(&mut byte).unwrap();
                        request.push(byte[0]);
                        assert!(request.len() < 8192);
                    }
                    let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
                    assert!(
                        request.starts_with("get /gateway/v1/models http/1.1"),
                        "{request}"
                    );
                    if engine == Provider::Claude {
                        assert!(request.contains("x-api-key: fixture"));
                        assert!(request.contains("anthropic-version: 2023-06-01"));
                    } else {
                        assert!(request.contains("authorization: bearer fixture"));
                    }
                    let body = r#"{"data":[{"id":"selected-model"}]}"#;
                    write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                });
                let result = fetch(
                    engine,
                    &format!("http://{address}/gateway{suffix}"),
                    Some("fixture"),
                );
                server.join().unwrap();
                assert_eq!(result.unwrap(), ["selected-model"]);
            }
        }
    }
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
