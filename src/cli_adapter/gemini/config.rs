//! Native policy overlay. System-settings overrides require administrator-owned files;
//! ordinary temporary files are silently ignored by Gemini and must never be used.
use super::*;
use anyhow::Context;
use std::{
    fs,
    io::Read,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub(super) const READ_TOOLS: [&str; 4] = ["list_directory", "read_file", "grep_search", "glob"];
const NO_MCP: &str = "__conduit_no_mcp_servers__";

pub(super) struct RunConfig {
    directory: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}
impl Drop for RunConfig {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.directory.join("policy.toml"));
        let _ = fs::remove_dir(&self.directory);
    }
}
impl RunConfig {
    pub fn prepare(request: &AdapterRequest) -> Result<Self> {
        let system = Path::new("C:/ProgramData/gemini-cli");
        let env_path = |key: &str| {
            request
                .route
                .env
                .get(key)
                .map(PathBuf::from)
                .or_else(|| std::env::var_os(key).map(PathBuf::from))
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        request.cwd.join(path)
                    }
                })
        };
        let home = env_path("GEMINI_CLI_HOME")
            .or_else(|| env_path("USERPROFILE"))
            .context(conduit::i18n::text("gemini.config_failed"))?;
        let system_settings = env_path("GEMINI_CLI_SYSTEM_SETTINGS_PATH")
            .unwrap_or_else(|| system.join("settings.json"));
        let defaults = env_path("GEMINI_CLI_SYSTEM_DEFAULTS_PATH")
            .unwrap_or_else(|| system_settings.with_file_name("system-defaults.json"));
        prepare_with_paths(
            request,
            &[
                defaults,
                home.join(".gemini/settings.json"),
                request.cwd.join(".gemini/settings.json"),
                system_settings,
            ],
            &system.join("policies"),
        )
    }
}

fn prepare_with_paths(
    request: &AdapterRequest,
    paths: &[PathBuf],
    system_policies: &Path,
) -> Result<RunConfig> {
    // Gemini ignores --admin-policy when this directory contains policies. Do not override,
    // relocate, or downgrade an enterprise policy: refuse this unsupported configuration.
    match fs::read_dir(system_policies) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.context(conduit::i18n::text("gemini.config_failed"))?;
                ensure!(
                    entry.path().extension().is_none_or(|e| e != "toml"),
                    conduit::i18n::text("gemini.policy_conflict")
                );
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => anyhow::bail!(conduit::i18n::text("gemini.config_failed")),
    }
    // Inspect every scope conservatively, including workspace settings even if currently
    // untrusted. Hooks/discovery can execute before ACP starts and have no approval ticket.
    for path in paths {
        validate_native(&read_settings(path)?)?;
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::env::temp_dir().join(format!(
        "conduit-gemini-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).context(conduit::i18n::text("gemini.config_failed"))?;
    let mut run = RunConfig {
        directory,
        args: vec![],
        environment: request.route.env.clone(),
    };
    let restricted = request.text_only
        || request.permission == PermissionLevel::ReadOnly
        || request.tool_policy == ToolPolicy::Automation;
    let mut policy = String::new();
    if restricted {
        rule(&mut policy, "*", "deny", 998);
        if !request.text_only {
            for name in READ_TOOLS {
                rule(&mut policy, name, "allow", 999);
            }
        }
    } else {
        if request.permission != PermissionLevel::FullAccess {
            rule(&mut policy, "*", "ask_user", 998);
        }
        for name in [
            "invoke_agent",
            "enter_plan_mode",
            "exit_plan_mode",
            "activate_skill",
            "save_memory",
        ] {
            rule(&mut policy, name, "deny", 999);
        }
    }
    let path = run.directory.join("policy.toml");
    fs::write(&path, policy).context(conduit::i18n::text("gemini.config_failed"))?;
    // These are official CLI flags. No credential/home redirect, config mutation or ACL bypass.
    run.args = vec![
        "--admin-policy".into(),
        path.to_string_lossy().into_owned(),
        "--extensions".into(),
        "none".into(),
    ];
    if restricted {
        run.args
            .extend(["--allowed-mcp-server-names".into(), NO_MCP.into()]);
    }
    Ok(run)
}
fn rule(policy: &mut String, name: &str, decision: &str, priority: u16) {
    policy.push_str(&format!(
        "[[rule]]\ntoolName = \"{name}\"\ndecision = \"{decision}\"\npriority = {priority}\n\n"
    ));
}
fn validate_native(settings: &Value) -> Result<()> {
    let configured = |path: &str| {
        settings.pointer(path).is_some_and(|v| match v {
            Value::Null => false,
            Value::Bool(v) => *v,
            Value::String(v) => !v.is_empty(),
            Value::Array(v) => !v.is_empty(),
            Value::Object(v) => !v.is_empty(),
            _ => true,
        })
    };
    ensure!(
        !configured("/adminPolicyPaths"),
        conduit::i18n::text("gemini.policy_conflict")
    );
    for path in [
        "/hooks",
        "/tools/discoveryCommand",
        "/tools/callCommand",
        "/mcp/serverCommand",
        "/experimental/autoMemory",
        "/experimental/enableAgents",
        "/experimental/extensionReloading",
        "/experimental/gemmaModelRouter/enabled",
        "/general/devtools",
    ] {
        ensure!(
            !configured(path),
            conduit::i18n::text("gemini.unsafe_native_config")
        );
    }
    ensure!(
        settings["mcpServers"].get(NO_MCP).is_none()
            && settings
                .pointer(&format!("/admin/mcp/config/{NO_MCP}"))
                .is_none(),
        conduit::i18n::text("gemini.unsafe_native_config")
    );
    Ok(())
}
fn read_settings(path: &Path) -> Result<Value> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(_) => anyhow::bail!(conduit::i18n::text("gemini.config_failed")),
    };
    let mut text = String::new();
    file.take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(|_| anyhow::anyhow!(conduit::i18n::text("gemini.config_failed")))?;
    ensure!(
        text.len() <= 1024 * 1024,
        conduit::i18n::text("gemini.config_failed")
    );
    crate::extensions::jsonc(&text)
        .filter(Value::is_object)
        .ok_or_else(|| anyhow::anyhow!(conduit::i18n::text("gemini.config_failed")))
}

#[cfg(test)]
mod tests;
