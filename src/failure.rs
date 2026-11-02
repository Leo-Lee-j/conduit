//! Compact, deduplicated engine diagnostics with actionable recovery hints.
use crate::domain::{ChatItem, ItemKind, SessionRecord};

/// Whether a failure notice repeats text an earlier assistant/tool message
/// already shows. CLI wrappers often echo the same provider error in both the
/// assistant output and the task result, and showing it twice reads as a
/// duplicated reply.
/// Turn a raw CLI failure blob into a one-line banner summary. The runtime
/// reports some failures as pretty-printed JSON (e.g. a Codex `turn.error`
/// object); those get parsed so the banner shows the cause, not the payload.
pub(crate) fn friendly_failure(error: &str) -> String {
    let trimmed = error.trim();
    let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return trimmed.to_string();
    };
    let get = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|key| {
            json.get(*key)
                .and_then(|value| {
                    value
                        .as_str()
                        .or_else(|| value.get("message").and_then(|v| v.as_str()))
                })
                .map(str::to_string)
                .filter(|value| !value.is_empty())
        })
    };
    let message = get(&["message", "error"]).or_else(|| {
        json.get("errors")
            .and_then(|v| v.as_array())
            .and_then(|v| v.first())
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    });
    let detail = json
        .get("codexErrorInfo")
        .and_then(|info| info.get("responseStreamDisconnected"))
        .map(|disconnected| {
            let status = disconnected["httpStatusCode"].as_i64();
            let detail = disconnected["additionalDetails"].as_str().unwrap_or("");
            let brief = detail.split(['\n', ';']).next().unwrap_or(detail).trim();
            let brief = if brief.chars().count() > 120 {
                format!("{}…", brief.chars().take(120).collect::<String>())
            } else {
                brief.to_string()
            };
            match (status, brief.is_empty()) {
                (Some(code), false) => format!("连接中断（HTTP {code}）：{brief}"),
                (Some(code), true) => format!("连接中断（HTTP {code}）"),
                (None, false) => format!("连接中断：{brief}"),
                (None, true) => "连接中断".to_string(),
            }
        });
    detail.or(message).unwrap_or_else(|| trimmed.to_string())
}
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
    let friendly = friendly_failure(error);
    let line = friendly
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("CLI 执行失败，请查看详情。");
    if line.chars().count() > 160 {
        format!("{}…", line.chars().take(160).collect::<String>())
    } else {
        line.to_string()
    }
}

pub(crate) fn failure_title(error: &str) -> &'static str {
    let text = friendly_failure(error).to_ascii_lowercase();
    if text.contains("401")
        || text.contains("403")
        || text.contains("api key")
        || text.contains("unauthorized")
        || text.contains("authentication")
    {
        "认证或权限失败"
    } else if text.contains("429")
        || text.contains("rate limit")
        || text.contains("quota")
        || text.contains("overloaded")
    {
        "服务限流或额度不足"
    } else if text.contains("context")
        && (text.contains("length") || text.contains("window") || text.contains("limit"))
    {
        "上下文超出限制"
    } else if text.contains("model")
        && (text.contains("not found")
            || text.contains("not available")
            || text.contains("does not exist"))
        || text.contains("provider") && (text.contains("不可用") || text.contains("not found"))
    {
        "模型或 Provider 配置不可用"
    } else if text.contains("timeout")
        || text.contains("timed out")
        || text.contains("连接")
        || text.contains("network")
        || text.contains("disconnect")
        || text.contains("connection")
        || text.contains("500")
        || text.contains("502")
        || text.contains("503")
        || text.contains("504")
    {
        "连接或服务异常"
    } else {
        "本次回复未完成"
    }
}

pub(crate) fn failure_guidance(error: &str) -> &'static str {
    match failure_title(error) {
        "认证或权限失败" => "请检查 Provider 的凭证、登录状态及模型访问权限。",
        "服务限流或额度不足" => "请稍后重试，或检查服务额度并切换 Provider / 模型。",
        "上下文超出限制" => "请压缩上下文、换用更大上下文的模型，或新建会话。",
        "模型或 Provider 配置不可用" => {
            "请在设置中检查引擎、Provider 和模型配置，然后重试。"
        }
        "连接或服务异常" => "请检查网络及服务状态；任务结束后可手动重试。",
        _ => "可展开或复制错误详情；检查引擎配置后再手动重试。",
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
    let friendly = friendly_failure(
        error
            .trim()
            .strip_prefix("执行失败：")
            .unwrap_or(error.trim()),
    );
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
        item.status = format!("repeats:{count}");
        item.detail = error.to_string();
        return index;
    }
    let key = format!("{prefix}-error-{}", session.messages.len());
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    session.messages.push(ChatItem {
        elapsed_ms: None,
        key,
        created_at,
        kind: ItemKind::Error,
        text: error.to_string(),
        input: String::new(),
        detail: error.to_string(),
        status: String::new(),
        approval: None,
        model: None,
    });
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
            );
            previous.detail = if item.detail.is_empty() {
                item.text
            } else {
                item.detail
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
        assert!(failure_guidance("401 unauthorized").contains("凭证"));
        assert!(failure_guidance("429 rate limit").contains("额度"));
        assert!(failure_guidance("maximum context length exceeded").contains("压缩"));
        assert!(failure_guidance("model does not exist").contains("模型配置"));
        assert_eq!(
            failure_summary(r#"{"error":{"message":"service unavailable"}}"#),
            "service unavailable"
        );
        assert!(is_engine_error_text("API Error: 503"));
        assert!(!is_engine_error_text(
            "Here is how to handle an API Error: 503"
        ));
    }
}
