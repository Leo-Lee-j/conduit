//! Local CLI configuration and bounded, non-model version diagnostics.
use crate::{
    child::{Child, available},
    domain::Provider,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct CliSettings {
    pub claude_path: Option<PathBuf>,
    pub codex_path: Option<PathBuf>,
}
impl CliSettings {
    pub fn path(&self, provider: Provider) -> Option<&Path> {
        match provider {
            Provider::Claude => &self.claude_path,
            Provider::Codex => &self.codex_path,
        }
        .as_deref()
    }
    pub fn set(&mut self, provider: Provider, path: Option<PathBuf>) {
        *match provider {
            Provider::Claude => &mut self.claude_path,
            Provider::Codex => &mut self.codex_path,
        } = path;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSource {
    Environment,
    Settings,
    SearchPath,
    CommonLocation,
}
impl PathSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Environment => "环境变量",
            Self::Settings => "应用配置",
            Self::SearchPath => "PATH",
            Self::CommonLocation => "常见安装位置",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ResolvedCli {
    pub path: PathBuf,
    pub source: PathSource,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Launcher {
    Native,
    Cmd,
}

pub fn validate_executable(path: &Path) -> Result<PathBuf> {
    validate_launcher(path).map(|(path, _)| path)
}

fn validate_launcher(path: &Path) -> Result<(PathBuf, Launcher)> {
    if !path.is_absolute() {
        bail!("请选择绝对路径的 Windows CLI");
    }
    let launcher = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => Launcher::Native,
        Some(ext) if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") => {
            Launcher::Cmd
        }
        _ => {
            bail!("目前只支持 Windows 原生 .exe 或 npm 全局 .cmd 包装器；不支持 .ps1 或 WSL 启动器")
        }
    };
    if !path.is_file() {
        bail!("文件不存在或不可访问：{}", path.display());
    }
    Ok((path.canonicalize()?, launcher))
}

/// Build the process to actually spawn. `.cmd` shims (npm global CLIs) are
/// wrapped in cmd.exe because CreateProcess cannot run batch files directly.
pub fn command_for(path: &Path, args: &[String]) -> (PathBuf, Vec<String>) {
    let launcher = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") => {
            Launcher::Cmd
        }
        _ => Launcher::Native,
    };
    match launcher {
        Launcher::Native => (path.to_path_buf(), args.to_vec()),
        Launcher::Cmd => {
            let shell = std::env::var_os("ComSpec")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("cmd.exe"));
            let mut command = vec![
                "/d".into(),
                "/s".into(),
                "/c".into(),
                crate::child::normalized_dir(path)
                    .to_string_lossy()
                    .into_owned(),
            ];
            command.extend(args.iter().cloned());
            (shell, command)
        }
    }
}
fn resolve_candidates(
    explicit: Option<(PathBuf, PathSource)>,
    candidates: impl IntoIterator<Item = (PathBuf, PathSource)>,
) -> Result<ResolvedCli> {
    if let Some((path, source)) = explicit {
        // An explicit invalid path is an error; do not silently run a different CLI.
        return Ok(ResolvedCli {
            path: validate_executable(&path)
                .with_context(|| format!("{}路径无效", source.label()))?,
            source,
        });
    }
    for (path, source) in candidates {
        if let Ok(path) = validate_executable(&path) {
            return Ok(ResolvedCli { path, source });
        }
    }
    bail!("未找到 CLI；请在设置中选择已安装的 .exe 或 npm 全局 .cmd 启动器")
}

pub fn resolve(provider: Provider, settings: &CliSettings) -> Result<ResolvedCli> {
    let (key, filename) = match provider {
        Provider::Claude => ("CONDUIT_CLAUDE_PATH", "claude.exe"),
        Provider::Codex => ("CONDUIT_CODEX_PATH", "codex.exe"),
    };
    let explicit = std::env::var_os(key)
        .map(|p| (PathBuf::from(p), PathSource::Environment))
        .or_else(|| {
            settings
                .path(provider)
                .map(|p| (p.to_path_buf(), PathSource::Settings))
        });
    let shim = filename.replace(".exe", ".cmd");
    let mut candidates: Vec<_> = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .flat_map(|p| {
                    [
                        (p.join(filename), PathSource::SearchPath),
                        (p.join(&shim), PathSource::SearchPath),
                    ]
                })
                .collect()
        })
        .unwrap_or_default();
    let common = match provider {
        Provider::Claude => {
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".local/bin/claude.exe"))
        }
        Provider::Codex => std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Programs/OpenAI/Codex/bin/codex.exe")),
    };
    if let Some(path) = common {
        candidates.push((path, PathSource::CommonLocation));
    }
    resolve_candidates(explicit, candidates)
}

