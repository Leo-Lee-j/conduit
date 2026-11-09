//! Interpret provider diagnostics for display; persisted text remains verbatim.
use serde_json::Value;

struct RetryPrefix {
    end: usize,
    attempt: Option<(u16, u16)>,
}

fn retry_prefix(message: &str) -> Option<RetryPrefix> {
    let lower = message.to_ascii_lowercase();
    let mut rest = lower.trim_start();
    for engine in [
        "codex cli",
        "codex",
        "claude code",
        "claude",
        "opencode",
        "qoder",
    ] {
        if let Some(tail) = rest.strip_prefix(engine)
            && tail.starts_with([' ', ':'])
        {
            rest = tail.trim_start_matches([' ', ':']);
            break;
        }
    }
    rest = rest
        .strip_prefix("reconnecting")
        .or_else(|| rest.strip_prefix("retrying"))?;
    if !rest.is_empty()
        && !rest.starts_with(|c: char| c.is_whitespace() || matches!(c, '.' | '…' | ':'))
    {
        return None;
    }
    rest = rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '.' | '…'));
    let mut attempt = None;
    if let Some((current, tail)) = rest.split_once('/')
        && let Ok(current) = current.trim().parse::<u16>()
    {
        let tail = tail.trim_start();
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        if let Ok(total) = tail[..digits].parse::<u16>() {
            rest = &tail[digits..];
            if current > 0 && current <= total {
                attempt = Some((current, total));
            }
        }
    }
    Some(RetryPrefix {
        end: lower.len() - rest.len(),
        attempt,
    })
}

fn payload(value: &Value) -> &Value {
    value
        .get("error")
        .filter(|error| !error.is_null())
        .unwrap_or(value)
}

/// Extract a useful message without replacing its cause with an internal enum.
pub(super) fn message(raw: &str) -> String {
    let raw = raw.trim().strip_prefix("执行失败：").unwrap_or(raw.trim());
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return raw.into();
    };
    let error = payload(&value);
    error
        .as_str()
        .or_else(|| error.get("message").and_then(Value::as_str))
        .or_else(|| error.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| error.pointer("/errors/0").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| error.to_string())
}

pub(super) fn fingerprint(raw: &str) -> String {
    let message = message(raw);
    let cause = if let Some(prefix) = retry_prefix(&message) {
        let cause = message[prefix.end..].trim_start_matches([' ', ':', '-']);
        if cause.is_empty() {
            "connection retry".into()
        } else {
            cause.into()
        }
    } else {
        message
    };
    // The same generic reconnect message can refer to different HTTP failures.
    match status_code(raw) {
        Some(code) => format!("{cause} (HTTP {code})"),
        None => cause,
    }
}

fn status_code(raw: &str) -> Option<u16> {
    fn visit(value: &Value) -> Option<u16> {
        let object = value.as_object()?;
        for key in ["httpStatusCode", "status_code", "status", "code"] {
            if let Some(code) = object.get(key).and_then(Value::as_u64)
                && (400..600).contains(&code)
            {
                return Some(code as u16);
            }
        }
        object.values().find_map(visit)
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .as_ref()
        .and_then(visit)
        .or_else(|| {
            let text = message(raw).to_ascii_lowercase();
            let words = text
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>();
            words.iter().enumerate().find_map(|(index, word)| {
                let code = word.parse::<u16>().ok()?;
                ((400..600).contains(&code)
                    && (index == 0
                        || matches!(words[index - 1], "http" | "status" | "code" | "error")))
                .then_some(code)
            })
        })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cause {
    Credentials,
    Permission,
    Quota,
    RateLimit,
    Context,
    Model,
    Timeout,
    Connection,
    Service,
    Unknown,
}

impl Cause {
    pub(super) fn detect(raw: &str) -> Self {
        let text = raw.to_ascii_lowercase();
        let code = status_code(raw);
        if code == Some(401)
            || text.contains("invalid_api_key")
            || text.contains("invalid api key")
            || text.contains("unauthorized")
            || text.contains("authentication")
            || text.contains("token expired")
        {
            Self::Credentials
        } else if code == Some(403)
            || text.contains("forbidden")
            || text.contains("permission denied")
        {
            Self::Permission
        } else if text.contains("insufficient_quota")
            || text.contains("quota")
            || text.contains("insufficient balance")
            || text.contains("credit balance")
            || text.contains("billing")
        {
            Self::Quota
        } else if code == Some(429)
            || text.contains("rate limit")
            || text.contains("rate_limit")
            || text.contains("too many requests")
        {
            Self::RateLimit
        } else if text.contains("context")
            && ["length", "window", "limit", "exceeded"]
                .iter()
                .any(|s| text.contains(s))
        {
            Self::Context
        } else if (text.contains("model") || text.contains("provider"))
            && [
                "not found",
                "not available",
                "unavailable",
                "does not exist",
                "notfound",
                "unsupported",
            ]
            .iter()
            .any(|s| text.contains(s))
        {
            Self::Model
        } else if matches!(code, Some(408 | 504))
            || text.contains("timeout")
            || text.contains("timed out")
        {
            Self::Timeout
        } else if code.is_some_and(|code| code >= 500)
            || [
                "internalservererror",
                "internal server error",
                "server_error",
                "service unavailable",
                "overloaded",
                "high demand",
            ]
            .iter()
            .any(|s| text.contains(s))
        {
            Self::Service
        } else if [
            "disconnect",
            "connection",
            "network",
            "broken pipe",
            "socket hang up",
            "dns",
            "reconnecting",
        ]
        .iter()
        .any(|s| text.contains(s))
        {
            Self::Connection
        } else {
            Self::Unknown
        }
    }

    pub(super) fn summary(self, raw: &str) -> Option<String> {
        let key = match self {
            Self::Credentials => "failure.credentials_invalid",
            Self::Permission => "failure.access_denied",
            Self::Quota => "failure.quota_exhausted",
            Self::RateLimit => "failure.rate_limited",
            Self::Context => "failure.context_too_long",
            Self::Model => "failure.model_unavailable",
            Self::Timeout => "failure.response_timed_out",
            Self::Connection => "failure.connection_lost",
            Self::Service => "failure.service_unavailable",
            Self::Unknown => return None,
        };
        let summary = conduit::i18n::text_static(key).to_string();
        Some(if let Some(code) = status_code(raw) {
            conduit::i18n::format(
                "failure.with_http_status",
                &[("summary", summary), ("code", code.to_string())],
            )
        } else {
            summary
        })
    }
}

/// An explicit false overrides words left in a final error's message.
pub(super) fn retry_message(raw: &str) -> Option<conduit::i18n::Message> {
    let value = serde_json::from_str::<Value>(raw).ok();
    let explicit = value
        .as_ref()
        .and_then(|v| v.get("willRetry").or_else(|| payload(v).get("willRetry")))
        .and_then(Value::as_bool);
    let message = message(raw);
    let prefix = retry_prefix(&message);
    if explicit == Some(false) || explicit.is_none() && prefix.is_none() {
        return None;
    }
    let attempt = prefix.and_then(|prefix| prefix.attempt);
    let reconnect = Cause::detect(raw) == Cause::Connection;
    Some(if let Some((current, total)) = attempt {
        conduit::i18n::Message::new(
            if reconnect {
                "failure.reconnecting_attempt"
            } else {
                "failure.retrying_attempt"
            },
            &[
                ("current", current.to_string()),
                ("total", total.to_string()),
            ],
        )
    } else {
        conduit::i18n::Message::new(
            if reconnect {
                "failure.reconnecting"
            } else {
                "failure.retrying"
            },
            &[],
        )
    })
}
