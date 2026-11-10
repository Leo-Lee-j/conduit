//! Shared registration contract for native CLI integrations.
use crate::domain::Provider;

pub struct EngineDescriptor {
    pub provider: Provider,
    pub id: &'static str,
    pub label: &'static str,
    pub executable: &'static str,
    pub path_env: &'static str,
    pub install_env: &'static str,
    pub install_path: &'static str,
    pub npm_binary: Option<&'static str>,
    pub tested_version: &'static str,
    pub version_signature: &'static str,
    pub guide: &'static str,
    pub capabilities: EngineCapabilities,
    pub adapter: crate::cli_adapter::AdapterFactory,
    pub commands: Option<crate::commands::Source>,
}

pub struct EngineCapabilities {
    pub model_services: bool,
    pub mcp_toggle: bool,
    pub protocol: &'static str,
}

pub const REGISTRY: &[EngineDescriptor] = &[
    EngineDescriptor {
        provider: Provider::Claude,
        id: "claude",
        label: "Claude Code",
        executable: "claude.exe",
        path_env: "CONDUIT_CLAUDE_PATH",
        install_env: "USERPROFILE",
        install_path: ".local/bin/claude.exe",
        npm_binary: None,
        tested_version: "2.1.301",
        version_signature: "Claude Code",
        guide: "https://code.claude.com/docs/en/setup",
        capabilities: EngineCapabilities {
            model_services: true,
            mcp_toggle: false,
            protocol: "Stream JSON",
        },
        adapter: crate::cli_adapter::claude,
        commands: None,
    },
    EngineDescriptor {
        provider: Provider::Codex,
        id: "codex",
        label: "Codex",
        executable: "codex.exe",
        path_env: "CONDUIT_CODEX_PATH",
        install_env: "LOCALAPPDATA",
        install_path: "Programs/OpenAI/Codex/bin/codex.exe",
        npm_binary: None,
        tested_version: "0.163.0",
        version_signature: "codex-cli",
        guide: "https://developers.openai.com/codex/cli",
        capabilities: EngineCapabilities {
            model_services: true,
            mcp_toggle: true,
            protocol: "App Server",
        },
        adapter: crate::cli_adapter::codex,
        commands: None,
    },
    EngineDescriptor {
        provider: Provider::OpenCode,
        id: "opencode",
        label: "OpenCode",
        executable: "opencode.exe",
        path_env: "CONDUIT_OPENCODE_PATH",
        install_env: "USERPROFILE",
        install_path: ".opencode/bin/opencode.exe",
        npm_binary: Some("node_modules/opencode-ai/bin/opencode.exe"),
        tested_version: "1.19.2",
        version_signature: "",
        guide: "https://opencode.ai/docs/",
        capabilities: EngineCapabilities {
            model_services: false,
            mcp_toggle: true,
            protocol: "ACP",
        },
        adapter: crate::cli_adapter::opencode,
        commands: Some(crate::cli_adapter::OPENCODE_COMMANDS),
    },
    EngineDescriptor {
        provider: Provider::Qoder,
        id: "qoder",
        label: "Qoder CLI",
        executable: "qodercli.exe",
        path_env: "CONDUIT_QODER_PATH",
        install_env: "USERPROFILE",
        install_path: ".qoder/bin/qodercli/qodercli.exe",
        npm_binary: None,
        tested_version: "1.1.85",
        version_signature: "",
        guide: "https://docs.qoder.com/cli/installation",
        capabilities: EngineCapabilities {
            model_services: false,
            mcp_toggle: false,
            protocol: "ACP",
        },
        adapter: crate::cli_adapter::qoder,
        commands: None,
    },
];

impl Provider {
    pub fn descriptor(self) -> &'static EngineDescriptor {
        REGISTRY
            .iter()
            .find(|engine| engine.provider == self)
            .expect("registered CLI engine")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_covers_all_engines_with_unique_stable_identifiers() {
        let _locale = crate::test_support::chinese();
        assert_eq!(REGISTRY.len(), Provider::ALL.len());
        let mut ids = std::collections::HashSet::new();
        let mut envs = std::collections::HashSet::new();
        for provider in Provider::ALL {
            let descriptor = provider.descriptor();
            assert!(ids.insert(descriptor.id));
            assert!(envs.insert(descriptor.path_env));
            assert_eq!(descriptor.provider, provider);
            assert!(!descriptor.capabilities.protocol.is_empty());
        }
    }
}