pub fn tested_version(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "2.1.301",
        Provider::Codex => "0.163.0",
    }
}

#[derive(Debug)]
pub struct VersionInfo {
    pub version: String,
    pub verified_version: bool,
}
fn parse_version(provider: Provider, output: &str) -> Result<VersionInfo> {
    let signature = match provider {
        Provider::Claude => "Claude Code",
        Provider::Codex => "codex-cli",
    };
    let line = output
        .lines()
        .find(|line| line.contains(signature))
        .ok_or_else(|| anyhow::anyhow!("未识别到 {} 版本标识，请检查所选程序", provider.label()))?;
    let version = line
        .split_whitespace()
        .find(|part| {
            let base = part.split(['-', '+']).next().unwrap_or("");
            base.split('.').count() == 3
                && base
                    .split('.')
                    .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        })
        .ok_or_else(|| anyhow::anyhow!("CLI 版本格式暂不支持：{}", line))?
        .to_string();
    Ok(VersionInfo {
        verified_version: version == tested_version(provider),
        version,
    })
}

pub struct VersionProbe {
    pub path: PathBuf,
    pub events: Receiver<Result<VersionInfo, String>>,
    cancelled: Arc<AtomicBool>,
}
impl VersionProbe {
    pub fn start(provider: Provider, path: PathBuf) -> Self {
        let (sender, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let executable = path.clone();
        thread::spawn(move || {
            let result = probe_command(
                &executable,
                &["--version".into()],
                &cancel,
                Duration::from_secs(10),
            )
            .and_then(|output| parse_version(provider, &output))
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        Self {
            path,
            events,
            cancelled,
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Drop for VersionProbe {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn read_available(file: &mut File, output: &mut Vec<u8>) -> Result<()> {
    if let Ok(count) = available(file)
        && count > 0
    {
        let mut buffer = [0; 4096];
        let count = file.read(&mut buffer[..(count as usize).min(4096)])?;
        if output.len() + count > 64 * 1024 {
            bail!("CLI 版本输出超过 64 KiB，已停止检测");
        }
        output.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}
fn probe_command(
    executable: &Path,
    args: &[String],
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<String> {
    let (program, full_args) = command_for(executable, args);
    let (child, mut pipes) = Child::spawn(&program, &full_args, &std::env::temp_dir())?;
    drop(pipes.stdin);
    let result = (|| {
        let started = Instant::now();
        let mut stdout = vec![];
        let mut stderr = vec![];
        loop {
            if cancel.load(Ordering::Acquire) {
                bail!("检测已取消");
            }
            if started.elapsed() > timeout {
                bail!("CLI 版本检测超时，已停止进程树");
            }
            read_available(&mut pipes.stdout, &mut stdout)?;
            read_available(&mut pipes.stderr, &mut stderr)?;
            if let Some(code) = child.exit_code()? {
                // Drain all output after exit, including a version line without a newline.
                while available(&pipes.stdout).unwrap_or(0) > 0 {
                    read_available(&mut pipes.stdout, &mut stdout)?;
                }
                while available(&pipes.stderr).unwrap_or(0) > 0 {
                    read_available(&mut pipes.stderr, &mut stderr)?;
                }
                if code != 0 {
                    bail!(
                        "CLI --version 退出码 {code}：{}",
                        String::from_utf8_lossy(&stderr)
                    );
                }
                return Ok(String::from_utf8_lossy(&stdout).into_owned());
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    child.terminate()?;
    while child.active_processes()? > 0 || child.exit_code()?.is_none() {
        thread::sleep(Duration::from_millis(10));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_versions_are_visible_without_claiming_compatibility() {
        assert!(
            parse_version(Provider::Claude, "2.1.301 (Claude Code)\r\n")
                .unwrap()
                .verified_version
        );
        let info = parse_version(Provider::Codex, "codex-cli 0.164.0").unwrap();
        assert_eq!(info.version, "0.164.0");
        assert!(!info.verified_version);
        assert!(
            !parse_version(Provider::Codex, "codex-cli 0.163.0-alpha.1")
                .unwrap()
                .verified_version
        );
        assert!(parse_version(Provider::Codex, "PowerShell 7.5.0").is_err());
    }
    #[test]
    fn explicit_invalid_path_does_not_fall_back_and_scripts_are_rejected() {
        let fallback = std::env::current_exe().unwrap();
        let missing = std::env::temp_dir().join("conduit-nonexistent-cli.exe");
        assert!(
            resolve_candidates(
                Some((missing, PathSource::Settings)),
                [(fallback.clone(), PathSource::SearchPath)]
            )
            .is_err()
        );
        assert!(validate_executable(Path::new("relative.exe")).is_err());
        assert!(validate_executable(&std::env::temp_dir().join("cli.ps1")).is_err());
        assert!(validate_executable(&std::env::temp_dir().join("cli.cmd")).is_err());
        let shim = std::env::temp_dir().join("conduit-test-shim.cmd");
        std::fs::write(&shim, "@echo off\r\n").unwrap();
        let canonical_shim = validate_executable(&shim).unwrap();
        let (program, full_args) = command_for(&canonical_shim, &["--version".into()]);
        assert!(program.to_string_lossy().to_lowercase().contains("cmd.exe"));
        let launch_shim = crate::child::normalized_dir(&canonical_shim);
        assert_eq!(
            full_args,
            vec![
                "/d",
                "/s",
                "/c",
                launch_shim.to_string_lossy().as_ref(),
                "--version"
            ]
        );
        assert!(!full_args[3].starts_with(r"\\?\"));
        let (native_program, native_args) = command_for(&fallback, &["--version".into()]);
        assert_eq!(native_program, fallback);
        assert_eq!(native_args, vec!["--version"]);
        let _ = std::fs::remove_file(&shim);
        let resolved =
            resolve_candidates(Some((fallback.clone(), PathSource::Environment)), []).unwrap();
        assert_eq!(resolved.source, PathSource::Environment);
        assert_eq!(resolved.path, fallback.canonicalize().unwrap());
    }
    #[test]
    fn version_probe_bounds_output_and_cleans_up_on_timeout_or_cancellation() {
        let shell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let arguments = |command: &str| {
            ["-NoProfile", "-NonInteractive", "-Command", command].map(String::from)
        };
        let output = probe_command(
            &shell,
            &arguments("[Console]::Write('codex-cli 0.163.0')"),
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(output, "codex-cli 0.163.0");
        assert!(
            probe_command(
                &shell,
                &arguments("Start-Sleep -Seconds 60"),
                &AtomicBool::new(false),
                Duration::from_millis(300)
            )
            .unwrap_err()
            .to_string()
            .contains("超时")
        );
        assert!(
            probe_command(
                &shell,
                &arguments("Start-Sleep -Seconds 60"),
                &AtomicBool::new(true),
                Duration::from_secs(10)
            )
            .unwrap_err()
            .to_string()
            .contains("已取消")
        );
        assert!(
            probe_command(
                &shell,
                &arguments("[Console]::Write(('x' * 70000))"),
                &AtomicBool::new(false),
                Duration::from_secs(10)
            )
            .unwrap_err()
            .to_string()
            .contains("64 KiB")
        );
        assert!(
            probe_command(
                &shell,
                &arguments("exit 7"),
                &AtomicBool::new(false),
                Duration::from_secs(10)
            )
            .unwrap_err()
            .to_string()
            .contains("退出码 7")
        );
    }
    #[test]
    #[ignore = "requires installed CLIs; --version only, no model call"]
    fn installed_clis_report_versions() {
        for provider in [Provider::Claude, Provider::Codex] {
            let resolved = resolve(provider, &CliSettings::default()).unwrap();
            let probe = VersionProbe::start(provider, resolved.path);
            let info = probe
                .events
                .recv_timeout(Duration::from_secs(15))
                .unwrap()
                .unwrap();
            println!(
                "{} {} (matches verified version: {})",
                provider.label(),
                info.version,
                info.verified_version
            );
        }
    }
}
