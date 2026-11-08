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

pub mod updates;

#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq)]
pub struct CliSettings {
    #[serde(default)]
    pub engines: std::collections::HashMap<Provider, EngineSettings>,
    pub claude_path: Option<PathBuf>,
    pub codex_path: Option<PathBuf>,
    #[serde(default)]
    pub opencode_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct EngineSettings {
    pub enabled: bool,
    pub path: Option<PathBuf>,
    pub models: crate::models::preferences::ModelPreferences,
}
impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
            models: Default::default(),
        }
    }
}
impl CliSettings {
    pub fn models_mut(
        &mut self,
        provider: Provider,
    ) -> &mut crate::models::preferences::ModelPreferences {
        let path = self.path(provider).map(Path::to_path_buf);
        &mut self
            .engines
            .entry(provider)
            .or_insert_with(|| EngineSettings {
                path,
                ..Default::default()
            })
            .models
    }
    pub fn path(&self, provider: Provider) -> Option<&Path> {
        if let Some(settings) = self.engines.get(&provider) {
            return settings.path.as_deref();
        }
        match provider {
            Provider::Claude => &self.claude_path,
            Provider::Codex => &self.codex_path,
            Provider::OpenCode => &self.opencode_path,
            Provider::Qoder => return None,
        }
        .as_deref()
    }
    pub fn set(&mut self, provider: Provider, path: Option<PathBuf>) {
        self.engines.entry(provider).or_default().path = path;
        *match provider {
            Provider::Claude => &mut self.claude_path,
            Provider::Codex => &mut self.codex_path,
            Provider::OpenCode => &mut self.opencode_path,
            Provider::Qoder => return,
        } = None;
    }
    pub fn enabled(&self, provider: Provider) -> bool {
        self.engines
            .get(&provider)
            .is_none_or(|settings| settings.enabled)
    }
    pub fn set_enabled(&mut self, provider: Provider, enabled: bool) {
        let path = self.path(provider).map(Path::to_path_buf);
        self.set(provider, path);
        self.engines.get_mut(&provider).unwrap().enabled = enabled;
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
            Self::Environment => conduit::i18n::text_static("settings.models.environment_variable"),
            Self::Settings => conduit::i18n::text_static("cli.application_configuration"),
            Self::SearchPath => "PATH",
            Self::CommonLocation => conduit::i18n::text_static("cli.common_install_location"),
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
        bail!(conduit::i18n::text(
            "cli.select_a_windows_cli_using_an_absolute"
        ));
    }
    let launcher = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => Launcher::Native,
        Some(ext) if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") => {
            Launcher::Cmd
        }
        _ => {
            bail!(conduit::i18n::text(
                "cli.only_native_windows_exe_files_and_global"
            ))
        }
    };
    if !path.is_file() {
        bail!(conduit::i18n::format(
            "cli.file_does_not_exist_or_is_inaccessible",
            &[("arg0", format!("{}", path.display()))]
        ));
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
            path: validate_executable(&path).with_context(|| {
                conduit::i18n::format("cli.invalid_path", &[("arg0", source.label().to_string())])
            })?,
            source,
        });
    }
    for (path, source) in candidates {
        if let Ok(path) = validate_executable(&path) {
            return Ok(ResolvedCli { path, source });
        }
    }
    bail!(conduit::i18n::text(
        "cli.cli_not_found_select_an_installed_exe"
    ))
}

pub fn resolve(provider: Provider, settings: &CliSettings) -> Result<ResolvedCli> {
    let descriptor = provider.descriptor();
    let (key, filename) = (descriptor.path_env, descriptor.executable);
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
    if let Some(binary) = descriptor.npm_binary
        && let Some(paths) = std::env::var_os("PATH")
    {
        candidates.extend(
            std::env::split_paths(&paths).map(|p| (p.join(binary), PathSource::SearchPath)),
        );
    }
    let common = std::env::var_os(descriptor.install_env)
        .map(|p| PathBuf::from(p).join(descriptor.install_path));
    if let Some(path) = common {
        candidates.push((path, PathSource::CommonLocation));
    }
    resolve_candidates(explicit, candidates)
}

pub fn tested_version(provider: Provider) -> &'static str {
    provider.descriptor().tested_version
}

/// Diagnostics remain available for disabled engines; every model launch uses this gate.
pub fn resolve_for_launch(provider: Provider, settings: &CliSettings) -> Result<ResolvedCli> {
    anyhow::ensure!(
        settings.enabled(provider),
        conduit::i18n::format(
            "cli.is_disabled_enable_it_in_ai_engine",
            &[("arg0", provider.label().to_string())]
        )
    );
    resolve(provider, settings)
}

