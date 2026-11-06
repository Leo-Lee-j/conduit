//! Build an explicit native MCP configuration preview, preserving other text.
use super::*;
use anyhow::{Result, ensure};

/// Change one native setting while preserving comments, credentials and other servers.
pub fn set_enabled_preview(
    engine: Provider,
    original: &str,
    name: &str,
    enabled: bool,
) -> Result<String> {
    ensure!(
        engine.descriptor().capabilities.mcp_toggle,
        "此引擎需要在来源配置中管理 MCP 启停"
    );
    if engine == Provider::Codex {
        let mut document = original
            .parse::<toml_edit::DocumentMut>()
            .map_err(|_| anyhow::anyhow!("原配置 TOML 无效，请先修复"))?;
        let server = document
            .get_mut("mcp_servers")
            .and_then(|servers| servers.get_mut(name))
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| anyhow::anyhow!("MCP 服务已移动或定义不是表，请刷新后重试"))?;
        ensure!(
            server.get("disabled").and_then(|v| v.as_bool()) != Some(true),
            "此定义还有 disabled 字段，请编辑来源文件"
        );
        let mut value = toml_edit::Value::from(enabled);
        if let Some(previous) = server.get("enabled").and_then(|v| v.as_value()) {
            ensure!(
                previous.as_bool().is_some(),
                "enabled 必须是布尔值，请先修复配置"
            );
            *value.decor_mut() = previous.decor().clone();
        }
        server.insert("enabled", toml_edit::Item::Value(value));
        return Ok(document.to_string());
    }
    let mut expected =
        jsonc(original).ok_or_else(|| anyhow::anyhow!("原配置 JSON / JSONC 无效"))?;
    let server = expected
        .get_mut("mcp")
        .and_then(|v| v.get_mut(name))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| anyhow::anyhow!("MCP 服务已移动，请刷新后重试"))?;
    ensure!(
        server.get("disabled") != Some(&Value::Bool(true)),
        "此定义还有 disabled 字段，请编辑来源文件"
    );
    ensure!(
        server.get("enabled").is_none_or(Value::is_boolean),
        "enabled 必须是布尔值，请先修复配置"
    );
    server.insert("enabled".into(), Value::Bool(enabled));
    let (root, end) =
        object_range(original, Some("mcp")).ok_or_else(|| anyhow::anyhow!("无法定位 MCP 对象"))?;
    let (start, finish) = object_range(&original[root..=end], Some(name))
        .ok_or_else(|| anyhow::anyhow!("无法定位 MCP 服务"))?;
    let (start, finish) = (root + start, root + finish);
    let items = tokens(&original[start..=finish]);
    let mut depth = 0;
    let mut replacement = None;
    for (index, (_, _, token)) in items.iter().enumerate() {
        if depth == 1
            && serde_json::from_str::<String>(token).ok().as_deref() == Some("enabled")
            && items.get(index + 1).is_some_and(|t| t.2 == ":")
        {
            let value = items
                .get(index + 2)
                .ok_or_else(|| anyhow::anyhow!("缺少 enabled 值"))?;
            replacement = Some((start + value.0, start + value.1));
            break;
        }
        if matches!(token.as_str(), "{" | "[") {
            depth += 1;
        }
        if matches!(token.as_str(), "}" | "]") {
            depth -= 1;
        }
    }
    let mut result = original.to_owned();
    if let Some((from, to)) = replacement {
        result.replace_range(from..to, if enabled { "true" } else { "false" });
    } else {
        let body = tokens(&original[start + 1..finish]);
        let comma = if body.is_empty() || body.last().is_some_and(|t| t.2 == ",") {
            ""
        } else {
            ","
        };
        // Insert a newline before the comma so trailing // comments cannot eat it.
        result.insert_str(finish, &format!("\n{comma}\"enabled\": {enabled}\n"));
    }
    ensure!(
        jsonc(&result).as_ref() == Some(&expected),
        "无法安全修改此定义，请编辑来源文件"
    );
    Ok(result)
}

