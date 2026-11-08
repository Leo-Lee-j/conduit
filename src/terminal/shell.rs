use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ShellProfile {
    #[default]
    Auto,
    PowerShell,
    Cmd,
}

impl ShellProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => conduit::i18n::text_static("terminal.shell.default_shell"),
            Self::PowerShell => "PowerShell",
            Self::Cmd => "cmd",
        }
    }

    pub fn resolve(self) -> Result<(PathBuf, Vec<String>)> {
        let windows = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        if self != Self::Cmd {
            // Do not search the current workspace or accept relative PATH entries.
            if let Some(path) = std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .filter(|p| p.is_absolute())
                    .map(|p| p.join("pwsh.exe"))
                    .find(|p| p.is_file())
            }) {
                return Ok((path, vec!["-NoLogo".into()]));
            }
            let path = windows.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
            if path.is_file() {
                return Ok((path, vec!["-NoLogo".into()]));
            }
            if self == Self::PowerShell {
                bail!(conduit::i18n::text(
                    "terminal.shell.powershell_not_found_choose_cmd_or_install"
                ));
            }
        }
        let path = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && p.is_file())
            .unwrap_or_else(|| windows.join(r"System32\cmd.exe"));
        if !path.is_file() {
            bail!(conduit::i18n::text("terminal.shell.cmd_exe_not_found"));
        }
        Ok((path, vec!["/d".into(), "/q".into()]))
    }
}