#[derive(Debug)]
pub struct VersionInfo {
    pub version: String,
    pub verified_version: bool,
    pub update: Option<Result<updates::UpdateInfo, String>>,
}

impl VersionInfo {
    pub fn diagnostic(&self) -> String {
        let compatibility = if self.verified_version {
            conduit::i18n::text_static("cli.matches_the_version_verified_by_this_project")
        } else {
            conduit::i18n::text_static("cli.not_yet_fully_regression_tested")
        };
        match &self.update {
            None => conduit::i18n::format(
                "cli.version",
                &[
                    ("arg0", self.version.to_string()),
                    ("compatibility", compatibility.to_string()),
                ],
            ),
            Some(Ok(update)) => conduit::i18n::format(
                "cli.installed_version",
                &[
                    ("arg0", self.version.to_string()),
                    ("arg1", update.summary().to_string()),
                    ("compatibility", compatibility.to_string()),
                ],
            ),
            Some(Err(error)) => conduit::i18n::format(
                "cli.installed_version_update_check_failed",
                &[
                    ("arg0", self.version.to_string()),
                    ("error", error.to_string()),
                    ("compatibility", compatibility.to_string()),
                ],
            ),
        }
    }
}
fn parse_version(provider: Provider, output: &str) -> Result<VersionInfo> {
    let signature = provider.descriptor().version_signature;
    let line = output
        .lines()
        .find(|line| {
            if signature.is_empty() {
                let value = line.trim();
                let base = value.split(['-', '+']).next().unwrap_or("");
                base.split('.').count() == 3
                    && base
                        .split('.')
                        .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
            } else {
                line.contains(signature)
            }
        })
        .ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::format(
                "cli.no_version_identifier_found_check_the_selected",
                &[("arg0", provider.label().to_string())]
            ))
        })?;
    let version = line
        .split_whitespace()
        .find(|part| {
            let base = part.split(['-', '+']).next().unwrap_or("");
            base.split('.').count() == 3
                && base
                    .split('.')
                    .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        })
        .ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::format(
                "cli.unsupported_cli_version_format",
                &[("arg0", line.to_string())]
            ))
        })?
        .to_string();
    Ok(VersionInfo {
        verified_version: version == tested_version(provider),
        update: None,
        version,
    })
}