pub fn add_preview(
    engine: Provider,
    original: Option<&str>,
    name: &str,
    remote: bool,
    target: &str,
    arguments: &str,
) -> Result<String> {
    ensure!(
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
        "服务名称只允许字母、数字、下划线和连字符"
    );
    ensure!(
        !target.trim().is_empty() && !target.contains(['\0', '\n', '\r']),
        "请输入有效的命令或 URL"
    );
    let args = arguments
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut server = serde_json::Map::new();
    if remote {
        let url = url::Url::parse(target).map_err(|_| anyhow::anyhow!("URL 无效"))?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none(),
            "URL 必须是 HTTP(S) 地址，不能包含账号或密码"
        );
        server.insert("url".into(), Value::String(target.into()));
        if engine != Provider::Codex {
            server.insert(
                "type".into(),
                Value::String(
                    if engine == Provider::OpenCode {
                        "remote"
                    } else {
                        "http"
                    }
                    .into(),
                ),
            );
        }
    } else {
        if engine == Provider::OpenCode {
            let command = std::iter::once(target.to_string())
                .chain(args)
                .map(Value::String)
                .collect();
            server.insert("command".into(), Value::Array(command));
            server.insert("type".into(), Value::String("local".into()));
        } else {
            server.insert("command".into(), Value::String(target.into()));
            server.insert("args".into(), serde_json::to_value(args)?);
        }
    }
    if engine == Provider::Codex {
        let original = original.unwrap_or("");
        let value: toml::Table =
            toml::from_str(original).map_err(|_| anyhow::anyhow!("原配置 TOML 无效，请先修复"))?;
        ensure!(
            value
                .get("mcp_servers")
                .is_none_or(|v| v.get(name).is_none()),
            "同名 MCP 服务已存在，请编辑原服务"
        );
        let server: toml::Table = serde_json::from_value(Value::Object(server))?;
        return Ok(format!(
            "{original}\n[mcp_servers.{name}]\n{}",
            toml::to_string_pretty(&server)?
        ));
    }
    let key = if engine == Provider::Claude {
        "mcpServers"
    } else {
        "mcp"
    };
    let original = original.unwrap_or("{\n}\n");
    let value = jsonc(original)
        .filter(Value::is_object)
        .ok_or_else(|| anyhow::anyhow!("原配置 JSON / JSONC 无效，请先修复"))?;
    let server = serde_json::to_string_pretty(&Value::Object(server))?;
    let (start, end, contents) = if let Some(existing) = value.get(key) {
        ensure!(existing.is_object(), "MCP 配置不是对象，请编辑原文件");
        ensure!(
            existing.get(name).is_none(),
            "同名 MCP 服务已存在，请编辑原服务"
        );
        let (start, end) = object_range(original, Some(key))
            .ok_or_else(|| anyhow::anyhow!("无法定位 MCP 配置，请编辑原文件"))?;
        (
            start,
            end,
            format!("{}: {server}", serde_json::to_string(name)?),
        )
    } else {
        let (start, end) =
            object_range(original, None).ok_or_else(|| anyhow::anyhow!("无法定位配置对象"))?;
        (
            start,
            end,
            format!(
                "{}: {{\n{}: {server}\n}}",
                serde_json::to_string(key)?,
                serde_json::to_string(name)?
            ),
        )
    };
    let tokens = tokens(&original[start + 1..end]);
    let comma = if tokens.is_empty() || tokens.last().is_some_and(|(_, _, t)| t == ",") {
        ""
    } else {
        ","
    };
    let result = format!(
        "{}{comma}\n{contents}\n{}",
        &original[..end],
        &original[end..]
    );
    ensure!(jsonc(&result).is_some(), "无法合并配置，请编辑原文件");
    Ok(result)
}

// Token spans retain the original byte positions, including UTF-8 and comments.
fn tokens(text: &str) -> Vec<(usize, usize, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut result = vec![];
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                i += 1;
            }
            i += 2;
            continue;
        }
        let start = i;
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            result.push((start, i, text[start..i].into()));
        } else if b"{}[]:,".contains(&bytes[i]) {
            i += 1;
            result.push((start, i, text[start..i].into()));
        } else {
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !b"{}[]:,/".contains(&bytes[i])
            {
                i += 1;
            }
            if i == start {
                i += 1;
            }
            result.push((start, i, String::new()));
        }
    }
    result
}

