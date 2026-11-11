//! Compact, deduplicated engine diagnostics with actionable recovery hints.
use crate::domain::{ChatItem, ItemKind, SessionRecord};

mod diagnostics;

/// Extract the original cause from native JSON diagnostics.
pub(crate) fn friendly_failure(error: &str) -> String {
    if codex_client_restricted(error) {
        return conduit::i18n::text_static("failure.codex_official_clients_only").to_string();
    }
    diagnostics::message(error)
}

pub(crate) fn retry_activity(error: &str) -> Option<String> {
    retry_activity_message(error).map(|message| message.render())
}

pub(crate) fn retry_activity_message(error: &str) -> Option<conduit::i18n::Message> {
    diagnostics::retry_message(error)
}

pub(crate) fn codex_client_restricted(error: &str) -> bool {
    error
        .to_ascii_lowercase()
        .contains("only allows codex official clients")
}

/// Whether a result echoes a diagnostic already shown in this turn.
pub(crate) fn same_failure(message: &str, error: &str) -> bool {
    let message = message.trim();
    let error = error.trim();
    if message.is_empty() || error.is_empty() {
        return false;
    }
    message == error || message.contains(error) || error.contains(message)
}

/// Keep long diagnostics out of the transcript's default reading flow.
pub(crate) fn failure_summary(error: &str) -> String {
    let error = error
        .trim()
        .strip_prefix("执行失败：")
        .unwrap_or(error.trim());
    let friendly = if codex_client_restricted(error) {
        friendly_failure(error)
    } else {
        diagnostics::Cause::detect(error)
            .summary(error)
            .unwrap_or_else(|| {
                let message = friendly_failure(error);
                if message.starts_with(['{', '[']) || !message.is_empty() && message.is_ascii() {
                    conduit::i18n::text_static("failure.unrecognized_response").into()
                } else {
                    message
                }
            })
    };
    let line = friendly
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(conduit::i18n::text_static(
            "failure.cli_execution_failed_view_details",
        ));
    if line.chars().count() > 160 {
        format!("{}…", line.chars().take(160).collect::<String>())
    } else {
        line.to_string()
    }
}

fn failure_category(error: &str) -> &'static str {
    use diagnostics::Cause;
    match diagnostics::Cause::detect(error) {
        Cause::Credentials | Cause::Permission => "认证或权限失败",
        Cause::Quota | Cause::RateLimit => "服务限流或额度不足",
        Cause::Context => "上下文超出限制",
        Cause::Model => "模型或 Provider 配置不可用",
        Cause::Timeout | Cause::Connection | Cause::Service => "连接或服务异常",
        Cause::Unknown if error.contains("连接") => "连接或服务异常",
        Cause::Unknown => "本次回复未完成",
    }
}

pub(crate) fn failure_title(error: &str) -> &'static str {
    use diagnostics::Cause;
    let key = match Cause::detect(error) {
        Cause::RateLimit => "failure.title_rate_limited",
        Cause::Quota => "failure.title_quota_exhausted",
        Cause::Timeout => "failure.title_timeout",
        Cause::Connection => "failure.title_connection",
        Cause::Service => "failure.title_service",
        _ => return conduit::i18n::source(failure_category(error)),
    };
    conduit::i18n::text_static(key)
}

pub(crate) fn failure_guidance(error: &str) -> &'static str {
    if codex_client_restricted(error) {
        return conduit::i18n::text_static("failure.codex_client_restriction_guidance");
    }
    match diagnostics::Cause::detect(error) {
        diagnostics::Cause::RateLimit => {
            return conduit::i18n::text_static("failure.rate_limit_guidance");
        }
        diagnostics::Cause::Quota => return conduit::i18n::text_static("failure.quota_guidance"),
        _ => {}
    }
    match failure_category(error) {
        "认证或权限失败" => {
            conduit::i18n::text_static("failure.check_provider_credentials_sign_in_status_and")
        }
        "服务限流或额度不足" => {
            conduit::i18n::text_static("failure.try_again_later_or_check_service_quota")
        }
        "上下文超出限制" => {
            conduit::i18n::text_static("failure.compact_the_context_use_a_model_with")
        }
        "模型或 Provider 配置不可用" => {
            conduit::i18n::text_static("failure.check_engine_provider_and_model_configuration_in")
        }
        "连接或服务异常" => {
            conduit::i18n::text_static("failure.check_your_network_and_service_status_you")
        }
        _ => conduit::i18n::text_static("failure.expand_or_copy_error_details_check_engine"),
    }
}

pub(crate) fn is_engine_error_text(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("API Error:")
        || text.starts_with("API error:")
        || text.starts_with("Error: API")
}

