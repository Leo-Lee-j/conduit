use super::{Item, Page, Payload, fail, http::Transport, text};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn search(client: &impl Transport, query: &str, cursor: Option<&str>) -> Result<Page> {
    let mut url = url::Url::parse("https://registry.modelcontextprotocol.io/v0.1/servers")?;
    url.query_pairs_mut()
        .append_pair("limit", "30")
        .append_pair("version", "latest");
    if !query.is_empty() {
        url.query_pairs_mut().append_pair("search", query);
    }
    if let Some(cursor) = cursor {
        url.query_pairs_mut().append_pair("cursor", cursor);
    }
    parse_page(client.json(url.as_str(), 2 * 1024 * 1024)?)
}
fn parse_page(value: Value) -> Result<Page> {
    let entries = value["servers"]
        .as_array()
        .ok_or_else(|| fail("invalid_response"))?;
    ensure!(entries.len() <= 100, text("response_limit"));
    let mut items = Vec::new();
    for entry in entries {
        let meta = &entry["_meta"]["io.modelcontextprotocol.registry/official"];
        if meta["status"].as_str().is_some_and(|s| s != "active") || meta["isLatest"] == false {
            continue;
        }
        let server = &entry["server"];
        let name = server["name"].as_str().unwrap_or("");
        if name.is_empty() || name.len() > 200 {
            continue;
        }
        let source_url = [
            server["repository"]["url"].as_str(),
            server["websiteUrl"].as_str(),
        ]
        .into_iter()
        .flatten()
        .find(|s| safe_url(s))
        .unwrap_or("https://registry.modelcontextprotocol.io")
        .to_owned();
        items.push(Item {
            id: format!("mcp:{name}"),
            name: name.into(),
            description: server["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(1200)
                .collect(),
            source_url,
            payload: Payload::Mcp(server.clone()),
        });
    }
    let next = value["metadata"]["nextCursor"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .map(str::to_owned);
    Ok(Page {
        items,
        next,
        ..Default::default()
    })
}
pub fn safe_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
#[derive(Clone, Debug)]
pub struct Field {
    pub key: String,
    pub label: String,
    pub description: String,
    pub required: bool,
    pub secret: bool,
    pub default: String,
    pub choices: Vec<String>,
    format: String,
}
#[derive(Clone, Debug)]
struct Template {
    value: String,
    input: Option<String>,
    variables: Vec<(String, String)>,
    url: bool,
}
#[derive(Clone, Debug)]
struct Argument {
    name: Option<String>,
    value: Template,
}
#[derive(Clone, Debug)]
pub struct OptionSpec {
    pub label: String,
    pub fields: Vec<Field>,
    pub remote: bool,
    target: Template,
    args: Vec<Argument>,
    values: Vec<(String, Template)>,
}
#[derive(Clone, Debug)]
pub struct InstallSpec {
    pub remote: bool,
    pub target: String,
    pub args: Vec<String>,
    pub values: BTreeMap<String, String>,
}
pub fn options(item: &Item) -> Vec<OptionSpec> {
    let Payload::Mcp(server) = &item.payload else {
        return vec![];
    };
    let mut result = Vec::new();
    for remote in server["remotes"].as_array().into_iter().flatten().take(20) {
        if remote["type"] == "streamable-http"
            && let Ok(option) = remote_option(remote)
        {
            result.push(option);
        }
    }
    for package in server["packages"].as_array().into_iter().flatten().take(20) {
        if let Ok(option) = package_option(package) {
            result.push(option);
        }
    }
    result
}
fn remote_option(remote: &Value) -> Result<OptionSpec> {
    let address = remote["url"]
        .as_str()
        .ok_or_else(|| fail("invalid_source"))?;
    let mut probe_url = address.to_owned();
    if let Some(variables) = remote["variables"].as_object() {
        for name in variables.keys() {
            probe_url = probe_url.replace(&format!("{{{name}}}"), "example");
        }
    }
    ensure!(safe_url(&probe_url), text("invalid_source"));
    let mut option = OptionSpec {
        label: format!("HTTP · {address}"),
        fields: vec![],
        remote: true,
        target: Template {
            value: String::new(),
            input: None,
            variables: vec![],
            url: true,
        },
        args: vec![],
        values: vec![],
    };
    let mut input = remote.clone();
    input["value"] = Value::String(address.into());
    option.target = template(&input, "url", "URL", true, true, &mut option.fields)?;
    for (index, header) in remote["headers"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let name = header["name"]
            .as_str()
            .ok_or_else(|| fail("invalid_source"))?;
        ensure!(valid_key(name, true), text("invalid_source"));
        let value = template(
            header,
            &format!("header-{index}"),
            name,
            false,
            false,
            &mut option.fields,
        )?;
        option.values.push((name.into(), value));
    }
    Ok(option)
}
fn package_option(package: &Value) -> Result<OptionSpec> {
    ensure!(
        package["registryType"] == "npm" && package["transport"]["type"] == "stdio",
        text("unsupported")
    );
    ensure!(
        package["runtimeHint"].as_str().is_none_or(|s| s == "npx")
            && package["runtimeArguments"]
                .as_array()
                .is_none_or(|v| v.is_empty())
            && package["registryBaseUrl"]
                .as_str()
                .is_none_or(|s| s.trim_end_matches('/') == "https://registry.npmjs.org")
            && package["fileSha256"].is_null(),
        text("unsupported")
    );
    let name = package["identifier"]
        .as_str()
        .ok_or_else(|| fail("invalid_source"))?;
    ensure!(valid_package(name), text("invalid_source"));
    let version = package["version"]
        .as_str()
        .ok_or_else(|| fail("unsupported"))?;
    semver::Version::parse(version).map_err(|_| fail("unsupported"))?;
    let target = Template {
        value: "npx".into(),
        input: None,
        variables: vec![],
        url: false,
    };
    let mut option = OptionSpec {
        label: format!("npm · {name}@{version}"),
        fields: vec![],
        remote: false,
        target,
        args: vec![],
        values: vec![],
    };
    for value in ["-y".to_owned(), format!("{name}@{version}")] {
        option.args.push(Argument {
            name: None,
            value: Template {
                value,
                input: None,
                variables: vec![],
                url: false,
            },
        });
    }
    for (index, argument) in package["packageArguments"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        ensure!(argument["isRepeated"] != true, text("unsupported"));
        let name = match argument["type"].as_str() {
            Some("positional") => None,
            Some("named") => {
                let name = argument["name"]
                    .as_str()
                    .ok_or_else(|| fail("invalid_source"))?;
                ensure!(
                    name.starts_with('-')
                        && name.len() <= 100
                        && !name.chars().any(char::is_whitespace),
                    text("invalid_source")
                );
                Some(name.to_owned())
            }
            _ => return Err(fail("unsupported")),
        };
        let label = name
            .clone()
            .unwrap_or_else(|| format!("{} {}", text("argument"), index + 1));
        option.args.push(Argument {
            name,
            value: template(
                argument,
                &format!("arg-{index}"),
                &label,
                false,
                false,
                &mut option.fields,
            )?,
        });
    }
    for (index, env) in package["environmentVariables"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let name = env["name"].as_str().ok_or_else(|| fail("invalid_source"))?;
        ensure!(valid_key(name, false), text("invalid_source"));
        option.values.push((
            name.into(),
            template(
                env,
                &format!("env-{index}"),
                name,
                false,
                false,
                &mut option.fields,
            )?,
        ));
    }
    Ok(option)
}
fn valid_package(name: &str) -> bool {
    let part = |s: &str| {
        !s.is_empty()
            && !s.starts_with(['.', '-'])
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    name.len() <= 214
        && if let Some(scoped) = name.strip_prefix('@') {
            scoped
                .split_once('/')
                .is_some_and(|(a, b)| part(a) && part(b))
        } else {
            part(name)
        }
}
fn valid_key(key: &str, header: bool) -> bool {
    !key.is_empty()
        && key.len() <= 100
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || (header && b == b'-'))
}
fn template(
    input: &Value,
    key: &str,
    label: &str,
    required: bool,
    url: bool,
    fields: &mut Vec<Field>,
) -> Result<Template> {
    ensure!(fields.len() <= 60, text("response_limit"));
    let mut template = Template {
        value: input["value"].as_str().unwrap_or("").into(),
        input: None,
        variables: vec![],
        url,
    };
    if let Some(variables) = input["variables"].as_object() {
        for (name, definition) in variables {
            ensure!(valid_key(name, true), text("invalid_source"));
            let field_key = format!("{key}:{name}");
            add_field(definition, &field_key, name, true, fields)?;
            template.variables.push((name.clone(), field_key));
        }
    }
    if input.get("value").is_none() && template.variables.is_empty() {
        add_field(input, key, label, required, fields)?;
        template.input = Some(key.into());
    }
    ensure!(template.value.len() <= 8192, text("response_limit"));
    Ok(template)
}
fn add_field(
    input: &Value,
    key: &str,
    label: &str,
    required: bool,
    fields: &mut Vec<Field>,
) -> Result<()> {
    ensure!(fields.len() < 60, text("response_limit"));
    let format = input["format"].as_str().unwrap_or("string");
    ensure!(
        matches!(format, "string" | "number" | "boolean" | "filepath"),
        text("unsupported")
    );
    let secret = input["isSecret"] == true
        || label.to_ascii_lowercase().contains("token")
        || label.eq_ignore_ascii_case("authorization")
        || label.to_ascii_lowercase().contains("key");
    fields.push(Field {
        key: key.into(),
        label: label.into(),
        description: input["description"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(500)
            .collect(),
        required: required || input["isRequired"] == true,
        secret,
        default: input["default"].as_str().unwrap_or("").into(),
        choices: input["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        format: format.into(),
    });
    Ok(())
}
impl OptionSpec {
    pub fn resolve(&self, inputs: &BTreeMap<String, String>) -> Result<InstallSpec> {
        for field in &self.fields {
            let value = inputs.get(&field.key).map(String::as_str).unwrap_or("");
            ensure!(
                !field.required || !value.trim().is_empty(),
                "{}: {}",
                field.label,
                text("required")
            );
            ensure!(
                value.len() <= 8192 && !value.contains(['\0', '\r', '\n']),
                "{}: {}",
                field.label,
                text("invalid_value")
            );
            if !value.is_empty() {
                ensure!(
                    field.choices.is_empty() || field.choices.iter().any(|s| s == value),
                    "{}: {}",
                    field.label,
                    text("invalid_value")
                );
                ensure!(
                    match field.format.as_str() {
                        "number" => value.parse::<f64>().is_ok_and(f64::is_finite),
                        "boolean" => matches!(value, "true" | "false"),
                        _ => true,
                    },
                    "{}: {}",
                    field.label,
                    text("invalid_value")
                );
            }
        }
        let target = self.target.resolve(inputs)?;
        if self.remote {
            ensure!(safe_url(&target), text("invalid_source"));
        }
        let mut args = vec![];
        for argument in &self.args {
            let value = argument.value.resolve(inputs)?;
            if value.is_empty() && argument.value.input.is_some() {
                continue;
            }
            if let Some(name) = &argument.name {
                args.push(name.clone());
            }
            if !value.is_empty() {
                args.push(value);
            }
        }
        let mut values = BTreeMap::new();
        for (key, value) in &self.values {
            let value = value.resolve(inputs)?;
            if !value.is_empty() {
                values.insert(key.clone(), value);
            }
        }
        Ok(InstallSpec {
            remote: self.remote,
            target,
            args,
            values,
        })
    }
}
impl Template {
    fn resolve(&self, inputs: &BTreeMap<String, String>) -> Result<String> {
        if let Some(key) = &self.input {
            return Ok(inputs.get(key).cloned().unwrap_or_default());
        }
        let mut result = String::new();
        let mut remaining = self.value.as_str();
        while let Some(start) = remaining.find('{') {
            result.push_str(&remaining[..start]);
            let end = remaining[start..]
                .find('}')
                .ok_or_else(|| fail("invalid_value"))?
                + start;
            let name = &remaining[start + 1..end];
            let key = self
                .variables
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, key)| key)
                .ok_or_else(|| fail("invalid_value"))?;
            let value = inputs.get(key).map(String::as_str).unwrap_or("");
            if self.url {
                for byte in value.bytes() {
                    if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                        result.push(byte as char);
                    } else {
                        result.push_str(&format!("%{byte:02X}"));
                    }
                }
            } else {
                result.push_str(value);
            }
            remaining = &remaining[end + 1..];
        }
        result.push_str(remaining);
        ensure!(
            !result.contains(['\0', '\r', '\n', '{', '}']),
            text("invalid_value")
        );
        Ok(result)
    }
}
pub fn npm_runtime_available() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let folders: Vec<_> = std::env::split_paths(&path).collect();
    let present = |names: &[&str]| {
        folders
            .iter()
            .any(|folder| names.iter().any(|name| folder.join(name).is_file()))
    };
    if cfg!(windows) {
        present(&["node.exe"]) && present(&["npx.cmd", "npx.exe"])
    } else {
        present(&["node"]) && present(&["npx"])
    }
}