fn object_range(text: &str, key: Option<&str>) -> Option<(usize, usize)> {
    let tokens = tokens(text);
    let mut depth = 0;
    let mut start = None;
    let mut target_depth = 0;
    for (index, (offset, _, token)) in tokens.iter().enumerate() {
        if key.is_some()
            && depth == 1
            && token.starts_with('"')
            && serde_json::from_str::<String>(token).ok().as_deref() == key
            && tokens.get(index + 1).is_some_and(|t| t.2 == ":")
            && tokens.get(index + 2).is_some_and(|t| t.2 == "{")
        {
            start = Some(tokens[index + 2].0);
            target_depth = 1;
        }
        if token == "{" || token == "[" {
            if key.is_none() && depth == 0 && token == "{" {
                start = Some(*offset);
            }
            depth += 1;
        } else if token == "}" || token == "]" {
            depth -= 1;
            if depth == target_depth
                && let Some(start) = start
            {
                return Some((start, *offset));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggles_preserve_jsonc_comments_nested_values_and_other_servers() {
        for original in [
            "{\"mcp\":{\"中文\":{\"command\":[\"node\",\"{enabled}\"],//keep\n},\"other\":{\"enabled\":true}},\"theme\":\"dark\"}",
            "{\"mcp\":{\"中文\":{\"env\":{\"enabled\":\"secret\"},\"enabled\":true /*keep*/}}}",
            "{\"mcp\":{\"中文\":{\"command\":[\"node\"] //keep\n}}}",
        ] {
            let disabled =
                set_enabled_preview(Provider::OpenCode, original, "中文", false).unwrap();
            assert!(disabled.contains("keep"));
            assert_eq!(jsonc(&disabled).unwrap()["mcp"]["中文"]["enabled"], false);
            let enabled = set_enabled_preview(Provider::OpenCode, &disabled, "中文", true).unwrap();
            assert_eq!(jsonc(&enabled).unwrap()["mcp"]["中文"]["enabled"], true);
        }
        assert!(set_enabled_preview(Provider::Claude, "{}", "a", false).is_err());
        assert!(set_enabled_preview(Provider::OpenCode, "{}", "missing", false).is_err());
    }
    #[test]
    fn toggles_preserve_toml_comments_inline_tables_and_literal_names() {
        for original in [
            "# keep\nmodel='default'\n[mcp_servers.'a.b']\ncommand='node'\nenabled=true # keep enabled\n[mcp_servers.'a.b'.env]\nTOKEN='secret'\n",
            "mcp_servers = { 'a.b' = { command = 'node' } } # keep\n",
        ] {
            let result = set_enabled_preview(Provider::Codex, original, "a.b", false).unwrap();
            assert!(result.contains("# keep"));
            let mut expected: toml::Table = toml::from_str(original).unwrap();
            expected["mcp_servers"]["a.b"]
                .as_table_mut()
                .unwrap()
                .insert("enabled".into(), toml::Value::Boolean(false));
            assert_eq!(toml::from_str::<toml::Table>(&result).unwrap(), expected);
        }
    }
    #[test]
    fn previews_preserve_comments_other_fields_and_reject_duplicate_names() {
        for original in [
            "{\n// 注释\n\"theme\":\"dark\"\n}",
            "{\"mcp\":{\"old\":{\"url\":\"https://a.invalid\"},}, /* 保留 */ \"name\":\"中文\"}",
        ] {
            let preview = add_preview(
                Provider::OpenCode,
                Some(original),
                "new",
                false,
                "node",
                "a.js\nspace arg",
            )
            .unwrap();
            assert!(preview.contains(if original.contains("注释") {
                "// 注释"
            } else {
                "/* 保留 */"
            }));
            let value = jsonc(&preview).unwrap();
            assert_eq!(value["mcp"]["new"]["command"][2], "space arg");
            assert!(
                add_preview(
                    Provider::OpenCode,
                    Some(&preview),
                    "new",
                    true,
                    "https://example.com/mcp",
                    ""
                )
                .is_err()
            );
        }
        let original = "# keep\nmodel = 'original'\n[mcp_servers.old]\ncommand = 'old'\n";
        let preview = add_preview(
            Provider::Codex,
            Some(original),
            "new",
            true,
            "https://example.com/mcp",
            "",
        )
        .unwrap();
        assert!(preview.starts_with(original));
        let value: toml::Table = toml::from_str(&preview).unwrap();
        assert_eq!(value["model"].as_str(), Some("original"));
        assert!(add_preview(Provider::Codex, Some(&preview), "new", false, "node", "").is_err());
    }
}