pub(crate) fn failure_repeats(item: &ChatItem) -> u64 {
    item.status
        .strip_prefix("repeats:")
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
        .max(1)
}

fn fingerprint(error: &str) -> String {
    // Group retries by their original cause, not by a translated generic label.
    let friendly = diagnostics::fingerprint(error);
    let mut text = friendly.trim().to_ascii_lowercase();
    for marker in ["request id:", "request_id:", "(request id:"] {
        if let Some(index) = text.find(marker) {
            text.truncate(index);
        }
    }
    text.trim().to_string()
}

fn retry_notice(item: &ChatItem) -> bool {
    if item.kind != ItemKind::Notice {
        return false;
    }
    let text = item.text.to_ascii_lowercase();
    text.contains("reconnect")
        || text.contains("retry")
        || text.contains("重试")
        || text.contains("重新连接")
}

/// Coalesce only consecutive errors/retry notices within the same user turn.
/// A completion echo updates the card but is not another retry occurrence.
pub(crate) fn record_failure(
    session: &mut SessionRecord,
    prefix: &str,
    error: &str,
    terminal: bool,
) -> usize {
    let candidate = session
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| !retry_notice(item));
    if let Some((index, previous)) = candidate
        && previous.kind == ItemKind::Error
        && fingerprint(&previous.text) == fingerprint(error)
    {
        let count = failure_repeats(previous).saturating_add(u64::from(!terminal));
        let item = &mut session.messages[index];
        item.status = format!("repeats:{count}").into();
        item.detail = error.to_string();
        return index;
    }
    let key = format!("{prefix}-error-{}", session.messages.len());
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    session
        .messages
        .push(ChatItem::from(crate::domain::ChatItemData {
            reasoning_summary: None,
            elapsed_ms: None,
            key,
            created_at,
            kind: ItemKind::Error,
            text: error.to_string(),
            input: String::new(),
            detail: error.to_string(),
            status: Default::default(),
            approval: None,
            model: None,
        }));
    session.messages.len() - 1
}