pub struct VersionProbe {
    pub path: PathBuf,
    pub checking_updates: bool,
    pub installing_update: bool,
    pub progress: Option<Receiver<String>>,
    pub events: Receiver<Result<VersionInfo, String>>,
    cancelled: Arc<AtomicBool>,
}
impl VersionProbe {
    pub fn start(provider: Provider, path: PathBuf) -> Self {
        Self::spawn(provider, path, false)
    }
    pub fn start_update_check(provider: Provider, path: PathBuf) -> Self {
        Self::spawn(provider, path, true)
    }
    pub fn start_install(provider: Provider, path: PathBuf) -> Self {
        let (sender, events) = mpsc::channel();
        let (progress_sender, progress) = mpsc::sync_channel(8);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel = cancelled.clone();
        let executable = path.clone();
        thread::spawn(move || {
            let result = updates::install(provider, &executable, &cancel, |text| {
                let _ = progress_sender.try_send(text);
            })
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        Self {
            path,
            checking_updates: false,
            installing_update: true,
            progress: Some(progress),
            events,
            cancelled,
        }
    }
    fn spawn(provider: Provider, path: PathBuf, checking_updates: bool) -> Self {
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
            .and_then(|mut info| {
                if checking_updates {
                    let update = updates::check(provider, &info.version, &cancel)
                        .map_err(|error| format!("{error:#}"));
                    anyhow::ensure!(
                        !cancel.load(Ordering::Acquire),
                        conduit::i18n::text("settings.cli.detection_cancelled")
                    );
                    info.update = Some(update);
                }
                Ok(info)
            })
            .map_err(|e| format!("{e:#}"));
            let _ = sender.send(result);
        });
        Self {
            path,
            checking_updates,
            installing_update: false,
            progress: None,
            events,
            cancelled,
        }
    }
    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn test_pending(
        path: PathBuf,
        checking_updates: bool,
    ) -> (Self, mpsc::Sender<Result<VersionInfo, String>>) {
        let (sender, events) = mpsc::channel();
        (
            Self {
                path,
                checking_updates,
                installing_update: false,
                progress: None,
                events,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
            sender,
        )
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
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

fn read_available(file: &mut File, output: &mut Vec<u8>, limit: usize) -> Result<()> {
    if let Ok(count) = available(file)
        && count > 0
    {
        let mut buffer = [0; 4096];
        let count = file.read(&mut buffer[..(count as usize).min(4096)])?;
        if output.len() + count > limit {
            bail!(conduit::i18n::format(
                "cli.cli_output_exceeded_kib_detection_stopped",
                &[("arg0", format!("{}", limit / 1024))]
            ));
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
    capture_command(
        executable,
        args,
        &std::env::temp_dir(),
        &Default::default(),
        cancel,
        timeout,
        64 * 1024,
    )
}

/// Bounded read-only diagnostics, with the same Job cleanup as CLI turns.
pub(crate) fn capture_command(
    executable: &Path,
    args: &[String],
    cwd: &Path,
    environment: &std::collections::BTreeMap<String, String>,
    cancel: &AtomicBool,
    timeout: Duration,
    limit: usize,
) -> Result<String> {
    let (program, full_args) = command_for(executable, args);
    let (child, mut pipes) = if environment.is_empty() {
        Child::spawn(&program, &full_args, cwd)?
    } else {
        Child::spawn_with_env(&program, &full_args, cwd, environment)?
    };
    drop(pipes.stdin);
    let result = (|| {
        let started = Instant::now();
        let mut stdout = vec![];
        let mut stderr = vec![];
        loop {
            if cancel.load(Ordering::Acquire) {
                bail!(conduit::i18n::text("settings.cli.detection_cancelled"));
            }
            if started.elapsed() > timeout {
                bail!(conduit::i18n::text(
                    "cli.cli_query_timed_out_process_tree_stopped"
                ));
            }
            read_available(&mut pipes.stdout, &mut stdout, limit)?;
            read_available(&mut pipes.stderr, &mut stderr, limit)?;
            if let Some(code) = child.exit_code()? {
                // Drain all output after exit, including a version line without a newline.
                while available(&pipes.stdout).unwrap_or(0) > 0 {
                    read_available(&mut pipes.stdout, &mut stdout, limit)?;
                }
                while available(&pipes.stderr).unwrap_or(0) > 0 {
                    read_available(&mut pipes.stderr, &mut stderr, limit)?;
                }
                if code != 0 {
                    bail!(conduit::i18n::format(
                        "cli.cli_query_exit_code",
                        &[
                            ("code", format!("{}", code)),
                            ("arg0", format!("{}", String::from_utf8_lossy(&stderr)))
                        ]
                    ));
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
    fn legacy_paths_migrate_without_losing_enabled_state_or_model_preferences() {
        let _locale = crate::test_support::chinese();
        let mut settings: CliSettings = serde_json::from_value(serde_json::json!({
            "claude_path":"C:/old/claude.exe", "codex_path":"C:/old/codex.exe"
        }))
        .unwrap();
        assert!(Provider::ALL.into_iter().all(|p| settings.enabled(p)));
        assert_eq!(
            settings.path(Provider::Codex),
            Some(Path::new("C:/old/codex.exe"))
        );
        settings
            .models_mut(Provider::Codex)
            .hidden
            .insert("hidden".into());
        assert_eq!(
            settings.path(Provider::Codex),
            Some(Path::new("C:/old/codex.exe"))
        );
        settings.set_enabled(Provider::Codex, false);
        assert!(settings.codex_path.is_none());
        assert!(
            resolve_for_launch(Provider::Codex, &settings)
                .unwrap_err()
                .to_string()
                .contains("已停用")
        );
        settings.set(Provider::Codex, None);
        assert_eq!(settings.path(Provider::Codex), None);
        assert!(!settings.enabled(Provider::Codex));
        let loaded: CliSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(loaded, settings);
        assert!(
            loaded.engines[&Provider::Codex]
                .models
                .hidden
                .contains("hidden")
        );
        assert_eq!(
            loaded.path(Provider::Claude),
            Some(Path::new("C:/old/claude.exe"))
        );
    }
    #[test]
    fn unknown_versions_are_visible_without_claiming_compatibility() {
        let _locale = crate::test_support::chinese();
        assert!(
            parse_version(Provider::OpenCode, "1.19.2\r\n")
                .unwrap()
                .verified_version
        );
        assert!(
            !parse_version(Provider::OpenCode, "1.19.3-alpha.1")
                .unwrap()
                .verified_version
        );
        assert!(parse_version(Provider::OpenCode, "PowerShell 7.5.0").is_err());
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
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
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
        let _locale = crate::test_support::chinese();
        for provider in Provider::ALL {
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
