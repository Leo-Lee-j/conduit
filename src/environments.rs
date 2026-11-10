//! Named WSL/SSH terminal targets. Credentials remain with the native clients.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Target {
    Wsl,
    Ssh,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment {
    pub id: u64,
    pub workspace: PathBuf,
    pub name: String,
    pub target: Target,
    pub host: String,
    pub directory: String,
}
fn invalid() -> String {
    conduit::i18n::text("environment.invalid")
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
impl Environment {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 256,
            invalid()
        );
        ensure!(
            !self.host.is_empty()
                && self.host.len() <= 256
                && !self.host.starts_with('-')
                && self
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-@".contains(&b)),
            invalid()
        );
        ensure!(
            self.directory.starts_with('/')
                && self.directory.len() <= 4096
                && !self.directory.chars().any(char::is_control),
            invalid()
        );
        Ok(())
    }
    pub fn command(&self) -> Result<(PathBuf, Vec<String>)> {
        self.validate()?;
        let windows = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let (exe, args) = match self.target {
            Target::Wsl => (
                windows.join("System32/wsl.exe"),
                vec![
                    "--distribution".into(),
                    self.host.clone(),
                    "--cd".into(),
                    self.directory.clone(),
                    "--exec".into(),
                    "/bin/sh".into(),
                    "-l".into(),
                ],
            ),
            Target::Ssh => (
                windows.join("System32/OpenSSH/ssh.exe"),
                vec![
                    "-t".into(),
                    "-o".into(),
                    "StrictHostKeyChecking=ask".into(),
                    "-o".into(),
                    "ConnectTimeout=15".into(),
                    "--".into(),
                    self.host.clone(),
                    format!("cd -- {} && exec /bin/sh -l", quote(&self.directory)),
                ],
            ),
        };
        ensure!(
            exe.is_file(),
            conduit::i18n::text("environment.missing_client")
        );
        Ok((exe, args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_reject_option_injection_and_quote_remote_paths() {
        let mut env = Environment {
            id: 1,
            workspace: "C:/repo".into(),
            name: "Remote".into(),
            target: Target::Ssh,
            host: "user@host".into(),
            directory: "/srv/a'b $(touch bad)".into(),
        };
        assert!(env.validate().is_ok());
        assert_eq!(quote(&env.directory), "'/srv/a'\\''b $(touch bad)'");
        env.host = "-oProxyCommand=bad".into();
        assert!(env.validate().is_err());
        env.host = "host".into();
        env.directory = "/srv/x\nwhoami".into();
        assert!(env.validate().is_err());
    }
}