pub(crate) fn coalesce_restored_failures(session: &mut SessionRecord) {
    let mut kept: Vec<ChatItem> = Vec::with_capacity(session.messages.len());
    for item in session.messages.drain(..) {
        if item.kind == ItemKind::Error
            && let Some(previous) = kept.iter_mut().rev().find(|item| !retry_notice(item))
            && previous.kind == ItemKind::Error
            && fingerprint(&previous.text) == fingerprint(&item.text)
        {
            previous.status = format!(
                "repeats:{}",
                failure_repeats(previous).saturating_add(failure_repeats(&item))
            )
            .into();
            previous.detail = if item.detail.is_empty() {
                item.text.clone()
            } else {
                item.detail.clone()
            };
            continue;
        }
        kept.push(item);
    }
    session.messages = kept;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> SessionRecord {
        serde_json::from_str(r#"{"id":1,"provider":"Codex","created_at":0,"exit_code":null}"#)
            .unwrap()
    }
    #[test]
    fn repeated_errors_and_completion_echo_share_one_persisted_card() {
        let _locale = crate::test_support::chinese();
        let mut session = session();
        for _ in 0..3 {
            record_failure(&mut session, "turn", "API Error: 503 unavailable", false);
        }
        record_failure(&mut session, "turn", "API Error: 503 unavailable", true);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(failure_repeats(&session.messages[0]), 3);
        let mut restored: SessionRecord =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        coalesce_restored_failures(&mut restored);
        assert_eq!(failure_repeats(&restored.messages[0]), 3);
        record_failure(&mut session, "turn", "API Error: 401 unauthorized", false);
        assert_eq!(
            session.messages.len(),
            2,
            "different errors must retain their diagnosis"
        );
    }
    #[test]
    fn duplicate_errors_do_not_cross_user_message_boundaries() {
        let _locale = crate::test_support::chinese();
        let mut session = session();
        record_failure(&mut session, "first", "timeout", false);
        let user: ChatItem =
            serde_json::from_str(r#"{"key":"user","kind":"User","text":"try another task"}"#)
                .unwrap();
        session.messages.push(user);
        record_failure(&mut session, "second", "timeout", false);
        coalesce_restored_failures(&mut session);
        assert_eq!(session.messages.len(), 3);
        assert_eq!(failure_repeats(&session.messages[0]), 1);
    }
    #[test]
    fn restored_duplicate_error_cards_merge_and_keep_the_latest_detail() {
        let _locale = crate::test_support::chinese();
        let mut session = session();
        record_failure(&mut session, "turn", "network timeout", false);
        let mut repeated = session.messages[0].clone();
        repeated.key = "turn-second".into();
        repeated.detail = "latest full diagnostic".into();
        session.messages.push(repeated);
        coalesce_restored_failures(&mut session);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(failure_repeats(&session.messages[0]), 2);
        assert_eq!(session.messages[0].detail, "latest full diagnostic");
    }
    #[test]
    fn common_failures_have_specific_recovery_guidance() {
        let _locale = crate::test_support::chinese();
        assert!(failure_guidance("401 unauthorized").contains("凭证"));
        assert!(failure_guidance("429 rate limit").contains("频率"));
        assert!(failure_guidance("insufficient_quota").contains("额度"));
        assert!(failure_guidance("maximum context length exceeded").contains("压缩"));
        assert!(failure_guidance("model does not exist").contains("模型配置"));
        assert_eq!(
            failure_summary(r#"{"error":{"message":"service unavailable"}}"#),
            conduit::i18n::text_static("failure.service_unavailable")
        );
        assert!(is_engine_error_text("API Error: 503"));
        assert!(!is_engine_error_text(
            "Here is how to handle an API Error: 503"
        ));
    }
    #[test]
    fn native_retries_keep_progress_raw_details_and_terminal_state() {
        let _locale = crate::test_support::chinese();
        let error = |attempt, will_retry| {
            serde_json::json!({
            "error": {"message": format!("Reconnecting... {attempt}/5: stream disconnected before completion"),
                "codexErrorInfo": "responseStreamDisconnected"},
            "willRetry": will_retry
        }).to_string()
        };
        let mut session = session();
        let first = error(1, true);
        let second = error(2, true);
        record_failure(&mut session, "turn", &first, false);
        record_failure(&mut session, "turn", &second, false);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(failure_repeats(&session.messages[0]), 2);
        assert_eq!(session.messages[0].detail, second);
        assert_eq!(
            retry_activity(&second).as_deref(),
            Some("正在重新连接（2/5）…")
        );
        assert_eq!(failure_summary(&second), "与模型服务的连接中断");
        assert_eq!(
            retry_activity("codex Reconnecting... 3/5").as_deref(),
            Some("正在重新连接（3/5）…")
        );
        let terminal = error(5, false);
        record_failure(&mut session, "turn", &terminal, true);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].detail, terminal);
        assert!(retry_activity(&session.messages[0].detail).is_none());
        assert!(retry_activity("已取消；不会自动重试。").is_none());
        assert!(retry_activity("We should discuss reconnecting to a database").is_none());
        // Different native causes must not merge just because their translations match.
        record_failure(&mut session, "turn", "HTTP 503: overloaded", false);
        record_failure(&mut session, "turn", "HTTP 502: gateway error", false);
        assert_eq!(session.messages.len(), 3);
        let status_only = |code| {
            serde_json::json!({"error": {"message": "Reconnecting... 1/5",
            "codexErrorInfo": {"httpConnectionFailed": {"httpStatusCode":code}}}, "willRetry":true})
            .to_string()
        };
        record_failure(&mut session, "turn", &status_only(502), false);
        record_failure(&mut session, "turn", &status_only(503), false);
        assert_eq!(session.messages.len(), 5);
    }

    #[test]
    fn native_errors_have_localized_causes_and_keep_real_status_codes() {
        let _locale = crate::test_support::chinese();
        for (raw, expected) in [
            (
                "API Error: 401 unauthorized",
                "登录凭证无效或已过期，请检查登录状态（HTTP 401）",
            ),
            (
                "unexpected status 403 Forbidden",
                "当前账户无权使用此模型或服务（HTTP 403）",
            ),
            (
                "429 Too Many Requests",
                "请求过于频繁，服务暂时限制了访问（HTTP 429）",
            ),
            ("insufficient_quota", "账户额度或余额不足，暂时无法继续请求"),
            (
                "maximum context length exceeded",
                "会话内容超过了模型可处理的长度",
            ),
            ("model does not exist", "所选模型或服务配置不可用"),
            ("request timed out", "等待模型响应超时"),
            (
                r#"{"error":{"message":"Reconnecting... 1/5","codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":503}}},"willRetry":true}"#,
                "模型服务暂时不可用，请稍后重试（HTTP 503）",
            ),
            (r#""internalServerError""#, "模型服务暂时不可用，请稍后重试"),
        ] {
            assert_eq!(failure_summary(raw), expected, "{raw}");
        }
        assert_eq!(
            friendly_failure(r#"{"error":{"message":"original detail"},"willRetry":true}"#),
            "original detail"
        );
    }
}
