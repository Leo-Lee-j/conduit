//! Direct, bounded forge HTTP API access. Replaces `gh api` / `glab api`.
use super::*;
use auth::{Source, Tokens};
use forge::Forge;

const TIMEOUT: Duration = Duration::from_secs(45);
const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
    Get,
    Post,
    Put,
}

/// An HTTP status the server returned. 4xx means the request was rejected, not applied.
#[derive(Debug)]
pub(super) struct Status {
    pub code: u16,
    pub message: String,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}: {}", self.code, self.message)
    }
}

impl std::error::Error for Status {}

/// True when the server definitely rejected a request instead of possibly applying it.
pub(super) fn rejected(error: &anyhow::Error) -> bool {
    auth::required(error).is_some()
        || error.chain().any(|e| {
            e.downcast_ref::<Status>()
                .is_some_and(|s| (400..500).contains(&s.code))
        })
}

pub(super) fn rest_base(forge: Forge, host: &str) -> String {
    match forge {
        Forge::Github if host.eq_ignore_ascii_case("github.com") => "https://api.github.com".into(),
        Forge::Github => format!("https://{host}/api/v3"),
        Forge::Gitlab => format!("https://{host}/api/v4"),
    }
}

pub(super) fn graphql_url(host: &str) -> String {
    if host.eq_ignore_ascii_case("github.com") {
        "https://api.github.com/graphql".into()
    } else {
        format!("https://{host}/api/graphql")
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // Read error bodies ourselves; never follow redirects with an Authorization header.
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(concat!("Conduit/", env!("CARGO_PKG_VERSION")))
        // Self-hosted GitLab / GitHub Enterprise often use a corporate CA from the Windows store.
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

/// Classify `body` from an unauthenticated `GET /api/v4/version`. GitLab answers 401 with
/// a JSON `message` (or 200 with `version` when public); GitHub Enterprise has no v4 API.
pub(super) fn gitlab_version_response(code: u16, body: &str) -> bool {
    let value: Value = serde_json::from_str(body).unwrap_or_default();
    match code {
        401 => value["message"]
            .as_str()
            .is_some_and(|m| m.starts_with("401")),
        200 => value["version"].is_string(),
        _ => false,
    }
}

/// Probe whether `host` serves the GitLab API. Sends no credentials; definite answers are
/// cached for the process, network failures are not (and fall back to GitHub, as before).
pub(super) fn is_gitlab(host: &str) -> bool {
    use std::{collections::HashMap, sync::Mutex, sync::OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = host.to_ascii_lowercase();
    if let Some(known) = cache.lock().ok().and_then(|c| c.get(&key).copied()) {
        return known;
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent(concat!("Conduit/", env!("CARGO_PKG_VERSION")))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let Ok(mut response) = agent.get(&format!("https://{host}/api/v4/version")).call() else {
        return false;
    };
    let code = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .unwrap_or_default();
    let gitlab = gitlab_version_response(code, &body);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, gitlab);
    }
    gitlab
}

fn message(body: &str) -> String {
    let value: Value = serde_json::from_str(body).unwrap_or_default();
    let text = value["message"]
        .as_str()
        .or_else(|| value["error_description"].as_str())
        .or_else(|| value["error"].as_str())
        .map(str::to_owned)
        .or_else(|| (!value["message"].is_null()).then(|| value["message"].to_string()))
        .unwrap_or_else(|| body.chars().take(300).collect());
    text.chars().filter(|c| !c.is_control()).take(500).collect()
}

pub(super) struct Client<'a> {
    pub forge: Forge,
    pub host: String,
    cancel: &'a AtomicBool,
    agent: ureq::Agent,
    tokens: Tokens<'a>,
    token: Option<(String, Source)>,
    started: Instant,
    bytes: usize,
}

impl<'a> Client<'a> {
    pub fn new(forge: Forge, host: &str, workspace: &'a Path, cancel: &'a AtomicBool) -> Self {
        Self {
            forge,
            host: host.to_owned(),
            cancel,
            agent: agent(),
            tokens: Tokens::new(forge, host, workspace, cancel),
            token: None,
            started: Instant::now(),
            bytes: 0,
        }
    }

    fn token(&mut self) -> Result<String> {
        if self.token.is_none() {
            self.token = self.tokens.next()?;
        }
        match &self.token {
            Some((token, _)) => Ok(token.clone()),
            None => Err(self.tokens.required().into()),
        }
    }

    /// Resolve a credential now, so callers can tell a missing sign-in from a send failure.
    pub fn authenticate(&mut self) -> Result<()> {
        self.token().map(drop)
    }

    fn once(&mut self, method: Method, url: &str, body: Option<&Value>) -> Result<String> {
        if self.cancel.load(Ordering::Acquire) {
            bail!(conduit::i18n::text("github.cancelled"));
        }
        anyhow::ensure!(
            self.started.elapsed() < Duration::from_secs(90),
            "{}",
            conduit::i18n::text("github.timeout")
        );
        let authorization = format!("Bearer {}", self.token()?);
        let accept = match self.forge {
            Forge::Github => "application/vnd.github+json",
            Forge::Gitlab => "application/json",
        };
        let result = match (method, body) {
            (Method::Get, _) => self
                .agent
                .get(url)
                .header("Authorization", &authorization)
                .header("Accept", accept)
                .call(),
            (Method::Post, body) => self
                .agent
                .post(url)
                .header("Authorization", &authorization)
                .header("Accept", accept)
                .send_json(body.cloned().unwrap_or_else(|| serde_json::json!({}))),
            (Method::Put, body) => self
                .agent
                .put(url)
                .header("Authorization", &authorization)
                .header("Accept", accept)
                .send_json(body.cloned().unwrap_or_else(|| serde_json::json!({}))),
        };
        let mut response = result.map_err(|error| match error {
            ureq::Error::Timeout(_) => anyhow::anyhow!(conduit::i18n::text("github.timeout")),
            error => {
                anyhow::Error::new(error).context(format!("{} {}", self.forge.name(), self.host))
            }
        })?;
        let code = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|error| match error {
                ureq::Error::BodyExceedsLimit(_) => {
                    anyhow::anyhow!(conduit::i18n::text("github.output_limit"))
                }
                error => error.into(),
            })?;
        self.bytes = self.bytes.saturating_add(text.len());
        anyhow::ensure!(
            self.bytes <= 8 * 1024 * 1024,
            "{}",
            conduit::i18n::text("github.output_limit")
        );
        if !(200..300).contains(&code) {
            return Err(Status {
                code,
                message: message(&text),
            }
            .into());
        }
        Ok(text)
    }

    /// Send one request. After a 401 the next credential is tried, which is safe even for
    /// writes because the server rejected the request before acting on it.
    pub fn send(&mut self, method: Method, url: &str, body: Option<&Value>) -> Result<String> {
        loop {
            match self.once(method, url, body) {
                Err(error)
                    if error
                        .downcast_ref::<Status>()
                        .is_some_and(|status| status.code == 401) =>
                {
                    // The next `once` resolves the next candidate, or reports AuthRequired.
                    self.token = None;
                }
                result => return result,
            }
        }
    }

    /// GET a REST path relative to the forge's API root.
    pub fn get(&mut self, path: &str) -> Result<Value> {
        let url = format!("{}/{}", rest_base(self.forge, &self.host), path);
        Ok(serde_json::from_str(&self.send(
            Method::Get,
            &url,
            None,
        )?)?)
    }

    /// Run a read-only GraphQL query and return the raw response body.
    pub fn graphql(&mut self, query: &str, variables: Value) -> Result<String> {
        let url = graphql_url(&self.host);
        let body = serde_json::json!({ "query": query, "variables": variables });
        self.send(Method::Post, &url, Some(&body))
    }

    /// Like [`graphql`], but fails when the response reports errors or lacks data.
    pub fn graphql_data(&mut self, query: &str, variables: Value) -> Result<Value> {
        let value: Value = serde_json::from_str(&self.graphql(query, variables)?)?;
        if let Some(errors) = value.get("errors").and_then(Value::as_array)
            && !errors.is_empty()
        {
            bail!(
                "GraphQL: {}",
                errors
                    .iter()
                    .filter_map(|e| e["message"].as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        anyhow::ensure!(value["data"].is_object(), "Invalid GraphQL response");
        Ok(value["data"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_follow_cloud_and_self_hosted_layouts() {
        assert_eq!(
            rest_base(Forge::Github, "github.com"),
            "https://api.github.com"
        );
        assert_eq!(
            rest_base(Forge::Github, "ghe.example.test:8443"),
            "https://ghe.example.test:8443/api/v3"
        );
        assert_eq!(
            rest_base(Forge::Gitlab, "gitlab.example.test"),
            "https://gitlab.example.test/api/v4"
        );
        assert_eq!(graphql_url("GitHub.com"), "https://api.github.com/graphql");
        assert_eq!(
            graphql_url("ghe.example.test"),
            "https://ghe.example.test/api/graphql"
        );
    }

    #[test]
    fn gitlab_probe_recognizes_only_gitlab_version_responses() {
        assert!(gitlab_version_response(
            401,
            r#"{"message":"401 Unauthorized"}"#
        ));
        assert!(gitlab_version_response(
            200,
            r#"{"version":"17.5.0","revision":"x"}"#
        ));
        assert!(!gitlab_version_response(404, r#"{"message":"Not Found"}"#));
        assert!(!gitlab_version_response(
            401,
            r#"{"message":"Bad credentials"}"#
        ));
        assert!(!gitlab_version_response(200, "<html>"));
    }

    #[test]
    fn server_messages_are_bounded_and_rejections_are_certain() {
        assert_eq!(
            message(r#"{"message":"Bad credentials"}"#),
            "Bad credentials"
        );
        assert_eq!(
            message(r#"{"message":{"base":["x"]}}"#),
            r#"{"base":["x"]}"#
        );
        assert_eq!(message(r#"{"error":"invalid_token"}"#), "invalid_token");
        assert_eq!(message(&"\n".repeat(5)), "");
        let rejected_error = anyhow::Error::new(Status {
            code: 422,
            message: "x".into(),
        });
        assert!(rejected(&rejected_error));
        let server_error = anyhow::Error::new(Status {
            code: 502,
            message: "x".into(),
        });
        assert!(!rejected(&server_error));
    }
}