pub fn alias(item: &Item) -> String {
    let base: String = item
        .name
        .rsplit('/')
        .next()
        .unwrap_or("server")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(60)
        .collect();
    format!("{base}-{}", &super::digest(item.id.as_bytes())[..6])
}
pub fn preview(
    engine: crate::domain::Provider,
    original: Option<&str>,
    name: &str,
    spec: &InstallSpec,
) -> Result<String> {
    use crate::domain::Provider;
    ensure!(engine != Provider::Qoder, text("unsupported"));
    let mut server = serde_json::Map::new();
    if spec.remote {
        server.insert("url".into(), json!(spec.target));
        if engine != Provider::Codex {
            server.insert(
                "type".into(),
                json!(if engine == Provider::Claude {
                    "http"
                } else {
                    "remote"
                }),
            );
        }
        if !spec.values.is_empty() {
            server.insert(
                if engine == Provider::Codex {
                    "http_headers"
                } else {
                    "headers"
                }
                .into(),
                json!(spec.values),
            );
        }
    } else {
        if engine == Provider::OpenCode {
            server.insert("type".into(), json!("local"));
            let command: Vec<_> = std::iter::once(spec.target.clone())
                .chain(spec.args.clone())
                .collect();
            server.insert("command".into(), json!(command));
        } else {
            server.insert("command".into(), json!(spec.target));
            server.insert("args".into(), json!(spec.args));
        }
        if !spec.values.is_empty() {
            server.insert(
                if engine == Provider::OpenCode {
                    "environment"
                } else {
                    "env"
                }
                .into(),
                json!(spec.values),
            );
        }
    }
    crate::extensions::mcp::merge_server(engine, original, name, server)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Provider;
    #[test]
    fn remote_inputs_are_required_encoded_once_and_secrets_map_to_each_engine() {
        let option = remote_option(&json!({"type":"streamable-http","url":"https://{region}.example.com/{project}/mcp",
            "variables":{"region":{"isRequired":true},"project":{"isRequired":true}},
            "headers":[{"name":"Authorization","value":"Bearer {token}","variables":{"token":{"isSecret":true}}}]})).unwrap();
        assert!(option.resolve(&BTreeMap::new()).is_err());
        let values = BTreeMap::from([
            ("url:region".into(), "us".into()),
            ("url:project".into(), "a/b".into()),
            ("header-0:token".into(), "secret-value".into()),
        ]);
        let spec = option.resolve(&values).unwrap();
        assert_eq!(spec.target, "https://us.example.com/a%2Fb/mcp");
        for engine in [Provider::Claude, Provider::Codex, Provider::OpenCode] {
            let original = if engine == Provider::Codex {
                "# keep\nmodel = 'keep'\n"
            } else {
                "{ /* keep */ \"theme\": \"keep\" }"
            };
            let output = preview(engine, Some(original), "service", &spec).unwrap();
            assert!(output.contains("keep"));
            assert!(output.contains("Bearer secret-value"));
            assert!(preview(engine, Some(&output), "service", &spec).is_err());
        }
        let mut bad = values;
        bad.insert("header-0:token".into(), "x\r\nInjected: true".into());
        assert!(option.resolve(&bad).is_err());
    }
    #[test]
    fn npm_arguments_keep_boundaries_and_unsafe_runtime_recipes_are_not_installable() {
        let package = json!({"registryType":"npm","identifier":"@example/server","version":"1.2.3","transport":{"type":"stdio"},
            "packageArguments":[{"type":"named","name":"--path","isRequired":true}],
            "environmentVariables":[{"name":"API_KEY","isRequired":true,"isSecret":true}]});
        let option = package_option(&package).unwrap();
        let spec = option
            .resolve(&BTreeMap::from([
                ("arg-0".into(), "C:/空间/my project".into()),
                ("env-0".into(), "token".into()),
            ]))
            .unwrap();
        assert_eq!(
            spec.args,
            [
                "-y",
                "@example/server@1.2.3",
                "--path",
                "C:/空间/my project"
            ]
        );
        let output = preview(Provider::OpenCode, None, "service", &spec).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["mcp"]["service"]["command"][4], "C:/空间/my project");
        assert_eq!(parsed["mcp"]["service"]["environment"]["API_KEY"], "token");
        let mut flags = package.clone();
        flags["environmentVariables"] = json!([]);
        flags["packageArguments"] = json!([
            {"type":"named","name":"--verbose","value":""},
            {"type":"named","name":"--optional"}
        ]);
        let flag_option = package_option(&flags).unwrap();
        assert_eq!(flag_option.fields.len(), 1);
        assert_eq!(
            flag_option.resolve(&BTreeMap::new()).unwrap().args,
            ["-y", "@example/server@1.2.3", "--verbose"]
        );
        for (key, value) in [
            ("version", json!("latest")),
            ("identifier", json!("https://example.com/a.js")),
            ("runtimeArguments", json!([{"value":"--call=bad"}])),
            ("registryType", json!("oci")),
            ("fileSha256", json!("unverified")),
        ] {
            let mut bad = package.clone();
            bad[key] = value;
            assert!(package_option(&bad).is_err(), "{key}");
        }
    }
}
