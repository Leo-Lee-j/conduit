//! Named WSL/SSH terminal targets. Credentials remain with the native clients.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    Wsl,
    Ssh,
}
impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wsl => "WSL",
            Self::Ssh => "SSH",
        }
    }
    pub fn executable(self) -> PathBuf {
        let windows = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        windows.join(match self {
            Self::Wsl => "System32/wsl.exe",
            Self::Ssh => "System32/OpenSSH/ssh.exe",
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment {
    pub id: u64,
    pub workspace: PathBuf,
    pub name: String,
    pub target: Target,
    pub host: String,
    pub directory: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
fn valid_host(host: &str) -> bool {
    let (user, hostname) = host
        .rsplit_once('@')
        .map_or((None, host), |(u, h)| (Some(u), h));
    let word = |s: &str| {
        !s.is_empty()
            && !s.starts_with('-')
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    };
    user.is_none_or(word) && (word(hostname) || hostname.parse::<std::net::Ipv6Addr>().is_ok())
}
impl Environment {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 256
                && !self.name.chars().any(char::is_control),
            conduit::i18n::text("environment.invalid_name")
        );
        let endpoint_valid = match self.target {
            Target::Ssh => valid_host(&self.host),
            Target::Wsl => {
                !self.host.is_empty()
                    && !self.host.starts_with('-')
                    && !self.host.chars().any(char::is_control)
            }
        };
        ensure!(
            self.host.len() <= 256 && endpoint_valid,
            conduit::i18n::text(match self.target {
                Target::Wsl => "environment.invalid_wsl",
                Target::Ssh => "environment.invalid_ssh",
            })
        );
        ensure!(
            self.port != Some(0) && (self.target == Target::Ssh || self.port.is_none()),
            conduit::i18n::text("environment.invalid_port")
        );
        ensure!(
            (self.directory.is_empty() || self.directory == "~" || self.directory.starts_with('/'))
                && self.directory.len() <= 4096
                && !self.directory.chars().any(char::is_control),
            conduit::i18n::text("environment.invalid_directory")
        );
        Ok(())
    }
    pub fn arguments(&self) -> Result<Vec<String>> {
        self.validate()?;
        Ok(match self.target {
            Target::Wsl => vec![
                "--distribution".into(),
                self.host.clone(),
                "--cd".into(),
                if self.directory.is_empty() {
                    "~".into()
                } else {
                    self.directory.clone()
                },
            ],
            Target::Ssh => {
                let mut args = vec![
                    "-t".into(),
                    "-o".into(),
                    "StrictHostKeyChecking=ask".into(),
                    "-o".into(),
                    "ConnectTimeout=15".into(),
                    "-o".into(),
                    "ServerAliveInterval=30".into(),
                    "-o".into(),
                    "ServerAliveCountMax=3".into(),
                ];
                if let Some(port) = self.port {
                    args.extend(["-p".into(), port.to_string()]);
                }
                args.extend(["--".into(), self.host.clone()]);
                // With no directory override, let SSH use the native login shell / RemoteCommand.
                if !self.directory.is_empty() && self.directory != "~" {
                    let script = format!(
                        "cd -- {} && exec \"${{SHELL:-/bin/sh}}\" -l",
                        quote(&self.directory)
                    );
                    // Explicit POSIX parsing also works when the login shell is fish or csh.
                    args.push(format!("/bin/sh -c {}", quote(&script)));
                }
                args
            }
        })
    }
    pub fn command(&self) -> Result<(PathBuf, Vec<String>)> {
        let args = self.arguments()?;
        let exe = self.target.executable();
        ensure!(
            exe.is_file(),
            conduit::i18n::text(match self.target {
                Target::Wsl => "environment.missing_wsl",
                Target::Ssh => "environment.missing_ssh",
            })
        );
        Ok((exe, args))
    }
    pub fn destination(&self) -> String {
        match self.port {
            Some(port) => format!("{} · {}", self.host, port),
            None => self.host.clone(),
        }
    }
}

/// Listing never starts a distribution. Bound process lifetime, output and elapsed time.
pub fn distributions() -> Result<Vec<String>> {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };
    let exe = Target::Wsl.executable();
    ensure!(
        exe.is_file(),
        conduit::i18n::text("environment.missing_wsl")
    );
    use std::{
        fs::File,
        os::windows::{io::OwnedHandle, process::CommandExt},
        process::{Command, Stdio},
    };
    // WSL's broker does not reliably complete inside our kill-on-close CLI Job.
    // This fixed read-only command has no shell; reap its process on every exit path.
    struct Probe(std::process::Child);
    impl Drop for Probe {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Probe(
        Command::new(&exe)
            .args(["--list", "--quiet"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()?,
    );
    let mut stdout = File::from(OwnedHandle::from(child.0.stdout.take().unwrap()));
    let start = Instant::now();
    let mut output = Vec::new();
    loop {
        let exit = child.0.try_wait()?;
        let count = crate::child::available(&stdout).unwrap_or(0).min(65536) as usize;
        if count > 0 {
            let mut bytes = vec![0; count];
            let read = stdout.read(&mut bytes)?;
            output.extend_from_slice(&bytes[..read]);
        }
        ensure!(
            output.len() <= 65536,
            conduit::i18n::text("environment.detect_failed")
        );
        if let Some(code) = exit {
            ensure!(
                code.success(),
                conduit::i18n::text("environment.detect_failed")
            );
            return Ok(parse_distributions(&output));
        }
        ensure!(
            start.elapsed() < Duration::from_secs(8),
            conduit::i18n::text("environment.detect_failed")
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}
fn parse_distributions(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.contains(&0) {
        let words: Vec<_> = bytes
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let mut names: Vec<_> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    fn environment() -> Environment {
        Environment {
            id: 0,
            workspace: "C:/repo".into(),
            name: "Remote".into(),
            target: Target::Ssh,
            host: "user@host".into(),
            directory: String::new(),
            port: None,
        }
    }
    #[test]
    fn targets_reject_option_injection_and_quote_remote_paths() {
        let mut env = environment();
        env.directory = "/srv/a'b $(touch bad)".into();
        assert_eq!(quote(&env.directory), "'/srv/a'\\''b $(touch bad)'");
        let command = env.arguments().unwrap().pop().unwrap();
        assert!(command.starts_with("/bin/sh -c '"));
        assert!(
            !command.contains("cd -- /srv/"),
            "directory must never be interpolated without quoting"
        );
        for host in [
            "-oProxyCommand=bad",
            "user@-bad",
            "a@b@host",
            "host;id",
            "host:2222",
            "host\n",
        ] {
            env.host = host.into();
            assert!(env.validate().is_err(), "{host}");
        }
        env.host = "user@2001:db8::1".into();
        assert!(env.validate().is_ok());
        env.directory = "/srv/x\nwhoami".into();
        assert!(env.validate().is_err());
    }
    #[test]
    fn defaults_ports_and_legacy_configs_preserve_native_client_behavior() {
        let mut env = environment();
        let args = env.arguments().unwrap();
        assert!(!args.contains(&"-p".into()));
        assert_eq!(&args[args.len() - 2..], ["--", "user@host"]);
        env.directory = "~".into();
        assert_eq!(env.arguments().unwrap(), args);
        env.directory.clear();
        env.port = Some(2222);
        assert!(
            env.arguments()
                .unwrap()
                .windows(2)
                .any(|w| w == ["-p", "2222"])
        );
        env.port = Some(0);
        assert!(env.validate().is_err());
        env.port = None;
        env.target = Target::Wsl;
        env.host = "Ubuntu Dev".into();
        assert_eq!(
            env.arguments().unwrap(),
            ["--distribution", "Ubuntu Dev", "--cd", "~"]
        );
        let mut json = serde_json::to_value(&env).unwrap();
        json.as_object_mut().unwrap().remove("port");
        assert!(
            serde_json::from_value::<Environment>(json)
                .unwrap()
                .port
                .is_none()
        );
    }
    #[test]
    fn discovery_decodes_windows_utf16_without_losing_distribution_names() {
        let bytes: Vec<_> = "\u{feff}Ubuntu\r\nDebian\r\nUbuntu\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(parse_distributions(&bytes), ["Debian", "Ubuntu"]);
        assert_eq!(parse_distributions(b"Ubuntu\r\n"), ["Ubuntu"]);
    }
    #[test]
    fn editing_preserves_identity_and_workspace_and_does_not_consume_capacity() {
        let mut workflow = crate::workflow::Workflow::default();
        let id = workflow.save_environment(environment()).unwrap();
        let mut edited = workflow.environments[0].clone();
        edited.name = "Updated".into();
        workflow.save_environment(edited.clone()).unwrap();
        assert_eq!(workflow.environments.len(), 1);
        assert_eq!(workflow.environments[0].id, id);
        edited.workspace = "C:/other".into();
        assert!(workflow.save_environment(edited).is_err());
        for _ in 1..50 {
            workflow.save_environment(environment()).unwrap();
        }
        assert!(
            workflow
                .save_environment(workflow.environments[0].clone())
                .is_ok()
        );
        assert!(workflow.save_environment(environment()).is_err());
    }
}
